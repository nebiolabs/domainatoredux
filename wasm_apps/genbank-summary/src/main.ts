import "./styles.css";
import wasmBase64 from "virtual:domainator-wasm";

type WasmExports = {
  memory: WebAssembly.Memory;
  alloc: (len: number) => number;
  dealloc: (ptr: number, len: number) => void;
  summarize: (
    genbankPtr: number,
    genbankLen: number,
    groupBy: number,
    domainsPtr: number,
    domainsLen: number,
    evalue: number,
  ) => number;
};

const inputFile = document.querySelector<HTMLInputElement>("#input-file")!;
const runButton = document.querySelector<HTMLButtonElement>("#run-summary")!;
const downloadButton = document.querySelector<HTMLButtonElement>("#download-output")!;
const form = document.querySelector<HTMLFormElement>("#summary-form")!;
const inputPath = document.querySelector<HTMLDivElement>("#input-path")!;
const groupBy = document.querySelector<HTMLSelectElement>("#group-by")!;
const domainNames = document.querySelector<HTMLInputElement>("#domain-names")!;
const evalue = document.querySelector<HTMLInputElement>("#evalue")!;
const status = document.querySelector<HTMLDivElement>("#status")!;
const preview = document.querySelector<HTMLTextAreaElement>("#preview")!;

let selectedFile: File | null = null;
let wasmPromise: Promise<WasmExports> | null = null;

function setStatus(message: string, error = false) {
  status.textContent = message;
  status.dataset.error = error ? "true" : "false";
}

function parsedEvalue(): number | null {
  const value = Number(evalue.value);
  return Number.isFinite(value) && value >= 0 ? value : null;
}

function updateButtons() {
  runButton.disabled = selectedFile === null || parsedEvalue() === null;
  downloadButton.disabled = preview.value.length === 0;
}

function decodeBase64Bytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

async function loadWasm(): Promise<WasmExports> {
  if (!wasmPromise) {
    wasmPromise = WebAssembly.instantiate(decodeBase64Bytes(wasmBase64), {}).then(
      (instance) => instance.exports as WasmExports,
    );
  }
  return wasmPromise;
}

function writeBytes(wasm: WasmExports, bytes: Uint8Array): { ptr: number; len: number } {
  const ptr = wasm.alloc(bytes.length);
  new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, len: bytes.length };
}

function readResult(wasm: WasmExports, ptr: number): string {
  const memory = new DataView(wasm.memory.buffer);
  const len = memory.getUint32(ptr, true);
  const bytes = new Uint8Array(wasm.memory.buffer, ptr + 4, len);
  const text = new TextDecoder().decode(bytes);
  wasm.dealloc(ptr, len + 4);
  return text;
}

function summarizeWithWasm(wasm: WasmExports, text: string, group: string, domains: string, threshold: number): string {
  const encoder = new TextEncoder();
  const input = writeBytes(wasm, encoder.encode(text));
  const domainInput = writeBytes(wasm, encoder.encode(domains));
  try {
    const ptr = wasm.summarize(
      input.ptr,
      input.len,
      group === "definition" ? 1 : 0,
      domainInput.ptr,
      domainInput.len,
      threshold,
    );
    const result = readResult(wasm, ptr);
    if (result.startsWith("ERROR\n")) {
      throw new Error(result.slice("ERROR\n".length));
    }
    return result;
  } finally {
    wasm.dealloc(input.ptr, input.len);
    wasm.dealloc(domainInput.ptr, domainInput.len);
  }
}

inputFile.addEventListener("change", () => {
  selectedFile = inputFile.files?.[0] ?? null;
  inputPath.textContent = selectedFile ? `${selectedFile.name} (${selectedFile.size.toLocaleString()} bytes)` : "No file selected";
  preview.value = "";
  setStatus(selectedFile ? "Ready" : "");
  updateButtons();
});

evalue.addEventListener("input", updateButtons);

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!selectedFile) {
    setStatus("Choose a GenBank file first.", true);
    return;
  }
  const threshold = parsedEvalue();
  if (threshold === null) {
    setStatus("Enter a finite non-negative evalue threshold.", true);
    updateButtons();
    return;
  }

  runButton.disabled = true;
  setStatus("Running summary in WebAssembly...");
  try {
    const [wasm, text] = await Promise.all([loadWasm(), selectedFile.text()]);
    preview.value = summarizeWithWasm(wasm, text, groupBy.value, domainNames.value, threshold);
    setStatus("Summary complete");
  } catch (error) {
    preview.value = "";
    setStatus(String(error), true);
  } finally {
    updateButtons();
  }
});

downloadButton.addEventListener("click", () => {
  const blob = new Blob([preview.value], { type: "text/tab-separated-values;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "genbank_summary.tsv";
  document.body.append(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
});

updateButtons();