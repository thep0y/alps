export interface ProbeResult {
  filename?: string;
  size?: number;
  contentType?: string;
}

export class ProxyRequestError extends Error {
  readonly code?: string;
  readonly status: number;

  constructor(message: string, status: number, code?: string) {
    super(message);
    this.name = "ProxyRequestError";
    this.status = status;
    this.code = code;
  }
}

const MESSAGES: Record<string, string> = {
  destination_denied: "This destination is not on the allowlist.",
  ip_denied: "The destination resolves to a blocked network address.",
  invalid_url: "The URL is not valid.",
  invalid_request: "The request is not valid.",
  url_too_long: "The URL is too long.",
  rate_limited: "Too many requests. Please wait and try again.",
  capacity_exceeded: "The server is busy. Please try again shortly.",
  unauthorized: "The download service is not configured correctly.",
  upstream_redirect_error: "The file server returned an invalid redirect.",
  upstream_connect_error: "The file server could not be reached.",
  upstream_dns_error: "The file server name could not be resolved.",
  upstream_response_too_large: "The file is larger than the allowed limit.",
  upstream_protocol_error: "The file server returned an invalid response.",
  upstream_timeout: "The file server took too long to respond.",
  upstream_unavailable: "The download service is temporarily unavailable.",
  internal_error: "The download service is temporarily unavailable.",
  method_not_allowed: "This request is not supported.",
  request_body_not_allowed: "This request is not supported.",
};

/** Public download URL. This is deliberately the only path the browser calls. */
export function proxyUrl(target: string): string {
  return `/?url=${encodeURIComponent(target)}`;
}

/** Ask the gateway for metadata without starting the file transfer. */
export async function probeTarget(target: string, signal?: AbortSignal): Promise<ProbeResult> {
  const url = proxyUrl(target);
  let response = await request(url, "HEAD", signal);

  // Some origins reject HEAD. Validate with a GET that is cancelled as soon as
  // the response headers arrive; the body is never buffered.
  if (response.status === 405 || response.status === 501) {
    response = await requestAndCancel(url, signal);
  }

  if (!response.ok) {
    throw await toError(response);
  }

  return {
    filename:
      filenameFromDisposition(response.headers.get("content-disposition")) ??
      filenameFromUrl(target),
    size: parseSize(response.headers.get("content-length")),
    contentType: cleanContentType(response.headers.get("content-type")),
  };
}

function request(url: string, method: string, signal?: AbortSignal): Promise<Response> {
  return fetch(url, {
    method,
    signal,
    headers: { Accept: "application/json" },
    cache: "no-store",
    credentials: "same-origin",
    redirect: "manual",
  });
}

async function requestAndCancel(url: string, signal?: AbortSignal): Promise<Response> {
  const controller = new AbortController();
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  try {
    const response = await request(url, "GET", controller.signal);
    await response.body?.cancel().catch(() => undefined);
    return response;
  } finally {
    signal?.removeEventListener("abort", abort);
    controller.abort();
  }
}

async function toError(response: Response): Promise<ProxyRequestError> {
  const headerCode = response.headers.get("x-error-code") ?? undefined;
  let code = headerCode;
  let message: string | undefined;

  try {
    const text = await response.text();
    if (text) {
      const body = JSON.parse(text) as { error?: { code?: string; message?: string } };
      code ??= body.error?.code;
      message = body.error?.message;
    }
  } catch {
    // Non-JSON upstream errors are intentionally not surfaced verbatim.
  }

  const friendly = code ? MESSAGES[code] : undefined;
  return new ProxyRequestError(
    friendly ?? message ?? statusMessage(response.status),
    response.status,
    code,
  );
}

function statusMessage(status: number): string {
  switch (status) {
    case 400:
      return "The request is not valid.";
    case 401:
      return "The download service is not available.";
    case 403:
      return "This destination is not allowed.";
    case 404:
      return "The file could not be found.";
    case 405:
      return "This request is not supported.";
    case 408:
      return "The request timed out.";
    case 413:
      return "The request is too large.";
    case 414:
      return "The URL is too long.";
    case 429:
      return "Too many requests. Please wait and try again.";
    case 502:
      return "The file server could not be reached.";
    case 503:
      return "The server is busy. Please try again shortly.";
    case 504:
      return "The file server took too long to respond.";
    case 500:
      return "The download service is temporarily unavailable.";
    default:
      return "The file could not be checked. Please try again.";
  }
}

function parseSize(value: string | null): number | undefined {
  if (!value) return undefined;
  const size = Number.parseInt(value, 10);
  return Number.isFinite(size) && size >= 0 ? size : undefined;
}

function cleanContentType(value: string | null): string | undefined {
  if (!value) return undefined;
  return value.split(";", 1)[0]?.trim() || undefined;
}

function filenameFromUrl(target: string): string | undefined {
  try {
    const url = new URL(target);
    const segment = url.pathname.split("/").filter(Boolean).at(-1);
    return segment ? decodeURIComponent(segment) : undefined;
  } catch {
    return undefined;
  }
}

function filenameFromDisposition(value: string | null): string | undefined {
  if (!value) return undefined;

  const extended = /filename\*\s*=\s*UTF-8''([^;]+)/i.exec(value);
  if (extended?.[1]) {
    try {
      return decodeURIComponent(extended[1].trim());
    } catch {
      return undefined;
    }
  }

  const quoted = /filename\s*=\s*"([^"]*)"/i.exec(value);
  if (quoted?.[1]) return quoted[1];

  const plain = /filename\s*=\s*([^;]+)/i.exec(value);
  return plain?.[1]?.trim() || undefined;
}
