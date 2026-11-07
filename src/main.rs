use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use alps::app::{AppHandle, AppState};
use alps::config::Config;
use alps::http::routes;
use alps::server;
use alps::telemetry;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::info;

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("argument error: {message}");
            print_usage();
            return ExitCode::from(2);
        }
    };
    if args.help {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let loaded = match Config::load_with(&args.config, args.static_dir.clone()) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("configuration error: {error}");
            return ExitCode::from(1);
        }
    };
    if args.check_config {
        println!("{}", loaded.config.summary());
        return ExitCode::SUCCESS;
    }

    if let Err(error) = telemetry::init(&loaded.config) {
        eprintln!("failed to initialize logging: {error}");
        return ExitCode::from(1);
    }

    let state = match AppState::build(&loaded) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("startup error: {error}");
            return ExitCode::from(1);
        }
    };

    let address = loaded.config.listen_addr();
    let metrics_enabled = loaded.config.metrics.enabled;
    let metrics_address = loaded.config.metrics.listen_addr();

    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("failed to bind {address}: {error}");
            return ExitCode::from(2);
        }
    };
    let admin_listener = if metrics_enabled {
        match TcpListener::bind(metrics_address).await {
            Ok(listener) => Some(listener),
            Err(error) => {
                eprintln!("failed to bind metrics listener {metrics_address}: {error}");
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };

    let web_dir = loaded.config.web.dir.clone();
    let web_enabled = loaded.config.web.enabled();
    let web_expose_api = loaded.config.web.expose_api;

    info!(
        listen = %address,
        allowlist_rules = state.policy.allowlist_len(),
        redirect_allowlist_rules = state.policy.redirect_allowlist_len(),
        signing = state.signer.enabled(),
        web_static = web_enabled,
        web_api = web_expose_api,
        "alps listening"
    );
    if metrics_enabled {
        info!(listen = %metrics_address, "metrics listener started");
    }
    if web_enabled {
        tracing::warn!(
            "public web UI enabled: GET/HEAD /?url= is unauthenticated and bounded only by the allowlist"
        );
        if web_expose_api {
            tracing::warn!("web.expose_api = true: JSON API routes remain on the public listener");
        }
    }

    let handle = AppHandle::new(state);
    let reload_web_dir = web_dir.clone();
    let app = if let Some(static_dir) = web_dir {
        routes::web_router(Arc::clone(&handle), static_dir)
    } else {
        routes::router(Arc::clone(&handle))
    };
    let config = Arc::new(loaded.config);
    spawn_reload_task(
        args.config.clone(),
        args.static_dir.clone(),
        reload_web_dir,
        Arc::clone(&handle),
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut servers = JoinSet::new();
    servers.spawn(server::serve(
        listener,
        app,
        Arc::clone(&config),
        wait_for_shutdown(shutdown_rx.clone()),
    ));
    if let Some(admin_listener) = admin_listener {
        servers.spawn(server::serve(
            admin_listener,
            routes::metrics_router(Arc::clone(&handle)),
            Arc::clone(&config),
            wait_for_shutdown(shutdown_rx.clone()),
        ));
    }

    let mut exit = ExitCode::SUCCESS;
    tokio::select! {
        _ = shutdown_signal() => {
            info!("shutdown signal received");
        }
        joined = servers.join_next() => {
            match joined {
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(error))) => {
                    eprintln!("server error: {error}");
                    exit = ExitCode::from(2);
                }
                Some(Err(error)) => {
                    eprintln!("server task error: {error}");
                    exit = ExitCode::from(2);
                }
                None => {}
            }
        }
    }

    let _ = shutdown_tx.send(true);
    while let Some(joined) = servers.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("server error: {error}");
                exit = ExitCode::from(2);
            }
            Err(error) => {
                eprintln!("server task error: {error}");
                exit = ExitCode::from(2);
            }
        }
    }
    exit
}

struct Args {
    config: PathBuf,
    static_dir: Option<PathBuf>,
    check_config: bool,
    help: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut config = std::env::var_os("ALPS_CONFIG")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut static_dir = None;
    let mut check_config = false;
    let mut help = false;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let value = args.next().ok_or("--config requires a path")?;
                config = Some(PathBuf::from(value));
            }
            "--static-dir" => {
                let value = args.next().ok_or("--static-dir requires a path")?;
                static_dir = Some(PathBuf::from(value));
            }
            "--check-config" => check_config = true,
            "--help" | "-h" => help = true,
            other if other.starts_with("--config=") => {
                config = Some(PathBuf::from(&other["--config=".len()..]));
            }
            other if other.starts_with("--static-dir=") => {
                static_dir = Some(PathBuf::from(&other["--static-dir=".len()..]));
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    Ok(Args {
        config: config.unwrap_or_else(default_config_path),
        static_dir,
        check_config,
        help,
    })
}

/// Default search order when neither --config nor ALPS_CONFIG is set:
/// $HOME/.coinfig/alps/alps.toml, $HOME/.alps.toml, then the binary directory.
fn default_config_path() -> PathBuf {
    default_config_path_from(home_dir().as_deref(), &binary_dir())
}

fn default_config_path_from(home: Option<&Path>, binary_dir: &Path) -> PathBuf {
    if let Some(home) = home {
        for relative in [".coinfig/alps/alps.toml", ".alps.toml"] {
            let candidate = home.join(relative);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    binary_dir.join("alps.toml")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn binary_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn print_usage() {
    println!(
        "alps {}\n\nUSAGE:\n    alps [OPTIONS]\n\nOPTIONS:\n    --config <PATH>     Configuration file (default: ALPS_CONFIG,\n                        $HOME/.coinfig/alps/alps.toml, $HOME/.alps.toml,\n                        or <exe dir>/alps.toml)\n    --static-dir <PATH> Serve the built web UI from this directory\n                        (overrides web.dir and ALPS_STATIC_DIR)\n    --check-config      Validate configuration and exit\n    -h, --help          Print this help",
        env!("CARGO_PKG_VERSION")
    );
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

#[cfg(unix)]
fn spawn_reload_task(
    path: PathBuf,
    static_dir: Option<PathBuf>,
    web_dir: Option<PathBuf>,
    handle: Arc<AppHandle>,
) {
    let mut signal = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(%error, "failed to install SIGHUP handler");
            return;
        }
    };
    tokio::spawn(async move {
        while signal.recv().await.is_some() {
            match alps::app::reload_from_path_with(&path, static_dir.as_deref(), web_dir.as_deref())
            {
                Ok(state) => {
                    handle.replace(state);
                    tracing::info!("configuration reloaded");
                }
                Err(error) => {
                    tracing::error!(%error, "reload failed; keeping previous configuration");
                }
            }
        }
    });
}

#[cfg(not(unix))]
fn spawn_reload_task(
    _path: PathBuf,
    _static_dir: Option<PathBuf>,
    _web_dir: Option<PathBuf>,
    _handle: Arc<AppHandle>,
) {
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::debug!(%error, "ctrl_c handler failed");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::debug!(%error, "SIGTERM handler failed");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("alps-config-paths-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn coinfig_path_wins_when_present() {
        let root = scratch("coinfig");
        let home = root.join("home");
        let binary = root.join("bin");
        std::fs::create_dir_all(&binary).unwrap();
        let coinfig = home.join(".coinfig/alps/alps.toml");
        write_config(&coinfig);
        write_config(&home.join(".alps.toml"));
        write_config(&binary.join("alps.toml"));

        assert_eq!(default_config_path_from(Some(&home), &binary), coinfig);
    }

    #[test]
    fn home_dotfile_is_used_when_coinfig_is_absent() {
        let root = scratch("dotfile");
        let home = root.join("home");
        let binary = root.join("bin");
        std::fs::create_dir_all(&binary).unwrap();
        let dotfile = home.join(".alps.toml");
        write_config(&dotfile);
        write_config(&binary.join("alps.toml"));

        assert_eq!(default_config_path_from(Some(&home), &binary), dotfile);
    }

    #[test]
    fn binary_directory_is_the_fallback() {
        let root = scratch("binary");
        let home = root.join("home");
        let binary = root.join("bin");
        std::fs::create_dir_all(&binary).unwrap();
        let fallback = binary.join("alps.toml");
        write_config(&fallback);

        assert_eq!(default_config_path_from(Some(&home), &binary), fallback);
        assert_eq!(default_config_path_from(None, &binary), fallback);
    }

    #[test]
    fn missing_config_returns_the_binary_path_for_the_error() {
        let root = scratch("missing");
        let home = root.join("home");
        let binary = root.join("bin");
        std::fs::create_dir_all(&binary).unwrap();

        assert_eq!(
            default_config_path_from(Some(&home), &binary),
            binary.join("alps.toml")
        );
    }
}
