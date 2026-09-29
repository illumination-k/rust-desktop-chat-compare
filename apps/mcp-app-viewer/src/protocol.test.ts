import { expect, test } from "vitest";
import {
  buildCsp,
  defaultSandboxOrigin,
  injectCsp,
  parseResource,
  permissionsToAllow,
  RESOURCE_MIME_TYPE,
} from "./protocol";

test("raw HTML becomes a resource with the fallback URI", () => {
  expect(parseResource("<p>hi</p>", "ui://x/y")).toEqual({
    uri: "ui://x/y",
    html: "<p>hi</p>",
    meta: {},
  });
});

test("resources/read response is unwrapped, including metadata", () => {
  const json = {
    jsonrpc: "2.0",
    id: 1,
    result: {
      contents: [
        {
          uri: "ui://w/d",
          mimeType: RESOURCE_MIME_TYPE,
          text: "<b>x</b>",
          _meta: { ui: { prefersBorder: false } },
        },
      ],
    },
  };
  expect(parseResource(JSON.stringify(json))).toEqual({
    uri: "ui://w/d",
    html: "<b>x</b>",
    meta: { prefersBorder: false },
  });
});

test("base64 blob content is decoded as UTF-8", () => {
  const blob = btoa(String.fromCharCode(...new TextEncoder().encode("<p>日本語</p>")));
  const json = { contents: [{ uri: "ui://a", mimeType: RESOURCE_MIME_TYPE, blob }] };
  expect(parseResource(JSON.stringify(json)).html).toBe("<p>日本語</p>");
});

test("invalid resources are rejected", () => {
  const content = { uri: "ui://a", mimeType: RESOURCE_MIME_TYPE, text: "" };
  expect(() => parseResource(JSON.stringify({ ...content, uri: "https://a" }))).toThrow("ui://");
  expect(() => parseResource(JSON.stringify({ ...content, mimeType: "text/html" }))).toThrow(
    "mimeType",
  );
  expect(() =>
    parseResource(JSON.stringify({ uri: "ui://a", mimeType: RESOURCE_MIME_TYPE })),
  ).toThrow("text or blob");
});

test("CSP defaults to the spec's restrictive policy", () => {
  const csp = buildCsp();
  expect(csp).toContain("default-src 'none'");
  expect(csp).toContain("connect-src 'none'");
  expect(csp).toContain("frame-src 'none'");
  expect(csp).toContain("object-src 'none'");
  expect(csp).toContain("base-uri 'self'");
});

test("CSP allows declared domains only, dropping injection attempts", () => {
  const csp = buildCsp({
    connectDomains: ["https://api.example.com"],
    resourceDomains: ["https://cdn.example.com", "https://x; script-src *"],
  });
  expect(csp).toContain("connect-src https://api.example.com");
  expect(csp).toContain("script-src 'self' 'unsafe-inline' https://cdn.example.com;");
  expect(csp).not.toContain("script-src *");
});

test("CSP meta is placed right after the doctype", () => {
  expect(injectCsp("<!DOCTYPE html><html><script>x</script>", "a 'b'")).toBe(
    `<!DOCTYPE html><meta http-equiv="Content-Security-Policy" content="a 'b'"><html><script>x</script>`,
  );
  expect(injectCsp("<p>x</p>", "p")).toBe(
    `<meta http-equiv="Content-Security-Policy" content="p"><p>x</p>`,
  );
});

test("permissions map to Permission Policy features", () => {
  expect(permissionsToAllow({ camera: {}, clipboardWrite: {} })).toBe("camera; clipboard-write");
  expect(permissionsToAllow()).toBe("");
});

test("sandbox origin swaps localhost and 127.0.0.1", () => {
  expect(defaultSandboxOrigin({ protocol: "http:", hostname: "localhost", port: "5180" })).toBe(
    "http://127.0.0.1:5180",
  );
  expect(defaultSandboxOrigin({ protocol: "http:", hostname: "127.0.0.1", port: "" })).toBe(
    "http://localhost",
  );
  expect(
    defaultSandboxOrigin({ protocol: "https:", hostname: "example.com", port: "" }),
  ).toBeNull();
});
