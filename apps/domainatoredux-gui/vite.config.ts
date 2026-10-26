import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri drives dev/build via package.json scripts; keep the fixed dev port and
// quiet the screen-clear so Tauri's own logs stay visible.
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  // Component tests mount in jsdom, which needs Svelte's browser build, not the
  // server one vitest (mode `test`) picks under Node.
  resolve: mode === "test" ? { conditions: ["browser"] } : undefined,
  clearScreen: false,
  server: {
    strictPort: true,
    host: "127.0.0.1",
    port: 1420,
  },
}));
