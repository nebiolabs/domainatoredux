import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const openButton = document.querySelector<HTMLButtonElement>("#open-input")!;
const runButton = document.querySelector<HTMLButtonElement>("#run-summary")!;
const saveButton = document.querySelector<HTMLButtonElement>("#save-output")!;
const form = document.querySelector<HTMLFormElement>("#summary-form")!;
const inputPath = document.querySelector<HTMLDivElement>("#input-path")!;
const groupBy = document.querySelector<HTMLSelectElement>("#group-by")!;
const domainNames = document.querySelector<HTMLInputElement>("#domain-names")!;
const evalue = document.querySelector<HTMLInputElement>("#evalue")!;
const status = document.querySelector<HTMLDivElement>("#status")!;
const preview = document.querySelector<HTMLTextAreaElement>("#preview")!;

let selectedInputPath = "";

function setStatus(message: string, error = false) {
  status.textContent = message;
  status.dataset.error = error ? "true" : "false";
}

function parsedEvalue(): number | null {
  const value = Number(evalue.value);
  return Number.isFinite(value) && value >= 0 ? value : null;
}

function updateButtons() {
  runButton.disabled = selectedInputPath.length === 0 || parsedEvalue() === null;
  saveButton.disabled = preview.value.length === 0;
}

openButton.addEventListener("click", async () => {
  try {
    const path = await invoke<string | null>("select_genbank_file");
    if (!path) {
      return;
    }
    selectedInputPath = path;
    inputPath.textContent = path;
    preview.value = "";
    setStatus("Ready");
  } catch (error) {
    setStatus(String(error), true);
  } finally {
    updateButtons();
  }
});

evalue.addEventListener("input", updateButtons);

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const threshold = parsedEvalue();
  if (threshold === null) {
    setStatus("Enter a finite non-negative evalue threshold.", true);
    updateButtons();
    return;
  }

  runButton.disabled = true;
  setStatus("Running summary...");
  try {
    preview.value = await invoke<string>("summarize_genbank", {
      inputPath: selectedInputPath,
      groupBy: groupBy.value,
      domainNamesText: domainNames.value,
      evalueThreshold: threshold,
    });
    setStatus("Summary complete");
  } catch (error) {
    preview.value = "";
    setStatus(String(error), true);
  } finally {
    updateButtons();
  }
});

saveButton.addEventListener("click", async () => {
  try {
    const path = await invoke<string | null>("save_tsv_file", { contents: preview.value });
    if (path) {
      setStatus(`Saved ${path}`);
    }
  } catch (error) {
    setStatus(String(error), true);
  } finally {
    updateButtons();
  }
});

updateButtons();