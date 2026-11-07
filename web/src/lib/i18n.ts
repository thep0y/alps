import type { TargetErrorCode } from "./target";

export type Locale = "zh" | "en";

export type FeatureIconName = "shield" | "stream" | "refresh";

interface FeatureText {
  title: string;
  body: string;
}

export interface Messages {
  pageTitle: string;
  metaDescription: string;
  brand: string;
  brandSubtitle: string;
  badge: string;
  eyebrow: string;
  heading: string;
  intro: string;
  urlLabel: string;
  urlPlaceholder: string;
  checkFile: string;
  checkingButton: string;
  hint: string;
  checkingMessage: string;
  errorTitle: string;
  readyTitle: string;
  allowlistChecked: string;
  downloadFile: string;
  howItWorks: string;
  footerStreamed: string;
  footerApproved: string;
  downloadFallback: string;
  features: Record<FeatureIconName, FeatureText>;
  targetErrors: Record<TargetErrorCode, string>;
  proxyErrors: Record<string, string>;
  statusErrors: Record<number, string>;
  genericError: string;
  cancelled: string;
}

const en: Messages = {
  pageTitle: "ALPS",
  metaDescription: "Download files from an allowlisted source through the ALPS secure streaming proxy.",
  brand: "ALPS",
  brandSubtitle: "Allowlisted file delivery",
  badge: "Allowlist enforced",
  eyebrow: "Secure file delivery",
  heading: "Download files from an approved source.",
  intro:
    "Paste a direct file URL. The proxy checks the destination against its allowlist and streams the file to your browser, without exposing internal service details.",
  urlLabel: "File URL",
  urlPlaceholder: "https://files.example.org/report.pdf",
  checkFile: "Check file",
  checkingButton: "Checking…",
  hint: "https:// is added when the scheme is omitted. Only allowlisted destinations can be fetched.",
  checkingMessage: "Checking the destination and reading file metadata…",
  errorTitle: "Could not check the file",
  readyTitle: "Ready to download",
  allowlistChecked: "Allowlist checked",
  downloadFile: "Download file",
  howItWorks: "How it works",
  footerStreamed: "Files are streamed through the server. Nothing is stored.",
  footerApproved: "Only approved destinations are reachable.",
  downloadFallback: "Download",
  features: {
    shield: {
      title: "Allowlist enforced",
      body: "The destination is checked before a single byte is fetched. Internal addresses stay blocked.",
    },
    stream: {
      title: "Streamed delivery",
      body: "Files are piped straight through with backpressure. Nothing is buffered in memory.",
    },
    refresh: {
      title: "Range friendly",
      body: "Resumable downloads and conditional requests are passed through to the origin.",
    },
  },
  targetErrors: {
    empty: "Enter a file URL to continue.",
    control: "The URL contains invalid control characters.",
    invalid: "Enter a valid absolute URL.",
    unsupported_scheme: "Only http and https URLs are supported.",
    missing_host: "The URL must include a host name.",
    credentials: "URLs with embedded credentials are not supported.",
    fragment: "Remove the #fragment from the URL.",
    too_long: "The URL is too long.",
  },
  proxyErrors: {
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
  },
  statusErrors: {
    400: "The request is not valid.",
    401: "The download service is not available.",
    403: "This destination is not allowed.",
    404: "The file could not be found.",
    405: "This request is not supported.",
    408: "The request timed out.",
    413: "The request is too large.",
    414: "The URL is too long.",
    429: "Too many requests. Please wait and try again.",
    500: "The download service is temporarily unavailable.",
    502: "The file server could not be reached.",
    503: "The server is busy. Please try again shortly.",
    504: "The file server took too long to respond.",
  },
  genericError: "The file could not be checked. Please try again.",
  cancelled: "The request was cancelled.",
};

const zh: Messages = {
  pageTitle: "ALPS",
  metaDescription: "通过 ALPS 安全流式代理，从允许列表中的来源下载文件。",
  brand: "ALPS",
  brandSubtitle: "允许列表文件传输",
  badge: "已启用允许列表",
  eyebrow: "安全文件传输",
  heading: "从已批准的来源下载文件。",
  intro: "粘贴文件直链。代理会按允许列表校验目标，并把文件以流式传输到浏览器，不暴露内部服务信息。",
  urlLabel: "文件 URL",
  urlPlaceholder: "https://files.example.org/report.pdf",
  checkFile: "检查文件",
  checkingButton: "检查中…",
  hint: "省略协议时会自动补全 https://。只有允许列表中的目标可以访问。",
  checkingMessage: "正在检查目标并读取文件信息…",
  errorTitle: "无法检查文件",
  readyTitle: "可以下载",
  allowlistChecked: "已通过允许列表检查",
  downloadFile: "下载文件",
  howItWorks: "工作原理",
  footerStreamed: "文件通过服务器流式传输，不会被存储。",
  footerApproved: "只有已批准的目标可以访问。",
  downloadFallback: "下载",
  features: {
    shield: {
      title: "强制允许列表",
      body: "在获取任何字节之前先校验目标，内部地址始终不可访问。",
    },
    stream: {
      title: "流式传输",
      body: "文件带背压直接转发，不会整体缓存在内存中。",
    },
    refresh: {
      title: "支持 Range",
      body: "断点续传和条件请求会原样透传到源站。",
    },
  },
  targetErrors: {
    empty: "请输入文件 URL。",
    control: "URL 中包含无效的控制字符。",
    invalid: "请输入有效的绝对 URL。",
    unsupported_scheme: "仅支持 http 和 https URL。",
    missing_host: "URL 必须包含主机名。",
    credentials: "不支持包含用户名或密码的 URL。",
    fragment: "请移除 URL 中的 #fragment。",
    too_long: "URL 太长。",
  },
  proxyErrors: {
    destination_denied: "该目标不在允许列表中。",
    ip_denied: "该目标解析到被禁止的网络地址。",
    invalid_url: "URL 无效。",
    invalid_request: "请求无效。",
    url_too_long: "URL 太长。",
    rate_limited: "请求过于频繁，请稍后重试。",
    capacity_exceeded: "服务器繁忙，请稍后重试。",
    unauthorized: "下载服务未正确配置。",
    upstream_redirect_error: "文件服务器返回了无效的重定向。",
    upstream_connect_error: "无法连接文件服务器。",
    upstream_dns_error: "无法解析文件服务器域名。",
    upstream_response_too_large: "文件超过允许的大小限制。",
    upstream_protocol_error: "文件服务器返回了无效响应。",
    upstream_timeout: "文件服务器响应超时。",
    upstream_unavailable: "下载服务暂时不可用。",
    internal_error: "下载服务暂时不可用。",
    method_not_allowed: "不支持该请求。",
    request_body_not_allowed: "不支持该请求。",
  },
  statusErrors: {
    400: "请求无效。",
    401: "下载服务不可用。",
    403: "该目标不被允许。",
    404: "找不到该文件。",
    405: "不支持该请求。",
    408: "请求超时。",
    413: "请求内容过大。",
    414: "URL 太长。",
    429: "请求过于频繁，请稍后重试。",
    500: "下载服务暂时不可用。",
    502: "无法连接文件服务器。",
    503: "服务器繁忙，请稍后重试。",
    504: "文件服务器响应超时。",
  },
  genericError: "无法检查文件，请重试。",
  cancelled: "请求已取消。",
};

const MESSAGES: Record<Locale, Messages> = { en, zh };

/** Pick the first browser language supported by the UI, falling back to English. */
export function detectLocale(languages: readonly string[] | undefined): Locale {
  for (const language of languages ?? []) {
    const tag = language.trim().toLowerCase();
    if (tag === "zh" || tag.startsWith("zh-")) return "zh";
    if (tag === "en" || tag.startsWith("en-")) return "en";
  }
  return "en";
}

export function messagesFor(locale: Locale): Messages {
  return MESSAGES[locale];
}

function browserLocale(): Locale {
  if (typeof navigator === "undefined") return "en";
  const languages = navigator.languages?.length ? navigator.languages : [navigator.language];
  return detectLocale(languages);
}

export const locale = browserLocale();
export const t = messagesFor(locale);

/** Keep the document title, description and language in sync with the UI. */
export function applyDocumentLocale(value: Locale): void {
  if (typeof document === "undefined") return;
  document.documentElement.lang = value === "zh" ? "zh-CN" : "en";
  document.title = messagesFor(value).pageTitle;
  document
    .querySelector('meta[name="description"]')
    ?.setAttribute("content", messagesFor(value).metaDescription);
}
