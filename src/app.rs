//! Shared application state.

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::auth::{Auth, AuthError};
use crate::config::{Config, ConfigError, LoadedConfig};
use crate::limits::Limits;
use crate::policy::{IpPolicy, UrlPolicy};
use crate::signing::Signer;
use crate::upstream::client;
use crate::upstream::redirects::RedirectController;
use crate::upstream::resolver::{GaiBackend, SecureResolver};

#[derive(Debug, thiserror::Error)]
pub enum ReloadError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    App(#[from] AppError),
    #[error("web.dir is a startup setting; restart ALPS to change it")]
    WebDirChanged,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication configuration is invalid: {0}")]
    Auth(#[from] AuthError),
    #[error("failed to build the upstream client: {0}")]
    Client(#[from] reqwest::Error),
}

pub struct AppState {
    pub config: Arc<Config>,
    pub policy: Arc<UrlPolicy>,
    pub resolver: Arc<SecureResolver>,
    pub limits: Arc<Limits>,
    pub auth: Arc<Auth>,
    pub signer: Arc<Signer>,
    pub redirects: RedirectController,
}

impl AppState {
    pub fn build(loaded: &LoadedConfig) -> Result<Arc<Self>, AppError> {
        let resolver = Arc::new(SecureResolver::new(
            Arc::new(GaiBackend),
            IpPolicy::strict(),
            loaded.config.proxy.connect_timeout(),
        ));
        Self::build_with_resolver(loaded, resolver)
    }

    /// Build with an injected resolver. Used by tests to point a fake host at
    /// a loopback upstream without weakening the production IP policy.
    pub fn build_with_resolver(
        loaded: &LoadedConfig,
        resolver: Arc<SecureResolver>,
    ) -> Result<Arc<Self>, AppError> {
        let config = Arc::new(loaded.config.clone());
        let policy = Arc::new(UrlPolicy::from_config(&config));
        let limits = Arc::new(Limits::new(&config.limits));
        let auth = Arc::new(Auth::new(&config, loaded.api_token.clone())?);
        let signer = Arc::new(Signer::new(&config, &loaded.signing_secrets));
        let client = client::build(&config, resolver.clone())?;
        let redirects = RedirectController::new(
            client,
            policy.clone(),
            resolver.clone(),
            limits.clone(),
            &config,
        );
        Ok(Arc::new(Self {
            config,
            policy,
            resolver,
            limits,
            auth,
            signer,
            redirects,
        }))
    }
}

/// Hot-swappable state holder. Every request loads the current snapshot once,
/// so a reload never changes the configuration under a request in flight.
pub struct AppHandle {
    current: ArcSwap<AppState>,
}

impl AppHandle {
    pub fn new(state: Arc<AppState>) -> Arc<Self> {
        Arc::new(Self {
            current: ArcSwap::from(state),
        })
    }

    pub fn current(&self) -> Arc<AppState> {
        self.current.load_full()
    }

    pub fn replace(&self, state: Arc<AppState>) {
        self.current.store(state);
    }
}

/// Load a new configuration and build a replacement state.
///
/// Listener addresses and inbound server limits require a restart; the policy,
/// allowlist, auth token, signing keys, limits and upstream client are swapped.
pub fn reload_from_path(path: &Path) -> Result<Arc<AppState>, ReloadError> {
    reload_from_path_with(path, None, None)
}

/// Reload while preserving a CLI `--static-dir` override and enforcing that
/// the startup web directory cannot change under a live static service.
pub fn reload_from_path_with(
    path: &Path,
    static_dir: Option<&Path>,
    expected_web_dir: Option<&Path>,
) -> Result<Arc<AppState>, ReloadError> {
    let loaded = Config::load_with(path, static_dir.map(Path::to_path_buf))?;
    if loaded.config.web.dir.as_deref() != expected_web_dir {
        return Err(ReloadError::WebDirChanged);
    }
    Ok(AppState::build(&loaded)?)
}
