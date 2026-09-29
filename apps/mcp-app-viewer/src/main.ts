// Host page: renders an MCP App view through the sandbox proxy and plays the
// host's side of the protocol with user-provided (mock) tool data.
import demoHtml from "./demo-app.html?raw";
import { AppBridge, RpcError, type Direction } from "./bridge";
import {
  defaultSandboxOrigin,
  isJsonRpc,
  parseResource,
  permissionsToAllow,
  PROTOCOL_VERSION,
  RESOURCE_MIME_TYPE,
  type DisplayMode,
  type JsonRpcMessage,
  type Tool,
  type UiResource,
} from "./protocol";

const HOST_INFO = { name: "mcp-app-viewer", version: "0.1.0" };
const HOST_DISPLAY_MODES: DisplayMode[] = ["inline", "fullscreen"];
const MAX_HEIGHT = 800;
const MAX_LOG_ENTRIES = 500;

// Theme-aware values (see "Theming" in the spec); the host page uses the same palette.
const STYLE_VARIABLES = {
  "--color-background-primary": "light-dark(#ffffff, #171717)",
  "--color-background-secondary": "light-dark(#f5f5f5, #262626)",
  "--color-text-primary": "light-dark(#171717, #fafafa)",
  "--color-text-secondary": "light-dark(#525252, #a3a3a3)",
  "--color-border-primary": "light-dark(#d4d4d4, #404040)",
  "--font-sans": "system-ui, -apple-system, 'Segoe UI', sans-serif",
  "--font-mono": "ui-monospace, SFMono-Regular, Menlo, monospace",
  "--border-radius-md": "8px",
};

const DEMO_TOOL: Tool = {
  name: "roll_dice",
  description: "Roll dice and show them in an interactive view",
  inputSchema: {
    type: "object",
    properties: { sides: { type: "integer" }, count: { type: "integer" } },
  },
  _meta: { ui: { resourceUri: "ui://demo/dice", visibility: ["model", "app"] } },
};
const demoResult = (rolls: number[]) => ({
  content: [{ type: "text", text: `Rolled ${rolls.join(", ")}` }],
  structuredContent: { rolls },
});

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const ui = {
  file: $<HTMLInputElement>("file"),
  resource: $<HTMLTextAreaElement>("resource"),
  tool: $<HTMLTextAreaElement>("tool"),
  toolInput: $<HTMLTextAreaElement>("tool-input"),
  toolResult: $<HTMLTextAreaElement>("tool-result"),
  autoResult: $<HTMLInputElement>("auto-result"),
  callResponse: $<HTMLTextAreaElement>("call-response"),
  theme: $<HTMLSelectElement>("theme"),
  error: $<HTMLParagraphElement>("error"),
  render: $<HTMLButtonElement>("render"),
  sendResult: $<HTMLButtonElement>("send-result"),
  cancel: $<HTMLButtonElement>("cancel"),
  teardown: $<HTMLButtonElement>("teardown"),
  uri: $<HTMLElement>("uri"),
  status: $<HTMLSpanElement>("status"),
  preview: $<HTMLDivElement>("preview"),
  exitFullscreen: $<HTMLButtonElement>("exit-fullscreen"),
  inbox: $<HTMLOListElement>("inbox"),
  log: $<HTMLOListElement>("log"),
};

const sandboxOrigin =
  new URLSearchParams(location.search).get("sandbox") ?? defaultSandboxOrigin(location);

interface Session {
  bridge: AppBridge;
  frame: HTMLIFrameElement;
  resource: UiResource;
  tool?: Tool;
  appModes?: DisplayMode[];
  displayMode: DisplayMode;
}
let session: Session | null = null;

function parseJson(text: string, label: string): any {
  if (!text.trim()) return undefined;
  try {
    return JSON.parse(text);
  } catch (e) {
    throw new Error(`${label}: ${(e as Error).message}`, { cause: e });
  }
}

const pretty = (value: unknown) => JSON.stringify(value, null, 2);

function showError(message: string | null): void {
  ui.error.hidden = message === null;
  ui.error.textContent = message ?? "";
}

function setStatus(text: string): void {
  ui.status.textContent = text;
  const ready = session?.bridge.isInitialized ?? false;
  ui.sendResult.disabled = !ready;
  ui.cancel.disabled = !ready;
  ui.teardown.disabled = session === null;
}

function append(list: HTMLOListElement, title: string, body: string, className = ""): void {
  const item = document.createElement("li");
  item.className = className;
  const details = document.createElement("details");
  const summary = document.createElement("summary");
  summary.append(
    Object.assign(document.createElement("time"), { textContent: new Date().toLocaleTimeString() }),
    ` ${title}`,
  );
  details.append(summary, Object.assign(document.createElement("pre"), { textContent: body }));
  item.append(details);
  list.prepend(item);
  list.querySelector(`li:nth-child(${MAX_LOG_ENTRIES + 1})`)?.remove();
}

function logMessage(direction: Direction, msg: JsonRpcMessage): void {
  const kind = msg.method ?? (msg.error ? `error #${msg.id}` : `result #${msg.id}`);
  const id = msg.method && msg.id !== undefined ? ` #${msg.id}` : "";
  append(
    ui.log,
    `${direction} ${kind}${id}`,
    pretty(msg),
    direction === "host→view" ? "out" : "in",
  );
}

const inbox = (title: string, body: string) => append(ui.inbox, title, body);

function containerDimensions(mode: DisplayMode) {
  return mode === "fullscreen"
    ? { width: window.innerWidth, height: window.innerHeight }
    : { width: ui.preview.clientWidth, maxHeight: MAX_HEIGHT };
}

function hostContext(s: Session) {
  return {
    toolInfo: s.tool ? { tool: s.tool } : undefined,
    theme: ui.theme.value,
    styles: { variables: STYLE_VARIABLES },
    displayMode: s.displayMode,
    availableDisplayModes: HOST_DISPLAY_MODES,
    containerDimensions: containerDimensions(s.displayMode),
    locale: navigator.language,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    userAgent: `${HOST_INFO.name}/${HOST_INFO.version}`,
    platform: "web",
    deviceCapabilities: {
      touch: matchMedia("(pointer: coarse)").matches,
      hover: matchMedia("(hover: hover)").matches,
    },
  };
}

function notifyContext(params: object): void {
  if (session?.bridge.isInitialized)
    session.bridge.notify("ui/notifications/host-context-changed", params);
}

function setDisplayMode(s: Session, mode: DisplayMode): void {
  s.displayMode = mode;
  ui.preview.classList.toggle("fullscreen", mode === "fullscreen");
  ui.exitFullscreen.hidden = mode !== "fullscreen";
  notifyContext({ displayMode: mode, containerDimensions: containerDimensions(mode) });
}

function sendResult(): void {
  try {
    const result = parseJson(ui.toolResult.value, "Tool result") ?? { content: [] };
    session?.bridge.notify("ui/notifications/tool-result", result);
  } catch (e) {
    showError((e as Error).message);
  }
}

function handlers(s: Session) {
  return {
    ping: () => ({}),
    "ui/open-link": ({ url }: { url: string }) => {
      let parsed: URL;
      try {
        parsed = new URL(url);
      } catch {
        throw new RpcError(-32000, "Invalid URL");
      }
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:")
        throw new RpcError(-32000, "Invalid URL");
      inbox("ui/open-link", parsed.href);
      if (!confirm(`The view wants to open:\n${parsed.href}`))
        throw new RpcError(-32000, "Link opening denied by user");
      window.open(parsed.href, "_blank", "noopener,noreferrer");
      return {};
    },
    "ui/message": (params: { role: string; content: { type: string; text?: string } }) => {
      const text = params?.content?.type === "text" ? params.content.text : undefined;
      if (typeof text !== "string") throw new RpcError(-32000, "Invalid message format");
      inbox(`ui/message (${params.role})`, text);
      return {};
    },
    "ui/update-model-context": (params: unknown) => {
      inbox("ui/update-model-context", pretty(params));
      return {};
    },
    "ui/request-display-mode": ({ mode }: { mode: DisplayMode }) => {
      const allowed =
        HOST_DISPLAY_MODES.includes(mode) && (!s.appModes || s.appModes.includes(mode));
      if (allowed && mode !== s.displayMode) setDisplayMode(s, mode);
      return { mode: s.displayMode };
    },
    "tools/call": ({ name }: { name: string }) => {
      const visibility = s.tool?._meta?.ui?.visibility ?? ["model", "app"];
      if (s.tool?.name === name && !visibility.includes("app")) {
        throw new RpcError(-32000, `Tool "${name}" is not callable by apps`);
      }
      return parseJson(ui.callResponse.value, "tools/call response") ?? { content: [] };
    },
    "resources/read": ({ uri }: { uri: string }) => {
      if (uri !== s.resource.uri) throw new RpcError(-32002, `Resource not found: ${uri}`);
      const { html, meta } = s.resource;
      return { contents: [{ uri, mimeType: RESOURCE_MIME_TYPE, text: html, _meta: { ui: meta } }] };
    },
  };
}

function start(resource: UiResource, tool: Tool | undefined, args: object): void {
  const frame = document.createElement("iframe");
  frame.title = resource.uri;
  frame.sandbox.value = "allow-scripts allow-same-origin allow-forms";
  frame.allow = permissionsToAllow(resource.meta.permissions);
  frame.src = new URL(
    `sandbox.html?host=${encodeURIComponent(location.origin)}`,
    sandboxOrigin + location.pathname,
  ).href;
  ui.preview.classList.toggle("bordered", resource.meta.prefersBorder !== false);

  const s: Session = { frame, resource, tool, displayMode: "inline" } as Session;
  s.bridge = new AppBridge({
    resource,
    post: (msg) => frame.contentWindow?.postMessage(msg, sandboxOrigin!),
    initialize: (params) => {
      s.appModes = params?.appCapabilities?.availableDisplayModes;
      const { csp, permissions } = resource.meta;
      return {
        protocolVersion: PROTOCOL_VERSION,
        hostInfo: HOST_INFO,
        hostCapabilities: {
          openLinks: {},
          serverTools: {},
          serverResources: {},
          logging: {},
          sandbox: { csp, permissions },
        },
        hostContext: hostContext(s),
      };
    },
    handlers: handlers(s),
    onInitialized: () => {
      setStatus("initialized");
      s.bridge.notify("ui/notifications/tool-input", { arguments: args });
      if (ui.autoResult.checked) sendResult();
    },
    onNotification: (method, params) => {
      if (method === "ui/notifications/size-changed" && typeof params?.height === "number") {
        frame.style.height = `${Math.min(params.height, MAX_HEIGHT)}px`;
      } else if (method === "notifications/message") {
        const data = typeof params?.data === "string" ? params.data : pretty(params?.data);
        inbox(`log (${params?.level ?? "info"})`, data);
      }
    },
    log: logMessage,
  });

  session = s;
  ui.preview.replaceChildren(frame);
  ui.uri.textContent = resource.uri;
  setStatus("loading");
}

async function teardown(reason: string): Promise<void> {
  const s = session;
  if (!s) return;
  if (s.bridge.isInitialized) {
    try {
      await s.bridge.request("ui/resource-teardown", { reason }, 3000);
    } catch (e) {
      inbox("teardown", (e as Error).message);
    }
  }
  s.frame.remove();
  session = null;
  ui.preview.classList.remove("fullscreen");
  ui.exitFullscreen.hidden = true;
  ui.preview.replaceChildren(
    Object.assign(document.createElement("p"), { className: "empty", textContent: "Torn down." }),
  );
  setStatus("idle");
}

async function render(): Promise<void> {
  showError(null);
  if (!sandboxOrigin || sandboxOrigin === location.origin) {
    showError(
      "The sandbox must be on a different origin. Open the viewer via localhost or 127.0.0.1, or pass ?sandbox=<origin>.",
    );
    return;
  }
  let resource: UiResource, tool: Tool | undefined, args: object;
  try {
    tool = parseJson(ui.tool.value, "Tool");
    args = parseJson(ui.toolInput.value, "Tool input") ?? {};
    resource = parseResource(ui.resource.value, tool?._meta?.ui?.resourceUri);
  } catch (e) {
    showError((e as Error).message);
    return;
  }
  const expected = tool?._meta?.ui?.resourceUri;
  if (expected && expected !== resource.uri)
    inbox("warning", `Tool points at ${expected}, resource is ${resource.uri}`);
  await teardown("re-render");
  start(resource, tool, args);
}

function loadDemo(): void {
  ui.resource.value = demoHtml;
  ui.tool.value = pretty(DEMO_TOOL);
  ui.toolInput.value = pretty({ sides: 6, count: 3 });
  ui.toolResult.value = pretty(demoResult([2, 5, 3]));
  ui.callResponse.value = pretty(demoResult([6, 6, 1]));
}

function applyTheme(): void {
  document.documentElement.style.colorScheme = ui.theme.value;
  notifyContext({ theme: ui.theme.value, styles: { variables: STYLE_VARIABLES } });
}

window.addEventListener("message", (event) => {
  const s = session;
  if (
    !s ||
    event.source !== s.frame.contentWindow ||
    event.origin !== sandboxOrigin ||
    !isJsonRpc(event.data)
  )
    return;
  void s.bridge.receive(event.data);
});

let lastWidth = 0;
new ResizeObserver(() => {
  const width = ui.preview.clientWidth;
  if (session?.displayMode !== "inline" || width === lastWidth) return;
  lastWidth = width;
  notifyContext({ containerDimensions: containerDimensions("inline") });
}).observe(ui.preview);

ui.file.addEventListener("change", async () => {
  const file = ui.file.files?.[0];
  if (file) ui.resource.value = await file.text();
});
ui.render.addEventListener("click", () => void render());
ui.sendResult.addEventListener("click", sendResult);
ui.cancel.addEventListener("click", () =>
  session?.bridge.notify("ui/notifications/tool-cancelled", {
    reason: "Cancelled from the viewer",
  }),
);
ui.teardown.addEventListener("click", () => void teardown("user"));
ui.exitFullscreen.addEventListener("click", () => session && setDisplayMode(session, "inline"));
ui.theme.addEventListener("change", applyTheme);
$("load-demo").addEventListener("click", loadDemo);
$("clear-log").addEventListener("click", () => ui.log.replaceChildren());

ui.theme.value = matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
applyTheme();
loadDemo();
void render();
