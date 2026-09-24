import { resolve } from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Serveur de dev accessible uniquement depuis la machine.
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "safari16",
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "index.html"),
        indicator: resolve(import.meta.dirname, "indicator.html"),
      },
    },
  },
});
