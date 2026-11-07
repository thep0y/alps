import { For, Match, Show, Switch, createMemo, createSignal } from "solid-js";
import { ProxyRequestError, probeTarget, proxyUrl } from "./lib/proxy";
import { TargetError, normalizeTarget } from "./lib/target";

type UiState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      target: string;
      filename: string;
      size?: number;
      contentType?: string;
    };

const FEATURES = [
  {
    icon: "shield",
    title: "Allowlist enforced",
    body: "The destination is checked before a single byte is fetched. Internal addresses stay blocked.",
  },
  {
    icon: "stream",
    title: "Streamed delivery",
    body: "Files are piped straight through with backpressure. Nothing is buffered in memory.",
  },
  {
    icon: "refresh",
    title: "Range friendly",
    body: "Resumable downloads and conditional requests are passed through to the origin.",
  },
] as const;

type FeatureIconName = (typeof FEATURES)[number]["icon"];

export default function App() {
  const [input, setInput] = createSignal("");
  const [state, setState] = createSignal<UiState>({ kind: "idle" });

  const isBusy = () => state().kind === "checking";
  const failure = createMemo(() => {
    const value = state();
    return value.kind === "error" ? value : null;
  });
  const ready = createMemo(() => {
    const value = state();
    return value.kind === "ready" ? value : null;
  });

  function handleInput(event: InputEvent & { currentTarget: HTMLInputElement }) {
    setInput(event.currentTarget.value);
    if (state().kind === "error" || state().kind === "ready") {
      setState({ kind: "idle" });
    }
  }

  async function handleSubmit(event: SubmitEvent) {
    event.preventDefault();
    if (isBusy()) return;

    setState({ kind: "checking" });

    let target: string;
    try {
      target = normalizeTarget(input());
    } catch (error) {
      setState({
        kind: "error",
        message:
          error instanceof TargetError
            ? error.message
            : "Enter a valid file URL.",
      });
      return;
    }

    setInput(target);

    try {
      const metadata = await probeTarget(target);
      setState({
        kind: "ready",
        target,
        filename: metadata.filename ?? filenameFromTarget(target),
        size: metadata.size,
        contentType: metadata.contentType,
      });
    } catch (error) {
      setState({
        kind: "error",
        message:
          error instanceof ProxyRequestError
            ? error.message
            : error instanceof DOMException && error.name === "AbortError"
              ? "The request was cancelled."
              : "The file could not be checked. Please try again.",
      });
    }
  }

  return (
    <div class="page-background relative min-h-dvh overflow-hidden">
      <div class="dot-grid pointer-events-none absolute inset-x-0 top-0 h-[30rem]" aria-hidden="true" />

      <div class="relative mx-auto flex min-h-dvh w-full max-w-4xl flex-col px-5 sm:px-8">
        <header class="flex items-center justify-between gap-4 py-7">
          <div class="flex items-center gap-3">
            <div class="flex h-10 w-10 items-center justify-center rounded-2xl bg-slate-950 text-indigo-200 shadow-lg shadow-slate-900/15">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-5 w-5" aria-hidden="true">
                <path d="M12 4v11m0 0 4-4m-4 4-4-4" stroke-linecap="round" stroke-linejoin="round" />
                <path d="M5 16v2a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-2" stroke-linecap="round" />
              </svg>
            </div>
            <div>
              <p class="text-sm font-semibold tracking-tight text-slate-950">File delivery</p>
              <p class="text-xs text-slate-500">Allowlisted downloads</p>
            </div>
          </div>

          <span class="inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full border border-emerald-200 bg-emerald-50 px-3 py-1 text-xs font-medium text-emerald-700">
            <span class="h-1.5 w-1.5 rounded-full bg-emerald-500" aria-hidden="true" />
            Allowlist enforced
          </span>
        </header>

        <main class="flex-1 py-10 sm:py-16">
          <div class="max-w-2xl">
            <p class="text-xs font-semibold uppercase tracking-[0.22em] text-indigo-600">Secure file delivery</p>
            <h1 class="mt-4 text-4xl font-semibold tracking-[-0.03em] text-slate-950 sm:text-5xl">
              Download files from an approved source.
            </h1>
            <p class="mt-5 max-w-xl text-base leading-7 text-slate-600">
              Paste a direct file URL. The proxy checks the destination against its allowlist and streams the file to
              your browser, without exposing internal service details.
            </p>
          </div>

          <form onSubmit={handleSubmit} class="mt-10 rounded-3xl border border-slate-200 bg-white/90 p-2 shadow-2xl shadow-slate-900/5 backdrop-blur">
            <div class="flex flex-col gap-2 sm:flex-row">
              <div class="relative flex-1">
                <span class="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-slate-400" aria-hidden="true">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-5 w-5">
                    <path d="M10 13a5 5 0 0 0 7.07 0l1.42-1.42a5 5 0 0 0-7.07-7.07L10.7 5.23" stroke-linecap="round" />
                    <path d="M14 11a5 5 0 0 0-7.07 0L5.5 12.43a5 5 0 0 0 7.07 7.07l.7-.7" stroke-linecap="round" />
                  </svg>
                </span>
                <label class="sr-only" for="file-url">File URL</label>
                <input
                  id="file-url"
                  type="text"
                  inputmode="url"
                  autocomplete="off"
                  autocapitalize="off"
                  spellcheck={false}
                  enterkeyhint="go"
                  placeholder="https://files.example.org/report.pdf"
                  value={input()}
                  onInput={handleInput}
                  disabled={isBusy()}
                  class="h-14 w-full rounded-2xl bg-slate-50 pl-11 pr-4 text-[15px] text-slate-900 outline-none ring-1 ring-inset ring-slate-200 transition placeholder:text-slate-400 focus:bg-white focus:ring-2 focus:ring-indigo-500 disabled:cursor-not-allowed disabled:opacity-70"
                />
              </div>

              <button
                type="submit"
                disabled={isBusy()}
                class="inline-flex h-14 items-center justify-center gap-2 rounded-2xl bg-slate-950 px-6 text-sm font-semibold text-white shadow-lg shadow-slate-900/10 transition hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-not-allowed disabled:opacity-60"
              >
                <Show
                  when={isBusy()}
                  fallback={
                    <>
                      Check file
                      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-4 w-4" aria-hidden="true">
                        <path d="M5 12h14m0 0-5-5m5 5-5 5" stroke-linecap="round" stroke-linejoin="round" />
                      </svg>
                    </>
                  }
                >
                  <span class="h-4 w-4 animate-spin rounded-full border-2 border-white/30 border-t-white" aria-hidden="true" />
                  Checking…
                </Show>
              </button>
            </div>
          </form>

          <p class="mt-3 text-xs leading-5 text-slate-500">
            https:// is added when the scheme is omitted. Only allowlisted destinations can be fetched.
          </p>

          <div class="mt-5" aria-live="polite">
            <Show when={state().kind === "checking"}>
              <div class="flex items-center gap-3 rounded-2xl border border-slate-200 bg-white/80 p-4 text-sm text-slate-600">
                <span class="h-4 w-4 animate-spin rounded-full border-2 border-slate-300 border-t-slate-900" aria-hidden="true" />
                <p>Checking the destination and reading file metadata…</p>
              </div>
            </Show>

            <Show when={failure()} keyed>
              {(value) => (
                <div role="alert" class="flex items-start gap-3 rounded-2xl border border-rose-200 bg-rose-50 p-4 text-sm text-rose-800">
                  <span class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-rose-100 text-rose-600" aria-hidden="true">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-4 w-4">
                      <path d="M12 8v5m0 3.5v.5" stroke-linecap="round" />
                      <path d="M10.3 4.9 3.6 17a2 2 0 0 0 1.7 3h13.4a2 2 0 0 0 1.7-3L13.7 4.9a2 2 0 0 0-3.4 0Z" stroke-linejoin="round" />
                    </svg>
                  </span>
                  <div>
                    <p class="font-semibold">Could not check the file</p>
                    <p class="mt-1 leading-5 text-rose-700">{value.message}</p>
                  </div>
                </div>
              )}
            </Show>

            <Show when={ready()} keyed>
              {(value) => (
                <div class="rounded-3xl border border-emerald-200 bg-white p-5 shadow-xl shadow-emerald-900/5">
                  <div class="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
                    <div class="flex min-w-0 items-start gap-3">
                      <span class="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-emerald-50 text-emerald-600 ring-1 ring-inset ring-emerald-200" aria-hidden="true">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-5 w-5">
                          <path d="m5 12 4 4L19 6" stroke-linecap="round" stroke-linejoin="round" />
                        </svg>
                      </span>
                      <div class="min-w-0">
                        <p class="text-xs font-semibold uppercase tracking-[0.18em] text-emerald-600">Ready to download</p>
                        <p class="mt-1 truncate text-base font-semibold text-slate-950" title={value.filename}>
                          {value.filename}
                        </p>
                        <div class="mt-2 flex flex-wrap items-center gap-2 text-xs text-slate-500">
                          <Show when={formatBytes(value.size)}>
                            {(size) => <span class="rounded-full bg-slate-100 px-2.5 py-1">{size()}</span>}
                          </Show>
                          <Show when={value.contentType}>
                            {(type) => <span class="rounded-full bg-slate-100 px-2.5 py-1">{type()}</span>}
                          </Show>
                          <span class="rounded-full bg-slate-100 px-2.5 py-1">Allowlist checked</span>
                        </div>
                      </div>
                    </div>

                    <a
                      href={proxyUrl(value.target)}
                      download={value.filename}
                      class="inline-flex h-11 shrink-0 items-center justify-center gap-2 rounded-xl bg-slate-950 px-5 text-sm font-semibold text-white shadow-lg shadow-slate-900/10 transition hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600"
                    >
                      Download file
                      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-4 w-4" aria-hidden="true">
                        <path d="M12 4v11m0 0 4-4m-4 4-4-4" stroke-linecap="round" stroke-linejoin="round" />
                        <path d="M5 19h14" stroke-linecap="round" />
                      </svg>
                    </a>
                  </div>
                </div>
              )}
            </Show>
          </div>

          <section class="mt-12 grid gap-3 sm:grid-cols-3" aria-label="How it works">
            <For each={FEATURES}>
              {(feature) => (
                <div class="rounded-2xl border border-slate-200/80 bg-white/60 p-4">
                  <span class="flex h-9 w-9 items-center justify-center rounded-xl bg-indigo-50 text-indigo-600 ring-1 ring-inset ring-indigo-100" aria-hidden="true">
                    <FeatureIcon name={feature.icon} />
                  </span>
                  <h2 class="mt-3 text-sm font-semibold text-slate-950">{feature.title}</h2>
                  <p class="mt-1 text-xs leading-5 text-slate-500">{feature.body}</p>
                </div>
              )}
            </For>
          </section>
        </main>

        <footer class="flex flex-col gap-2 border-t border-slate-200/80 py-6 text-xs text-slate-500 sm:flex-row sm:items-center sm:justify-between">
          <p>Files are streamed through the server. Nothing is stored.</p>
          <p>Only approved destinations are reachable.</p>
        </footer>
      </div>
    </div>
  );
}

function FeatureIcon(props: { name: FeatureIconName }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-4 w-4">
      <Switch>
        <Match when={props.name === "shield"}>
          <path d="M12 3 5 6v5c0 4.4 2.9 7.6 7 9 4.1-1.4 7-4.6 7-9V6l-7-3Z" stroke-linejoin="round" />
          <path d="m9 12 2 2 4-4" stroke-linecap="round" stroke-linejoin="round" />
        </Match>
        <Match when={props.name === "stream"}>
          <path d="M4 7h10a3 3 0 0 1 0 6H6a3 3 0 0 0 0 6h14" stroke-linecap="round" />
          <path d="m17 16 3 3-3 3" stroke-linecap="round" stroke-linejoin="round" />
        </Match>
        <Match when={props.name === "refresh"}>
          <path d="M20 7v5h-5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M4 17v-5h5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M6.1 9a7 7 0 0 1 11.6-2.6L20 8.7M4 15.3l2.3 2.3A7 7 0 0 0 17.9 15" stroke-linecap="round" />
        </Match>
      </Switch>
    </svg>
  );
}

function filenameFromTarget(target: string): string {
  try {
    const url = new URL(target);
    const segment = url.pathname.split("/").filter(Boolean).at(-1);
    return segment ? decodeURIComponent(segment) : "Download";
  } catch {
    return "Download";
  }
}

function formatBytes(bytes: number | undefined): string | undefined {
  if (bytes === undefined) return undefined;
  if (bytes < 1024) return `${bytes} B`;

  const units = ["KB", "MB", "GB", "TB"] as const;
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${units[unit] ?? "B"}`;
}
