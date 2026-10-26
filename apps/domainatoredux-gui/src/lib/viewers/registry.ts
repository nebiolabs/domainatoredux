import type { Component } from "svelte";
import type { FileType } from "../types";
import ContigView from "./ContigView.svelte";
import EnumReport from "./EnumReport.svelte";
import HmmLogo from "./HmmLogo.svelte";
import HmmReport from "./HmmReport.svelte";
import MatrixView from "./MatrixView.svelte";
import SsnView from "./SsnView.svelte";
import SummaryView from "./SummaryView.svelte";
import SvgView from "./SvgView.svelte";
import TableView from "./TableView.svelte";
import TextView from "./TextView.svelte";

// Props every viewer is given. `active` is false while the view is mounted but
// hidden (another tab or sub-view is showing -- see ViewerDock's keep-alive);
// a viewer that acts on window-wide input or on a one-shot `ws` request gates
// on it. Only viewers that need it declare it.
export interface ViewerProps {
  path: string;
  active?: boolean;
}

// One selectable view tab within an opened file.
export interface ViewDescriptor {
  id: string;
  label: string;
  component: Component<ViewerProps>;
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
      // A fast profile table by default (it never computes a logo, so a
      // 25k-profile file opens instantly); the Logo tab draws one profile on
      // demand and carries its own profile browser.
      return [
        { id: "report", label: "Profiles", component: HmmReport },
        { id: "logo", label: "Logo", component: HmmLogo },
      ];
    case "table":
      return [{ id: "table", label: "Table", component: TableView }];
    case "text":
      return [{ id: "text", label: "Text", component: TextView }];
    case "svg":
      return [{ id: "svg", label: "Image", component: SvgView }];
    default:
      return [];
  }
}
