import {
  isSandboxMessage,
  type JsonRpcError,
  type JsonRpcId,
  type JsonRpcMessage,
  type UiResource,
} from "./protocol";

export type Direction = "host→view" | "view→host";

/** Handler for a request from the view. Throw a `RpcError` to answer with an error. */
type RequestHandler = (params: any) => unknown;

export class RpcError extends Error {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(message);
  }
}

export interface BridgeOptions {
  resource: UiResource;
  /** Posts a message to the sandbox proxy iframe. */
  post: (msg: JsonRpcMessage) => void;
  /** Builds the `McpUiInitializeResult` from the view's `ui/initialize` params. */
  initialize: (params: any) => object;
  handlers: Record<string, RequestHandler>;
  onNotification?: (method: string, params: any) => void;
  onInitialized?: () => void;
  log?: (direction: Direction, msg: JsonRpcMessage) => void;
}

/**
 * Host side of the MCP Apps JSON-RPC channel. Transport-agnostic: feed it
 * messages with `receive` and it answers through `post`.
 */
export class AppBridge {
  private initialized = false;
  private nextId = 1;
  private readonly pending = new Map<JsonRpcId, (msg: JsonRpcMessage) => void>();

  constructor(private readonly options: BridgeOptions) {}

  get isInitialized(): boolean {
    return this.initialized;
  }

  async receive(msg: JsonRpcMessage): Promise<void> {
    this.options.log?.("view→host", msg);
    if (msg.method === "ui/notifications/sandbox-proxy-ready") {
      const { html, meta } = this.options.resource;
      this.send({
        jsonrpc: "2.0",
        method: "ui/notifications/sandbox-resource-ready",
        params: { html, csp: meta.csp, permissions: meta.permissions },
      });
      return;
    }
    if (isSandboxMessage(msg)) return;
    if (msg.method === undefined) {
      // A response to one of our requests.
      if (msg.id !== undefined) this.pending.get(msg.id)?.(msg);
      return;
    }
    if (msg.id === undefined) {
      if (msg.method === "ui/notifications/initialized" && !this.initialized) {
        this.initialized = true;
        this.options.onInitialized?.();
      }
      this.options.onNotification?.(msg.method, msg.params);
      return;
    }
    this.send(await this.respond(msg.id, msg.method, msg.params));
  }

  /** Sends a notification. The spec forbids this before the view is initialized. */
  notify(method: string, params: unknown): void {
    if (!this.initialized) throw new Error(`View is not initialized; cannot send ${method}`);
    this.send({ jsonrpc: "2.0", method, params });
  }

  /** Sends a request and waits for the response (rejecting after `timeoutMs`). */
  request(method: string, params: unknown, timeoutMs = 5000): Promise<unknown> {
    if (!this.initialized)
      return Promise.reject(new Error(`View is not initialized; cannot send ${method}`));
    const id = `host-${this.nextId++}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out`));
      }, timeoutMs);
      this.pending.set(id, (msg) => {
        clearTimeout(timer);
        this.pending.delete(id);
        if (msg.error) reject(new Error(msg.error.message));
        else resolve(msg.result);
      });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }

  private async respond(id: JsonRpcId, method: string, params: unknown): Promise<JsonRpcMessage> {
    const handler =
      method === "ui/initialize" ? this.options.initialize : this.options.handlers[method];
    if (!handler)
      return {
        jsonrpc: "2.0",
        id,
        error: { code: -32601, message: `Method not found: ${method}` },
      };
    try {
      return { jsonrpc: "2.0", id, result: (await handler(params)) ?? {} };
    } catch (e) {
      const error: JsonRpcError =
        e instanceof RpcError
          ? { code: e.code, message: e.message }
          : { code: -32603, message: String(e) };
      return { jsonrpc: "2.0", id, error };
    }
  }

  private send(msg: JsonRpcMessage): void {
    this.options.log?.("host→view", msg);
    this.options.post(msg);
  }
}
