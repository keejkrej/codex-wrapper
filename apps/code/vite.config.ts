import tailwindcss from "@tailwindcss/vite";
import preact from "@preact/preset-vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import pkg from "./package.json" with { type: "json" };

const port = Number(process.env.PORT ?? 5733);
const isVitest = process.env.VITEST === "true";
const sourcemapEnv = process.env.T3CODE_WEB_SOURCEMAP?.trim().toLowerCase();

const buildSourcemap =
  sourcemapEnv === "0" || sourcemapEnv === "false"
    ? false
    : sourcemapEnv === "hidden"
      ? "hidden"
      : true;

export default defineConfig({
  plugins: [tanstackRouter(), ...(!isVitest ? [preact()] : []), tailwindcss()],
  optimizeDeps: {
    include: ["@pierre/diffs", "@pierre/diffs/react", "@pierre/diffs/worker/worker.js"],
  },
  define: {
    // In dev mode, tell the web app where the WebSocket server lives
    "import.meta.env.VITE_WS_URL": JSON.stringify(process.env.VITE_WS_URL ?? ""),
    "import.meta.env.APP_VERSION": JSON.stringify(pkg.version),
  },
  resolve: {
    alias: !isVitest
      ? [
          {
            find: "react-dom/test-utils",
            replacement: "preact/test-utils",
          },
          {
            find: "react-dom/server",
            replacement: "preact-render-to-string",
          },
          {
            find: "react-dom/client",
            replacement: "preact/compat/client",
          },
          {
            find: "react-dom",
            replacement: "preact/compat",
          },
          {
            find: "react/jsx-runtime",
            replacement: "preact/jsx-runtime",
          },
          {
            find: "react/jsx-dev-runtime",
            replacement: "preact/jsx-dev-runtime",
          },
          {
            find: "react",
            replacement: fileURLToPath(new URL("./src/react-compat.js", import.meta.url)),
          },
        ]
      : [],
    tsconfigPaths: true,
  },
  server: {
    port,
    strictPort: true,
    hmr: {
      // Explicit config so Vite's HMR WebSocket connects reliably inside
      // the Tauri dev shell as well as a regular browser.
      protocol: "ws",
      host: "localhost",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: buildSourcemap,
  },
});
