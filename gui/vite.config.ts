import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed dev port and leaves the build output in `dist/`.
// CEC Support owns 1432/1433 so its dev shell can run beside AllMyStuff's
// 1430/1431 pair on the same machine.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // Force the browser export of `svelte` so `mount()` resolves to the
  // client build rather than the SSR stub (which throws
  // `lifecycle_function_unavailable` and leaves the WebView blank).
  resolve: {
    conditions: ["browser", "module", "import", "default"],
  },
  server: {
    port: 1432,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1433 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
