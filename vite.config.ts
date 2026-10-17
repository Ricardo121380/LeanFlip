import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const here = path.dirname(fileURLToPath(import.meta.url));
export default defineConfig(({ command, mode }) => {
  // `pnpm dev:mock` swaps the Tauri bridge for an in-memory demo backend (browser only, never bundled).
  const mockBackend = command === "serve" && (mode === "mock" || process.env.LEANFLIP_MOCK === "1")
    ? [{ find: /^@tauri-apps\/api\/core$/, replacement: path.resolve(here, "ui/src/dev/mock-tauri.ts") }]
    : [];

  return {
    root: "ui",
    plugins: [react()],
    resolve: { alias: [...mockBackend, { find: "@", replacement: path.resolve(here, "ui/src") }] },
    clearScreen: false,
    server: { port: 1420, strictPort: true },
    build: { outDir: "../dist", emptyOutDir: true, target: "es2021", chunkSizeWarningLimit: 2000 },
    test: {
      environment: "jsdom",
      globals: true,
      setupFiles: ["./src/test/setup.ts"],
      css: false,
    },
  };
});
