import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  root: "ui",
  plugins: [react()],
  resolve: { alias: { "@": path.resolve(path.dirname(fileURLToPath(import.meta.url)), "ui/src") } },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "../dist", emptyOutDir: true, target: "es2021", chunkSizeWarningLimit: 2000 },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
  },
});
