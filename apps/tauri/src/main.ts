import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isSendKey } from "./keys";

type Role = "user" | "assistant";
type ProviderKind = "anthropic" | "mock";

interface Snapshot {
  conversations: { id: string; title: string }[];
  activeId: string | null;
  /**
   * `content` is plain text for user messages and sanitized HTML for assistant ones.
   * `app` is the key of the MCP App view shown below the text.
   */
  messages: { role: Role; content: string; error: string | null; app: string | null }[];
  streaming: boolean;
}

interface Settings {
  provider: ProviderKind;
  model: string;
  system_prompt: string;
  max_tokens: number;
}

/** Answer of `app_message`: JSON-RPC messages to post to the view, and host events. */
interface AppReply {
  messages: unknown[];
  events: ({ type: "resize"; value: number } | { type: "send-message"; value: string })[];
}

interface StreamPayload {
  conversationId: string;
  html: string;
  done: boolean;
}

function $<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} not found`);
  return found as T;
}

const ui = {
  conversations: $<HTMLUListElement>("conversations"),
  messages: $<HTMLDivElement>("messages"),
  empty: $<HTMLDivElement>("empty"),
  input: $<HTMLTextAreaElement>("input"),
  send: $<HTMLButtonElement>("send"),
  stop: $<HTMLButtonElement>("stop"),
  error: $<HTMLDivElement>("error"),
  errorText: $<HTMLSpanElement>("error-text"),
  chatView: $<HTMLElement>("chat-view"),
  settingsView: $<HTMLFormElement>("settings-view"),
  provider: $<HTMLSelectElement>("provider"),
  apiKey: $<HTMLInputElement>("api-key"),
  removeKey: $<HTMLButtonElement>("remove-key"),
  model: $<HTMLInputElement>("model"),
  maxTokens: $<HTMLInputElement>("max-tokens"),
  systemPrompt: $<HTMLTextAreaElement>("system-prompt"),
};

let current: Snapshot | null = null;
/** What each rendered message node shows, so unchanged nodes (and their iframes) are kept. */
let renderedKeys: string[] = [];
/** MCP App iframes by message key. */
const frames = new Map<string, HTMLIFrameElement>();

function showError(message: string | null): void {
  ui.error.hidden = message === null;
  ui.errorText.textContent = message ?? "";
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T | undefined> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    showError(String(e));
    return undefined;
  }
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function renderMessage(message: Snapshot["messages"][number]): HTMLElement {
  const bubble = el("div", `message ${message.role}`);
  bubble.append(el("div", "role", message.role === "user" ? "You" : "Assistant"));
  if (message.role === "user") {
    bubble.append(el("div", "plain", message.content));
  } else {
    const body = el("div", "markdown");
    body.innerHTML = message.content; // sanitized by chat-core's markdown::to_html
    bubble.append(body);
  }
  if (message.app) bubble.append(renderApp(message.app));
  if (message.error) bubble.append(el("div", "message-error", `⚠ ${message.error}`));
  return bubble;
}

/** A sandboxed iframe (opaque origin) for an MCP App view; the host logic lives in Rust. */
function renderApp(key: string): HTMLIFrameElement {
  const frame = el("iframe", "mcp-app");
  frame.sandbox.value = "allow-scripts allow-forms";
  frames.set(key, frame);
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  void call<{ url: string; prefersBorder: boolean }>("app_open", { key, dark }).then((info) => {
    if (!info) return;
    frame.classList.toggle("bordered", info.prefersBorder);
    frame.src = info.url;
  });
  return frame;
}

async function closeApps(node: Element): Promise<void> {
  for (const [key, frame] of frames) {
    if (!node.contains(frame)) continue;
    frames.delete(key);
    // Best effort: the iframe is removed right after.
    const teardown = await call<unknown>("app_close", { key });
    if (teardown) frame.contentWindow?.postMessage(teardown, "*");
  }
}

window.addEventListener("message", async (event) => {
  const entry = [...frames].find(([, frame]) => frame.contentWindow === event.source);
  if (!entry || typeof event.data !== "object" || event.data === null) return;
  const [key, frame] = entry;
  const reply = await call<AppReply>("app_message", { key, message: event.data });
  for (const message of reply?.messages ?? []) frame.contentWindow?.postMessage(message, "*");
  for (const e of reply?.events ?? []) {
    if (e.type === "resize") {
      frame.style.height = `${e.value}px`;
    } else {
      ui.input.value = e.value;
      void send();
    }
  }
});

/** Replaces only the message nodes that changed, so MCP App iframes are not reloaded. */
function renderMessages(messages: Snapshot["messages"]): void {
  const nodes = ui.messages.children;
  messages.forEach((message, i) => {
    const key = JSON.stringify(message);
    const node = nodes.item(i);
    if (node && renderedKeys[i] === key) return;
    const fresh = renderMessage(message);
    if (node) {
      void closeApps(node);
      node.replaceWith(fresh);
    } else {
      ui.messages.append(fresh);
    }
    renderedKeys[i] = key;
  });
  while (nodes.length > messages.length) {
    const last = nodes.item(nodes.length - 1);
    if (!last) break;
    void closeApps(last);
    last.remove();
  }
  renderedKeys.length = messages.length;
}

function render(snapshot: Snapshot | undefined): void {
  if (!snapshot) return;
  current = snapshot;
  ui.conversations.replaceChildren(
    ...snapshot.conversations.map(({ id, title }) => {
      const item = el("li", id === snapshot.activeId ? "active" : "");
      const label = el("span", "title", title);
      label.addEventListener(
        "click",
        () => void call<Snapshot>("select", { id }).then(render).then(showChat),
      );
      const remove = el("button", "link", "🗑");
      remove.title = "Delete";
      remove.addEventListener("click", () => void call<Snapshot>("delete", { id }).then(render));
      item.append(label, remove);
      return item;
    }),
  );
  ui.empty.hidden = snapshot.activeId !== null;
  renderMessages(snapshot.messages);
  ui.send.hidden = snapshot.streaming;
  ui.stop.hidden = !snapshot.streaming;
}

async function send(): Promise<void> {
  const text = ui.input.value;
  if (!text.trim() || current?.streaming) return;
  const snapshot = await call<Snapshot>("send", { text });
  if (snapshot) {
    ui.input.value = "";
    showError(null);
  }
  render(snapshot);
}

function showChat(): void {
  ui.settingsView.hidden = true;
  ui.chatView.hidden = false;
}

async function openSettings(): Promise<void> {
  const view = await call<{ settings: Settings; apiKeySet: boolean }>("get_settings");
  if (!view) return;
  ui.provider.value = view.settings.provider;
  ui.apiKey.value = "";
  ui.apiKey.placeholder = view.apiKeySet ? "•••• (saved in keychain)" : "sk-ant-…";
  ui.removeKey.hidden = !view.apiKeySet;
  ui.model.value = view.settings.model;
  ui.maxTokens.value = String(view.settings.max_tokens);
  ui.systemPrompt.value = view.settings.system_prompt;
  ui.chatView.hidden = true;
  ui.settingsView.hidden = false;
}

async function saveSettings(): Promise<void> {
  const settings: Settings = {
    provider: ui.provider.value as ProviderKind,
    model: ui.model.value.trim(),
    system_prompt: ui.systemPrompt.value,
    max_tokens: Math.max(1, Number.parseInt(ui.maxTokens.value, 10) || 1),
  };
  try {
    await invoke("save_settings", { settings, apiKey: ui.apiKey.value || null });
    showChat();
  } catch (e) {
    showError(String(e));
  }
}

ui.input.addEventListener("keydown", (event) => {
  if (isSendKey(event)) {
    event.preventDefault();
    void send();
  }
});
ui.send.addEventListener("click", () => void send());
ui.stop.addEventListener("click", () => void call("stop"));
$("new-chat").addEventListener(
  "click",
  () => void call<Snapshot>("new_chat").then(render).then(showChat),
);
$("open-settings").addEventListener("click", () => void openSettings());
$("dismiss-error").addEventListener("click", () => showError(null));
$("cancel-settings").addEventListener("click", showChat);
ui.settingsView.addEventListener("submit", (event) => {
  event.preventDefault();
  void saveSettings();
});
ui.removeKey.addEventListener("click", async () => {
  // Unit commands resolve to `null`; `call` yields `undefined` only on error.
  if ((await call("remove_api_key")) !== undefined) {
    ui.removeKey.hidden = true;
    ui.apiKey.placeholder = "sk-ant-…";
  }
});

void listen<StreamPayload>("stream", ({ payload }) => {
  if (payload.done) {
    // Refresh everything once: error state, title and sidebar order may have changed.
    void call<Snapshot>("get_state").then(render);
    return;
  }
  if (current?.activeId !== payload.conversationId) return;
  const last = ui.messages.lastElementChild?.querySelector(".markdown");
  if (last) last.innerHTML = payload.html; // sanitized by chat-core's markdown::to_html
  // The node no longer matches what was rendered for it.
  renderedKeys[ui.messages.children.length - 1] = "";
});

void call<Snapshot>("get_state").then(render);
