import { expect, test } from "vitest";
import { AppBridge, RpcError } from "./bridge";
import type { JsonRpcMessage } from "./protocol";

function setup() {
  const sent: JsonRpcMessage[] = [];
  let initialized = 0;
  const bridge = new AppBridge({
    resource: {
      uri: "ui://a",
      html: "<p>x</p>",
      meta: { csp: { connectDomains: ["https://api"] } },
    },
    post: (msg) => sent.push(msg),
    initialize: () => ({ protocolVersion: "v" }),
    handlers: {
      echo: (params) => params,
      deny: () => {
        throw new RpcError(-32000, "denied");
      },
    },
    onInitialized: () => initialized++,
  });
  return { bridge, sent, initialized: () => initialized };
}

test("sends the resource once the sandbox proxy is ready", async () => {
  const { bridge, sent } = setup();
  await bridge.receive({
    jsonrpc: "2.0",
    method: "ui/notifications/sandbox-proxy-ready",
    params: {},
  });
  expect(sent).toEqual([
    {
      jsonrpc: "2.0",
      method: "ui/notifications/sandbox-resource-ready",
      params: {
        html: "<p>x</p>",
        csp: { connectDomains: ["https://api"] },
        permissions: undefined,
      },
    },
  ]);
});

test("answers requests with results, errors, and method-not-found", async () => {
  const { bridge, sent } = setup();
  await bridge.receive({ jsonrpc: "2.0", id: 1, method: "ui/initialize", params: {} });
  await bridge.receive({ jsonrpc: "2.0", id: 2, method: "echo", params: { a: 1 } });
  await bridge.receive({ jsonrpc: "2.0", id: 3, method: "deny" });
  await bridge.receive({ jsonrpc: "2.0", id: 4, method: "nope" });
  expect(sent).toEqual([
    { jsonrpc: "2.0", id: 1, result: { protocolVersion: "v" } },
    { jsonrpc: "2.0", id: 2, result: { a: 1 } },
    { jsonrpc: "2.0", id: 3, error: { code: -32000, message: "denied" } },
    { jsonrpc: "2.0", id: 4, error: { code: -32601, message: "Method not found: nope" } },
  ]);
});

test("refuses to notify the view before it is initialized", async () => {
  const { bridge, sent, initialized } = setup();
  expect(() => bridge.notify("ui/notifications/tool-input", {})).toThrow("not initialized");
  await bridge.receive({ jsonrpc: "2.0", method: "ui/notifications/initialized" });
  expect(initialized()).toBe(1);
  bridge.notify("ui/notifications/tool-input", { arguments: {} });
  expect(sent).toEqual([
    { jsonrpc: "2.0", method: "ui/notifications/tool-input", params: { arguments: {} } },
  ]);
});

test("resolves host requests with the view's response", async () => {
  const { bridge, sent } = setup();
  await bridge.receive({ jsonrpc: "2.0", method: "ui/notifications/initialized" });
  const pending = bridge.request("ui/resource-teardown", { reason: "x" });
  await bridge.receive({ jsonrpc: "2.0", id: sent[0].id, result: { ok: true } });
  await expect(pending).resolves.toEqual({ ok: true });
});
