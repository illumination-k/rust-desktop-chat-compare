import { defineConfig } from "vite";

// The sandbox proxy must be served from a different origin than the host page.
// Binding to 127.0.0.1 lets the same server answer as both `localhost` (host)
// and `127.0.0.1` (sandbox); see `defaultSandboxOrigin`.
const server = { host: "127.0.0.1", port: 5180, strictPort: true };

export default defineConfig({
  base: "./",
  clearScreen: false,
  server,
  preview: server,
  build: {
    target: "es2022",
    rollupOptions: { input: ["index.html", "sandbox.html"] },
  },
});
