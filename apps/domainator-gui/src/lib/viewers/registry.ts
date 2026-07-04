import type { Component } from "svelte";
import type { FileType } from "../types";
import ContigView from "./ContigView.svelte";
import EnumReport from "./EnumReport.svelte";
import HmmLogo from "./HmmLogo.svelte";
import MatrixView from "./MatrixView.svelte";
import SsnView from "./SsnView.svelte";
import SummaryView from "./SummaryView.svelte";

// One selectable view tab within an opened file. `component` takes a `path` prop.
export interface ViewDescriptor {
  id: string;
  label: string;
  component: Component<{ path: string }>;
}

// Maps a file type to its ordered view tabs. Sequence files will grow Enum /
// Summary tabs in a later phase; today they have only the contig view.
export function viewsFor(fileType: FileType): ViewDescriptor[] {
  switch (fileType) {
    case "genbank":
    case "fasta":
      return [
        { id: "contig", label: "Contig View", component: ContigView },
        { id: "enum", label: "Enum Report", component: EnumReport },
        { id: "summary", label: "Summary", component: SummaryView },
      ];
    case "hdf5":
      return [{ id: "matrix", label: "Matrix Report", component: MatrixView }];
    case "ssn_viewer":
      return [{ id: "ssn", label: "SSN Viewer", component: SsnView }];
    case "hmm":
      return [{ id: "logo", label: "HMM Logo", component: HmmLogo }];
    default:
      return [];
  }
}
