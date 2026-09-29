import { defineConfig } from "oxlint";

export default defineConfig({
  categories: {
    correctness: "error",
    suspicious: "warn",
  },
  rules: {
    // `_meta` is a field name defined by the MCP spec.
    "no-underscore-dangle": ["warn", { allow: ["_meta"] }],
  },
  ignorePatterns: ["**/dist", "target", "apps/tauri/src-tauri/gen"],
});
