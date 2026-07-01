import { readFile, rm, unlink, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const indexPath = resolve(appRoot, "dist/index.html");
const html = await readFile(indexPath, "utf8");
const scriptMatch = html.match(/<script type="module" crossorigin src="\.\/(assets\/[^\"]+\.js)"><\/script>/);
const styleMatch = html.match(/<link rel="stylesheet" crossorigin href="\.\/(assets\/[^\"]+\.css)">/);

if (!scriptMatch) {
  throw new Error("Could not find Vite script tag in dist/index.html");
}

if (!styleMatch) {
  throw new Error("Could not find Vite stylesheet tag in dist/index.html");
}

const scriptPath = resolve(appRoot, "dist", scriptMatch[1]);
const script = await readFile(scriptPath, "utf8");
const escapedScript = script.replaceAll("</script", "<\\/script");
const stylePath = resolve(appRoot, "dist", styleMatch[1]);
const style = await readFile(stylePath, "utf8");
const packedHtml = html
  .replace(scriptMatch[0], "")
  .replace(styleMatch[0], `<style>${style}</style>`)
  .replace("</body>", `    <script>${escapedScript}</script>\n  </body>`);

await writeFile(indexPath, packedHtml);
await unlink(scriptPath);
await unlink(stylePath);
await rm(resolve(appRoot, "dist/assets"), { recursive: true, force: true });

await unlink(resolve(appRoot, "dist/domainator_genbank_summary_wasm.wasm")).catch((error) => {
  if (error?.code !== "ENOENT") {
    throw error;
  }
});