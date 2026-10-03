import path from "node:path";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The production build is a plain static folder (`dist/`) that the Rust
// `trss-web` binary serves. `vite dev` is a development convenience only and
// proxies `/api` to a locally running `trss-web`, WebSockets included (the
// remote screen). It keeps the browser's `Host`, which the web's Host and
// Origin checks compare with the `Origin`, so `changeOrigin` stays off.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "src") },
  },
  server: {
    proxy: {
      "/api": { target: process.env.TRSS_WEB_DEV_API ?? "http://127.0.0.1:8080", ws: true },
    },
  },
});
