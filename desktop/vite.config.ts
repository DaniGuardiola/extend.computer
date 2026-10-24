import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
export default defineConfig(({ mode, command }) => ({
  resolve: {
    alias:
      mode === "ui-review" && command === "serve"
        ? {
            "@tauri-apps/api/core": new URL(
              "./src/review/core.ts",
              import.meta.url,
            ).pathname,
            "./Appearance": new URL(
              "./src/review/appearance.tsx",
              import.meta.url,
            ).pathname,
            "@tauri-apps/api/window": new URL(
              "./src/review/window.ts",
              import.meta.url,
            ).pathname,
          }
        : undefined,
  },
  plugins: [
    react({ babel: { plugins: ["babel-plugin-react-compiler"] } }),
    tailwindcss(),
  ],
  server: {
    port: mode === "ui-review" ? 1421 : 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  clearScreen: false,
}));
