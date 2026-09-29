// Types and pure helpers for MCP Apps (SEP-1865, spec version 2026-01-26).
// https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx

export const PROTOCOL_VERSION = "2026-01-26";
export const RESOURCE_MIME_TYPE = "text/html;profile=mcp-app";
const SANDBOX_PREFIX = "ui/notifications/sandbox-";

export type JsonRpcId = string | number;

export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

export interface JsonRpcMessage {
  jsonrpc: "2.0";
  id?: JsonRpcId;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: JsonRpcError;
}

export interface ResourceCsp {
  connectDomains?: string[];
  resourceDomains?: string[];
  frameDomains?: string[];
  baseUriDomains?: string[];
}

export interface ResourcePermissions {
  camera?: object;
  microphone?: object;
  geolocation?: object;
  clipboardWrite?: object;
}

interface UiResourceMeta {
  csp?: ResourceCsp;
  permissions?: ResourcePermissions;
  domain?: string;
  prefersBorder?: boolean;
}

export interface UiResource {
  uri: string;
  html: string;
  meta: UiResourceMeta;
}

export type DisplayMode = "inline" | "fullscreen" | "pip";

export interface Tool {
  name: string;
  description?: string;
  inputSchema?: object;
  _meta?: { ui?: { resourceUri?: string; visibility?: Array<"model" | "app"> } };
}

export function isJsonRpc(value: unknown): value is JsonRpcMessage {
  return typeof value === "object" && value !== null && (value as JsonRpcMessage).jsonrpc === "2.0";
}

export function isSandboxMessage(msg: JsonRpcMessage): boolean {
  return msg.method?.startsWith(SANDBOX_PREFIX) ?? false;
}

/**
 * Parses what a user pastes as the view: either a raw HTML document, or the
 * JSON of a `resources/read` result (optionally wrapped in its JSON-RPC response).
 */
export function parseResource(input: string, fallbackUri = "ui://viewer/inline"): UiResource {
  const text = input.trim();
  if (!text.startsWith("{")) {
    return { uri: fallbackUri, html: input, meta: {} };
  }
  const json = JSON.parse(text);
  const result = json.result ?? json;
  const content = Array.isArray(result.contents) ? result.contents[0] : result;
  if (typeof content?.uri !== "string" || !content.uri.startsWith("ui://")) {
    throw new Error("Resource URI must use the ui:// scheme");
  }
  if (content.mimeType !== RESOURCE_MIME_TYPE) {
    throw new Error(`Resource mimeType must be "${RESOURCE_MIME_TYPE}"`);
  }
  let html: string;
  if (typeof content.text === "string") {
    html = content.text;
  } else if (typeof content.blob === "string") {
    html = new TextDecoder().decode(Uint8Array.from(atob(content.blob), (c) => c.charCodeAt(0)));
  } else {
    throw new Error("Resource must have either text or blob");
  }
  return { uri: content.uri, html, meta: content._meta?.ui ?? {} };
}

// Drop anything that could break out of a CSP source list (`;`, quotes, spaces).
function sources(domains: string[] | undefined): string[] {
  return (domains ?? []).filter((d) => /^[^\s;,'"]+$/.test(d));
}

const list = (...items: string[]) => items.join(" ");

/** Builds the view's CSP: the spec's restrictive default plus declared domains only. */
export function buildCsp(csp: ResourceCsp = {}): string {
  const res = sources(csp.resourceDomains);
  const connect = sources(csp.connectDomains);
  const frame = sources(csp.frameDomains);
  const base = sources(csp.baseUriDomains);
  return [
    "default-src 'none'",
    list("script-src 'self' 'unsafe-inline'", ...res),
    list("style-src 'self' 'unsafe-inline'", ...res),
    list("img-src 'self' data:", ...res),
    list("font-src 'self' data:", ...res),
    list("media-src 'self' data:", ...res),
    connect.length ? list("connect-src", ...connect) : "connect-src 'none'",
    frame.length ? list("frame-src", ...frame) : "frame-src 'none'",
    "object-src 'none'",
    base.length ? list("base-uri", ...base) : "base-uri 'self'",
  ].join("; ");
}

/**
 * Puts a CSP `<meta>` before any markup of the document (after the doctype),
 * so no script in the view runs before the policy applies.
 */
export function injectCsp(html: string, policy: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${policy.replaceAll('"', "&quot;")}">`;
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  const at = doctype ? doctype[0].length : 0;
  return html.slice(0, at) + meta + html.slice(at);
}

const PERMISSION_FEATURES: Record<keyof ResourcePermissions, string> = {
  camera: "camera",
  microphone: "microphone",
  geolocation: "geolocation",
  clipboardWrite: "clipboard-write",
};

/** Maps declared permissions to an iframe `allow` attribute value. */
export function permissionsToAllow(permissions: ResourcePermissions = {}): string {
  return (Object.keys(PERMISSION_FEATURES) as Array<keyof ResourcePermissions>)
    .filter((key) => permissions[key])
    .map((key) => PERMISSION_FEATURES[key])
    .join("; ");
}

/**
 * Web hosts must load the sandbox proxy from a different origin. For local use
 * we swap `localhost` and `127.0.0.1`, which the same dev server answers to.
 */
export function defaultSandboxOrigin(location: {
  protocol: string;
  hostname: string;
  port: string;
}): string | null {
  const swapped = { localhost: "127.0.0.1", "127.0.0.1": "localhost" }[location.hostname];
  if (!swapped) return null;
  return `${location.protocol}//${swapped}${location.port ? `:${location.port}` : ""}`;
}
