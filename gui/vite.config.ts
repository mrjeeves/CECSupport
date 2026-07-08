import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed dev port and leaves the build output in `dist/`.
// Mirrors the AllMyStuff / MyOwnMesh GUI setup so `pnpm tauri dev|build`
// works the same way across the product family.
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
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
