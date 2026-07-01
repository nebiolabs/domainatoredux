import { defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const wasmPath = resolve(
  __dirname,
  "wasm/target/wasm32-unknown-unknown/release/domainator_genbank_summary_wasm.wasm",
);

function domainatorWasmPlugin() {
  return {
    name: "domainator-wasm-inline",
    resolveId(id: string) {
      return id === "virtual:domainator-wasm" ? id : null;
    },
    load(id: string) {
      if (id !== "virtual:domainator-wasm") {
        return null;
      }
      const wasmBase64 = readFileSync(wasmPath).toString("base64");
      return `export default ${JSON.stringify(wasmBase64)};`;
    },
  };
}

export default defineConfig({
  base: "./",
  clearScreen: false,
  plugins: [domainatorWasmPlugin()],
  build: {
    cssCodeSplit: false,
    modulePreload: false,
  },
  server: {
    strictPort: true,
    host: "127.0.0.1",
    port: 1430,
  },
});