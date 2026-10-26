# GUI tool specs

Each `<id>.yml` here is the desktop app's policy for one tool's dialog, layered over the
tool's manifest entry (`domainatoredux describe --tool <id>`):
- its menu label
- the fields the dialog shows, their labels, and in what order
- which files each input offers
- what an option starts out as, where the dialog should differ from the command line

The manifest supplies everything else: each option's type, choices, bounds, CLI default
and help, the tool's Tools-menu group, and, for a tool on the 1.0 interface, which
options are files to read or write. `build.rs` embeds these files at compile time, and
`tool_specs::specs` resolves them against the manifest when the app starts.
`cmd_tools::list_tools` serves the result to `ToolDialog.svelte`. Nothing else in the
app lists the tools the GUI offers. Editing a file takes effect on the next build.

## Adding a tool or an option

- **New tool:** write `<id>.yml`. The file name must equal the tool's `id`, and the CLI
  must have a subcommand of that name. Nothing else is needed: a job runs
  `domainatoredux <id> --config <the dialog's params>` in a worker process.
- **File fields drive provenance and Cancel.** An input's role in the job's provenance
  record comes from its `file_type` (a field named `reference`/`references` is always
  `reference`), and the SSN viewer's node → sequence tracing follows those roles. A
  canceled job deletes the files named by its `output_file` fields that it created or
  changed, so a field that names an output file should have that role.
- **New CLI option:** a new manifest parameter makes
  `every_manifest_parameter_is_listed_once` fail until the option is listed here, with
  `show: false` if the dialog should not offer it. That failure is how you decide about
  the option. `-v`/`-q` are never listed.

## Example

```yaml
id: sort_contigs
label: "Sort contigs"
description: "Shown under the dialog title."
fields:
  - name: input                       # an input: the manifest says so
    label: "Input sequences"
    file_type: sequence               # one value, or a list
    sources: workspace_first
    select: first
  - name: genbank_out                 # an output: its name ends in _out
    label: "Output (GenBank)"
    output_name: "{stem}.sorted.gb"
  - name: max_hits
    label: "Keep top N"
    gui_default: 100                  # the command line's default is none
  - name: databases
    show: false
    label: "Databases"
```

## Tool keys

| Key | Required | What it does |
|---|---|---|
| `id` | yes | The tool's id, as in `domainatoredux <id>` and `domainatoredux describe --tool <id>`. It must equal the file name without `.yml`. A job runs `domainatoredux <id> --config …`. |
| `label` | yes | The name shown in the Tools menu, on the toolbar's Tools button, and as the dialog title. |
| `description` | yes | The sentence under the dialog title. |
| `fields` | yes | Every parameter of the tool's manifest entry except `verbose`/`quiet`, once each. List order is dialog order. Hidden fields can go anywhere; the generated files put them last. |

The Tools-menu group is the manifest's `category`, shown with its label from
`cmd_tools::CATEGORY_LABELS` (upstream's groups, in upstream's order). Within one,
tools appear in file-name order.

## Field keys

| Key | Applies to | Default | What it does |
|---|---|---|---|
| `name` | all | (required) | The manifest parameter, spelled as `domainatoredux describe` spells it (`max_overlap`, not `max-overlap`). |
| `label` | all | (required) | The text left of the control. |
| `required` | all | the manifest's | Adds a red `*` to the label. On an input, removes the "(none)" entry. On a dropdown option, removes the blank "(default)" entry. A spec can make a field required, never optional. A required field cannot be hidden. A required output needs an `output_name`. The dialog doesn't block Run when a required field is blank. The tool refuses instead. |
| `show` | all | `true` | `false` keeps the option out of the dialog. It is never sent, so the CLI default applies. The one exception is `cpu`, which gets the Preferences thread count when that isn't 0. A program path (`*_path` other than a directory) is never shown. |
| `help` | shown fields | the manifest's `help` | The hint under the control. See [Help text](#help-text). |
| `gui_default` | shown `param` / `columns` fields | none | What the dialog starts with instead of the CLI default. See [Defaults](#defaults). |
| `file_type` | inputs | (required) | Which files and databases the dropdown offers. See [File types](#file-types). |
| `sources` | inputs | (required) | Which dropdown group comes first: `databases_first` or `workspace_first`. |
| `select` | inputs | (required) | The dropdown's starting entry when no selected file was placed in it. See [Input dropdowns](#input-dropdowns). |
| `prefill_priority` | inputs | `0` | Which inputs selected files fill first. See [Selected files](#selected-files). |
| `output_name` | outputs | none | The file name the output starts with. See [Outputs](#outputs). |
| `column_types` | `columns` | none | Per-type labels, hints, parameter defaults and suggested values for the column builder. See [Column builder](#column-builder). |

## Roles

| Role | Control | Sent to the tool as |
|---|---|---|
| `param` | Drawn from the parameter's manifest type (below) | The typed value |
| `input_file` | A dropdown of databases and files | A string: `"input": "a.gb"` |
| `input_files` | The same dropdown, one choice per row, with "+ Add file" | A list: `"input": ["a.gb"]` |
| `output_file` | A file-name box | The name joined onto the current folder |
| `columns` | A report's column builder: an ordered list of columns and their arguments. See [Column builder](#column-builder). | A list of column tokens, `name:arg:arg`, in list order |

A field's role is derived from its manifest parameter, never written here: `kind:
columns` is `columns`; `role: input` is `input_files` when the parameter takes several
values, else `input_file`; `role: output` (a name ending in `_out`) is `output_file`;
`role: directory` is `input_file` when the field has a `file_type` (the taxonomy
database), else `param`; everything else is `param`.

A `param` is drawn by its manifest type:

| Manifest | Control | Blank / unchecked |
|---|---|---|
| has `choices` | a dropdown of the choices, with a "(default)" entry when the field isn't required | not sent |
| `kind: flag` | a checkbox | not sent (the CLI's default) |
| `multiple` | a text box of comma-separated values; numbers are converted for numeric lists | not sent |
| `type: integer`, `number` | a number box, bounded by `min`/`max` | not sent |
| anything else | a text box | not sent |

Anything not sent falls back to the CLI default.

## File types

`file_type` is either one value (`file_type: hmm`) or a list (`file_type: [sequence, hmm]`).
The dropdown offers a file or database that matches any entry. Unknown values fail
`input_fields_declare_sources_and_select`, since they would match nothing.

A workspace file's type comes from its extension alone (`classify_file` in
`crates/domainator-core/src/app_support/workspace.rs`), ignoring one trailing `.gz`, `.bgz`
or `.bgzf`:

| `file_type` | Workspace files | Databases |
|---|---|---|
| `genbank` | `.gb` `.gbk` `.genbank` `.gbff` | GenBank |
| `fasta` | `.fasta` `.fa` `.fna` `.faa` `.pep` | FASTA |
| `hmm` | `.hmm` `.hmmer` | HMM |
| `sequence` | `genbank` or `fasta` | GenBank, FASTA |
| `reference` | `hmm` or `fasta` | HMM, FASTA |
| `hdf5` | `.hdf5` `.hdf` `.h5` | none |
| `stockholm` | `.sto` `.stockholm` `.sth` | none |
| `table` | `.tsv` `.csv` `.tab` | none |
| `text` | `.txt` `.text` `.list` | none |
| `cm` | `.cm` | none |
| `svg` | `.svg` | none |
| `ssn_viewer` | `.dsnv` `.ssnv` `.ssnview` | none |
| `taxonomy` | none | NCBI taxonomy (`taxdump*.tar.gz`, e.g. the managed NCBI Taxonomy download) |

`file_type` only filters what the dropdown offers. The tool receives the chosen path and
reads it however it reads that input, so list only types the tool can actually take.

`file_type` also decides which inputs the selected files can go to. See
[Selected files](#selected-files).

## Input dropdowns

An input lists two groups with a bar between them:
- **Databases:** everything in the Databases panel whose format matches.
- **Workspace:** files in the current folder whose type matches.

`sources` decides which group comes first. A path that is in both groups appears once, in
the first group. An empty group isn't shown. When both are empty, the dropdown is replaced
by "No … files/databases available".

An input a selected file was placed in starts on that file (see
[Selected files](#selected-files)). Otherwise `select` picks the starting entry:

| `select` | Starts on | When that isn't on offer |
|---|---|---|
| `database:<id>` | the managed reference download with that id: `pfam`, `swissprot`, `uniref50`, `uniref90`, `trembl` or `ncbi_taxonomy` (`reference_dbs::specs`) | the first entry |
| `first` | the first entry, which `sources` decides | blank, when nothing is on offer |
| `none` | blank ("(none)") | blank |

`none` suits optional inputs. A test checks that a `database:<id>` exists and that its
format can appear in the field's dropdown.

## Selected files

Every way into a dialog starts it with the selected files placed in its inputs. That
covers the Tools menu, the toolbar's Tools button and the file table's right-click Tools
submenu. The "selected files" are the file table's selection. With nothing selected, they
are the active tab's file, but only when it is in the current folder.

A file can go to any input whose `file_type` accepts it (`placeFiles` in
`src/lib/toolInputs.ts`):
- An `input_file` takes one file, and an `input_files` takes any number.
- A file only single inputs can take is matched first, so it always gets an input when
  one is free. A GenBank file goes to domain_search's `input` whatever else is selected.
- The other files go to the first list input that takes their type. A type with just one
  selected file goes to a free single input instead, when one comes before that list
  input.
- "First" and "before" are in `prefill_priority` order: higher first, ties in field
  order. domain_search and domainate set `prefill_priority: 1` on `references`, so a
  selected FASTA becomes a reference and `input` starts on its `select`.

The toolbar's Tools button and the right-click submenu offer only the tools that can
take every selected file at once. The Tools menu offers every tool. Its dialog places
what fits and lists the files it left out under the directory line.

## Column builder

A `columns` field (enum_report's and hmmer_report's `columns`) is an ordered list. The
Enum Report viewer tab uses the same builder, with `enum_report.yml`'s `column_types`. Each
row is a column plus its arguments, and the list order is the report's column order. ↑/↓
reorder the rows.

**The columns come from the manifest, not from this file.** `domainatoredux describe
--tool <id>` lists them under the `columns` parameter, one entry per column:
- its `name`, shown in the dropdown
- its `help`, shown as the row's hint
- its `args`, the argument names, e.g. `qualifier` → `feature`, `qualifier`
- `repeat_last`: the last argument takes one or more values (`taxid:genus:species`)

The catalog is `crates/domainator-config/src/columns.rs`, the same one that parses the
run's `--columns`, so a new column appears in the builder without any change here.

A row is sent as a token: the column's name and its arguments joined by `:`. The last
argument keeps any `:` it contains; a repeating argument is a comma-separated box, and
each value becomes one `:`-separated part. An empty argument, or a `:` where the token
cannot hold one, stops the run before it starts, naming the column.

**`column_types` adjusts how the builder shows them.** It is keyed by column name:

```yaml
    column_types:
      taxname:
        label: "Taxon name"        # replaces the column's name in the dropdown
        help: "..."                # replaces the column's help under the row
        show: false                # leave the column out of the dropdown
        params:
          rank:
            label: "Ranks"         # the text beside the box (default: the argument name)
            help: "..."            # tooltip and placeholder
            default: [species]     # what a newly added column starts with
            choices: [self, domain, superkingdom, phylum, genus, species]
```

Every key is optional. A repeating argument shows its `choices` as one-click chips that
add or remove a value; any other argument shows them as a suggestion list. Values outside
`choices` can always be typed. A new row starts each argument at its `default`, else blank.
Changing a row's column keeps the values of arguments the two columns share.

`gui_default` for a `columns` field is the list of tokens the builder starts with, e.g.
`[domains, taxname:genus]`. Each must parse (`gui_defaults_fit_their_property`) and must
not use a column `column_types` hides.

## Outputs

An `output_file` is a file-name box. The app writes the file into the current folder, the
one named at the top of the dialog. `output_name` sets the starting name:
- `{stem}` is replaced with the primary input's name without its final extension.
  For `/data/run1.search.gb`, the stem is `run1.search`.
- The primary input is the field named `input` when it has a value, otherwise the first
  input with one.
- With no input chosen, `{stem}` becomes `output`.
- Changing the primary input updates templated output names until you edit an output
  name yourself. Cloned jobs keep their saved output names.
- `output_name` must be a file name, not a path. It can use no placeholder other than
  `{stem}`.

An output without `output_name` starts blank. A blank output isn't sent, so that file isn't
written. Every required output needs an `output_name`.

## Defaults

- **The CLI default** is the manifest's `default`. A spec does not repeat it.
- **`gui_default`** is what the dialog starts with instead:
  - It must fit the parameter: its type, its choices, its bounds, a list when it takes
    several values.
  - It is allowed only on shown `param` and `columns` fields. Files
    are chosen with `select` and `output_name`.
  - A list is written as a YAML list (`[a, b]`) and shows in the box as `a,b`.
  - When `gui_default` and the CLI default differ, the dialog says so under the field.
- **Clearing a field** sends nothing, and the run falls back to the CLI default: the
  params go to the CLI itself.
- **A cloned job** (the Jobs panel's ⎘ button, or "Clone into Tool Dialog") starts from
  that job's saved values. Those replace
  these defaults for every field the job set.

## Help text

The hint under a field is the manifest parameter's `help`, the same text
`domainatoredux describe` gives agents: the doc comment of the option in
`crates/domainator-config/src/args/<tool>.rs`.

- Setting `help:` replaces the hint in the dialog only. Leave the key out to keep the
  manifest's text.
- A `help` on a hidden field, or an empty one, is a test failure.
- On a `columns` field, `help` is the hint under the whole builder, and it is
  required: the manifest's is the command line's list of every column. Each row's own
  hint comes from its column (see [Column builder](#column-builder)).

## What checks these files

| Check | Catches |
|---|---|
| `build.rs` (the build fails) | invalid YAML, a misspelled or unknown key, a bad `select` value |
| `every_manifest_parameter_is_listed_once` | a manifest parameter missing here, a field that isn't a manifest parameter, a field listed twice |
| `gui_defaults_fit_their_property` | a `gui_default` of the wrong type, outside its choices or bounds, on a hidden field, or on a file field; for `columns`, a token that does not parse or a hidden column |
| `input_fields_declare_sources_and_select` | a shown input missing `file_type`/`sources`/`select`; an unknown `file_type`; an unknown or unfitting `database:<id>`; those keys or `prefill_priority` on a non-input; a hidden required field; a shown program path; a hidden or empty `help` |
| `column_type_overrides_name_real_columns_and_arguments` | a `column_types` column or argument the catalog lacks; a `default` or `choice` that isn't text; `column_types` on a non-`columns` field; every column hidden |
| `required_outputs_have_a_name_template` | a required output with no `output_name`, one with another placeholder or a path, or a tool whose manifest `required_outputs` none of its shown outputs seeds |
| `one_spec_file_per_tool_named_after_it` | a file name that isn't its `id`, a tool not in the manifest, a category with no menu label, a duplicate `id` |
| `every_offered_tool_is_dispatched` (`cmd_tools.rs`) | a tool here the CLI has no subcommand for |
| `every_tools_dialog_params_parse_as_its_cli_config` (`cmd_tools.rs`) | a dialog whose default params (seeded values, files filled, default columns) the CLI rejects |
| `derived_provenance_roles_match_the_golden_table` (`cmd_tools.rs`) | a `file_type` or field-name change that alters an input role the SSN tracing follows |

Run them with `cargo test -p domainatoredux-gui`. Where selected files land is tested by
`src/lib/toolInputs.test.ts`, and the column tokens by `src/lib/columns.test.ts`
(`npm test`).
