# GUI tool specs

Each `<id>.yml` here is the desktop app's whole description of one tool's dialog:
- its menu label and category
- the fields the dialog shows, and in what order
- which files each input offers
- what every option starts out as

`build.rs` embeds these files at compile time. `cmd_tools::list_tools` serves them to
`ToolDialog.svelte`. Nothing else in the app lists the tools the GUI offers. Editing a
file takes effect on the next build.

## Adding a tool or an option

- **New tool:** write `<id>.yml`. The file name must equal the tool's `id`, and the CLI
  must have a subcommand of that name. Nothing else is needed: a job runs
  `domainator <id> --config <the dialog's params>` in a worker process.
- **File fields drive provenance and Cancel.** An input's role in the job's provenance
  record comes from its `file_type` (a field named `reference`/`references` is always
  `reference`), and the SSN viewer's node → sequence tracing follows those roles. A
  canceled job deletes the files named by its `output_file` fields that it created or
  changed, so a field that names an output file should have that role.
- **New CLI option:** a new schema property makes
  `every_schema_property_is_listed_once` fail until the option is listed here, with
  `show: false` if the dialog should not offer it. That failure is how you decide about
  the option.

## Example

```yaml
id: domain-search
label: "Domain search"
category: "Annotation & search"
description: "Shown under the dialog title."
fields:
  - name: input
    label: "Input sequences"
    role: input_file
    file_type: [sequence, hmm]        # one value, or a list
    required: true
    sources: databases_first
    select: first
  - name: references
    label: "References (HMM/FASTA)"
    role: input_files
    file_type: reference
    required: true
    sources: databases_first
    select: "database:pfam"
  - name: output
    label: "Output (GenBank)"
    role: output_file
    required: true
    output_name: "{stem}.search.gb"
  - name: evalue
    label: "E-value"
    help: "Report hits at or below this E-value."
    cli_default: 0.001
    gui_default: 1.0e-5
  - name: batch_size
    show: false
    label: "Batch size"
    cli_default: 10000
```

## Tool keys

| Key | Required | What it does |
|---|---|---|
| `id` | yes | The tool's id, as in `domainator <id>` and `domainator schema <id>`. It must equal the file name without `.yml`. A job runs `domainator <id> --config …`. |
| `label` | yes | The name shown in the Tools menu, on the toolbar's Tools button, and as the dialog title. |
| `category` | yes | The Tools menu submenu. It must be one of `cmd_tools::TOOL_CATEGORIES`: `Annotation & search`, `Selection & extraction`, `Matrices & networks`, `Reports`, `HMM profiles`, `Color` or `Conversion`. Categories appear in that order. Within one, tools appear in file-name order. |
| `description` | yes | The sentence under the dialog title. |
| `fields` | yes | Every property of the tool's schema, once each. List order is dialog order. Hidden fields can go anywhere; the generated files put them last. |

## Field keys

| Key | Applies to | Default | What it does |
|---|---|---|---|
| `name` | all | (required) | The schema property. Use its spelling in `domainator schema <id>`, which is the underscore form (`max_overlap`, not `max-overlap`). |
| `label` | all | (required) | The text left of the control. |
| `role` | all | `param` | Which control the dialog draws, and how the value is sent. See [Roles](#roles). |
| `required` | all | `false` | Adds a red `*` to the label. On an input, removes the "(none)" entry. On a dropdown option (enum), removes the blank "(default)" entry unless the schema allows null. A required field cannot be hidden. A required output needs an `output_name`. The dialog doesn't block Run when a required field is blank. The tool refuses instead. |
| `show` | all | `true` | `false` keeps the option out of the dialog. It is never sent, so the CLI default applies. The one exception is `cpu`, which gets the Preferences thread count when that isn't 0. |
| `help` | shown fields | schema `description` | The hint under the control. See [Help text](#help-text). |
| `cli_default` | all | none | The schema's `default`, copied here. See [Defaults](#defaults). |
| `gui_default` | shown `param` / `columns` fields | none | What the dialog starts with instead of `cli_default`. |
| `file_type` | inputs | (required) | Which files and databases the dropdown offers. See [File types](#file-types). |
| `sources` | inputs | (required) | Which dropdown group comes first: `databases_first` or `workspace_first`. |
| `select` | inputs | (required) | The dropdown's starting entry. See [Input dropdowns](#input-dropdowns). |
| `output_name` | outputs | none | The file name the output starts with. See [Outputs](#outputs). |
| `column_types` | `columns` | none | Per-type labels, hints, parameter defaults and suggested values for the column builder. See [Column builder](#column-builder). |

## Roles

| `role` | Control | Sent to the tool as |
|---|---|---|
| `param` | Drawn from the property's schema type (below) | The typed value |
| `input_file` | A dropdown of databases and files | A string: `"input": "a.gb"` |
| `input_files` | The same dropdown, one choice | A one-item list: `"input": ["a.gb"]` |
| `output_file` | A file-name box | The name joined onto the current folder |
| `columns` | A report's column builder: an ordered list of column types and their parameters. See [Column builder](#column-builder). | A list of column objects, `{type, ...parameters}`, in list order |

`input_file` and `input_files` differ only in shape. The role has to match the type the
schema gives the property: `"type": ["string", "null"]` means `input_file`, and
`"type": "array"` means `input_files`. A mismatch fails at Run with
"bad params: invalid type". No field lets you choose several files yet.

A `param` is drawn by its schema type:

| Schema type | Control | Blank / unchecked |
|---|---|---|
| has an `enum` | a dropdown of the enum values, with a "(default)" entry when the schema allows null or the field isn't required | not sent |
| `boolean` | a checkbox | not sent (the CLI's `false`) |
| `integer`, `number` | a number box | not sent |
| `array` | a text box of comma-separated values; numbers are converted for numeric arrays | not sent |
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

`file_type` also drives the toolbar's Tools button. For the selected file, it offers each
tool with a shown input that accepts the file's type.

## Input dropdowns

An input lists two groups with a bar between them:
- **Databases:** everything in the Databases panel whose format matches.
- **Workspace:** files in the current folder whose type matches.

`sources` decides which group comes first. A path that is in both groups appears once, in
the first group. An empty group isn't shown. When both are empty, the dropdown is replaced
by "No … files/databases available".

`select` picks the starting entry. The "active file" is the active tab's file, which is
also the file the file table highlights.

| `select` | Starts on | When that isn't on offer |
|---|---|---|
| `active_file` | the active file, when it matches `file_type` | the first entry |
| `active_file_or_none` | the active file, when it matches `file_type` | blank |
| `database:<id>` | the managed reference download with that id: `pfam`, `swissprot`, `uniref50`, `uniref90`, `trembl` or `ncbi_taxonomy` (`reference_dbs::specs`) | the first entry |
| `first` | the first entry, which `sources` decides | blank, when nothing is on offer |
| `none` | blank ("(none)") | blank |

`none` suits optional inputs. A test checks that a `database:<id>` exists and that its
format can appear in the field's dropdown.

## Column builder

A `columns` field (enum-report's and hmmer-report's `columns`) is an ordered list. The
Enum Report viewer tab uses the same builder, with `enum-report.yml`'s `column_types`. Each
row is a column type plus that type's parameters, and the list order is the report's
column order. ↑/↓ reorder the rows.

**The types come from the schema, not from this file.** `domainator schema <id>` gives
`columns.items.oneOf`, one entry per column type. Each entry has:
- a `title`, shown in the type dropdown
- a `description`, shown as the row's hint
- its parameters with their types, e.g. `taxname` → `ranks`, a list of strings

`domainator-config` writes those entries next to the column enum the run deserializes,
and `column_schemas_match_their_enums` checks the two agree. So a new column type
appears in the builder without any change here.

**`column_types` adjusts how the builder shows them.** It is keyed by column type:

```yaml
    column_types:
      taxname:
        label: "Taxon name"        # replaces the schema's title in the dropdown
        help: "..."                # replaces the schema's description under the row
        show: false                # leave the type out of the dropdown
        params:
          ranks:
            label: "Ranks"         # the text beside the box (default: the parameter name)
            help: "..."            # tooltip and placeholder (default: the schema's description)
            default: [species]     # what a newly added column starts with
            choices: [self, domain, superkingdom, phylum, genus, species]
```

Every key is optional. How each parameter is drawn depends on its schema type:

| Parameter | Box | `choices` |
|---|---|---|
| list (`"type": "array"`) | comma-separated text, sent as an array | one-click chips that add or remove a value |
| enum | a dropdown of the enum values | not used |
| number | a text box, sent as a number | not used |
| string | a text box | a suggestion list |

Values outside `choices` can always be typed. A new row starts each parameter at its
`default`. Without a default, an enum starts on its first value and anything else starts
blank. A required list left empty stops the run before it starts, naming the column.
Changing a row's type keeps the values of parameters the two types share.

`gui_default` for a `columns` field is the list of column objects the builder starts with,
e.g. `[{type: domains}, {type: taxname, ranks: [genus]}]`. Each object must be valid for
its type (`gui_defaults_fit_their_property`) and must not use a type `column_types` hides.

## Outputs

An `output_file` is a file-name box. The app writes the file into the current folder, the
one named at the top of the dialog. `output_name` sets the starting name:
- `{stem}` is replaced with the primary input's name, up to its first `.`. For
  `/data/run1.search.gb`, the stem is `run1`.
- The primary input is the field named `input` when it has a value, otherwise the first
  input with one.
- With no input chosen, `{stem}` becomes `output`.
- `output_name` must be a file name, not a path. It can use no placeholder other than
  `{stem}`.

An output without `output_name` starts blank. A blank output isn't sent, so that file isn't
written. Every required output needs an `output_name`.

## Defaults

- **`cli_default`** repeats the schema's `default` and must equal it
  (`cli_defaults_match_the_schema`). Numbers compare by value, so `1000` and `1000.0` are
  equal. It is present exactly when the schema declares a `default`. It is there so this
  file shows what the command line does. To change the command line's default, edit the
  schema in `crates/domainator-config`, not this file.
- **`gui_default`** is what the dialog starts with instead:
  - It must fit the property's type, and its enum if it has one.
  - It is allowed only on shown `param` and `columns` fields. Files
    are chosen with `select` and `output_name`.
  - An array is written as a YAML list (`[a, b]`) and shows in the box as `a,b`.
  - When `gui_default` and `cli_default` differ, the dialog says so under the field.
- **Clearing a field** sends nothing, and the run falls back to the CLI default: the
  params go to the CLI itself.
- **A cloned job** (the Jobs panel's ⎘ button, or "Clone into Tool Dialog") starts from
  that job's saved values. Those replace
  these defaults for every field the job set.

## Help text

The hint under a field is the schema property's `description`, the same text
`domainator schema <id>` gives agents. Those descriptions are written in
`crates/domainator-config/src/lib.rs`: in each tool's schema function, with gaps filled
from the shared `common_help` list by `fill_common_descriptions`.

- Setting `help:` replaces the hint in the dialog only. Leave the key out to keep the
  schema's text.
- A `help` on a hidden field, or an empty one, is a test failure.
- On a `columns` field, `help` is the hint under the whole builder; the schema's
  description there is written for agents, so the builder shows none by default. Each
  row's own hint comes from its column type (see [Column builder](#column-builder)).

## What checks these files

| Check | Catches |
|---|---|
| `build.rs` (the build fails) | invalid YAML, a misspelled or unknown key, a bad `select` value |
| `every_schema_property_is_listed_once` | a schema property missing here, a field that isn't a schema property, a field listed twice |
| `cli_defaults_match_the_schema` | a `cli_default` that is stale, missing, or has no schema default behind it |
| `gui_defaults_fit_their_property` | a `gui_default` of the wrong type, outside its enum, on a hidden field, or on a file field; for `columns`, a column object its type rejects or a hidden type |
| `input_fields_declare_sources_and_select` | an input missing `file_type`/`sources`/`select`; an unknown `file_type`; an unknown or unfitting `database:<id>`; those keys on a non-input; a hidden required field; a hidden or empty `help` |
| `column_type_overrides_name_real_types_and_parameters` | a `column_types` type or parameter the schema lacks; a `default` or `choice` that doesn't fit; `column_types` on a non-`columns` field; every type hidden |
| `column_schemas_match_their_enums` (`domainator-config`) | a column type or parameter in the schema that the column enum lacks, or the reverse |
| `required_outputs_have_a_name_template` | a required output with no `output_name`, or one with another placeholder or a path |
| `one_spec_file_per_tool_named_after_it` | a file name that isn't its `id`, an unknown `category`, a duplicate `id` |
| `every_offered_tool_is_dispatched_and_has_a_schema` (`cmd_tools.rs`) | a tool here the CLI has no subcommand for, or a shown field that is not a schema property |
| `every_tools_dialog_params_parse_as_its_cli_config` (`cmd_tools.rs`) | a dialog whose default params (seeded values, files filled, default columns) the CLI rejects |
| `derived_provenance_roles_match_the_golden_table` (`cmd_tools.rs`) | a `file_type` or field-name change that alters an input role the SSN tracing follows |

Run them with `cargo test -p domainator-gui`.
