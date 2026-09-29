import { defineConfig } from "oxlint";

export default defineConfig({
  categories: {
    correctness: "error",
    suspicious: "warn",
  },
  ignorePatterns: ["**/dist", "target", "apps/tauri/src-tauri/gen"],
});
