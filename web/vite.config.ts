import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  build: {
    target: "es2022",
    sourcemap: false,
    cssCodeSplit: true,
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    // Develop the UI against a local ALPS web instance. The public API is
    // /?url=..., so no path rewrite or bearer token is needed.
    proxy: {
      "^/\\?url=": {
        target: "http://127.0.0.1:8080",
        changeOrigin: false,
      },
    },
  },
});
