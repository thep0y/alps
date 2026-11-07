const EXPLICIT_SCHEME_RE = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//;
const CONTROL_RE = /[\u0000-\u001f\u007f]/;
const MAX_URL_BYTES = 8192;

export type TargetErrorCode =
  | "empty"
  | "control"
  | "invalid"
  | "unsupported_scheme"
  | "missing_host"
  | "credentials"
  | "fragment"
  | "too_long";

export class TargetError extends Error {
  readonly code: TargetErrorCode;

  constructor(code: TargetErrorCode, message: string) {
    super(message);
    this.name = "TargetError";
    this.code = code;
  }
}

/**
 * Normalize a user-supplied file URL and reject values that can never be
 * accepted by ALPS. The gateway remains the authority: this only improves the
 * feedback before a request is made.
 */
export function normalizeTarget(raw: string): string {
  const value = raw.trim();
  if (!value) {
    throw new TargetError("empty", "Enter a file URL to continue.");
  }
  if (CONTROL_RE.test(value)) {
    throw new TargetError("control", "The URL contains invalid control characters.");
  }

  // Match ALPS: a scheme is recognized only when it is followed by "://".
  // This keeps "example.com:8080/file" a host:port URL rather than a scheme.
  let candidate = value;
  if (candidate.startsWith("//")) {
    candidate = `https:${candidate}`;
  } else if (!EXPLICIT_SCHEME_RE.test(candidate)) {
    candidate = `https://${candidate}`;
  }

  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    throw new TargetError("invalid", "Enter a valid absolute URL.");
  }

  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    throw new TargetError("unsupported_scheme", "Only http and https URLs are supported.");
  }
  if (!parsed.hostname) {
    throw new TargetError("missing_host", "The URL must include a host name.");
  }
  if (parsed.username || parsed.password) {
    throw new TargetError("credentials", "URLs with embedded credentials are not supported.");
  }
  if (parsed.hash) {
    throw new TargetError("fragment", "Remove the #fragment from the URL.");
  }

  const normalized = parsed.toString();
  if (new TextEncoder().encode(normalized).byteLength > MAX_URL_BYTES) {
    throw new TargetError("too_long", "The URL is too long.");
  }
  return normalized;
}
