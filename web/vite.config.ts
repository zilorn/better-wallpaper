import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  server: {
    host: "127.0.0.1",
    proxy: {
      "/api": "http://127.0.0.1:43127",
      "/ws": { target: "ws://127.0.0.1:43127", ws: true },
    },
  },
  build: { target: "es2022" },
});
