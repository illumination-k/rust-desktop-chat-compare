// Sandbox proxy: runs on a different origin than the host, loads the view's
// HTML into an inner iframe under its CSP, and relays JSON-RPC both ways.
import {
  buildCsp,
  injectCsp,
  isJsonRpc,
  isSandboxMessage,
  permissionsToAllow,
  type JsonRpcMessage,
  type ResourceCsp,
  type ResourcePermissions,
} from "./protocol";

interface ResourceReady {
  html: string;
  sandbox?: string;
  csp?: ResourceCsp;
  permissions?: ResourcePermissions;
}

const hostOrigin = new URLSearchParams(location.search).get("host");
let inner: HTMLIFrameElement | null = null;

function load({ html, sandbox, csp, permissions }: ResourceReady): void {
  inner?.remove();
  inner = document.createElement("iframe");
  inner.sandbox.value = sandbox ?? "allow-scripts allow-same-origin allow-forms";
  inner.allow = permissionsToAllow(permissions);
  inner.srcdoc = injectCsp(html, buildCsp(csp));
  document.body.append(inner);
}

function toHost(msg: JsonRpcMessage): void {
  if (hostOrigin) window.parent.postMessage(msg, hostOrigin);
}

window.addEventListener("message", (event) => {
  const msg: unknown = event.data;
  if (!isJsonRpc(msg)) return;
  if (event.source === window.parent && event.origin === hostOrigin) {
    if (msg.method === "ui/notifications/sandbox-resource-ready") load(msg.params as ResourceReady);
    // The inner view may have an opaque origin, so "*" is the only usable target.
    else if (!isSandboxMessage(msg)) inner?.contentWindow?.postMessage(msg, "*");
  } else if (inner && event.source === inner.contentWindow && !isSandboxMessage(msg)) {
    toHost(msg);
  }
});

toHost({ jsonrpc: "2.0", method: "ui/notifications/sandbox-proxy-ready", params: {} });
