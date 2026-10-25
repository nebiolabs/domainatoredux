use anyhow::{anyhow, Context, Result};
use domainator_config::args::DomainSearchArgs;
use domainator_core::tools::domain_search::{self, input_molecule_types, DomainSearchOptions};
use domainator_core::tools::domainate::read_hmm_references;
use domainator_core::tools::neighborhood::{validate_range, Range};
use domainator_core::tools::select_common::parse_name_list;

use super::{cpu_threads, usage_error};
use crate::output::{write_seq_outputs_capped, OutputCap};

pub fn run(args: &DomainSearchArgs) -> Result<()> {
    let cpu = cpu_threads(args.cpu)?;
    if args.evalue.0 > 10.0 {
        return Err(usage_error("-e/--evalue must be <= 10."));
    }
    if args.z == 0 && args.gene_call.is_some() {
        return Err(usage_error(
            "-Z 0 can't be used with --gene_call: give the database size explicitly.",
        ));
    }
    let batch_size = u64::try_from(args.batch_size)
        .ok()
        .filter(|&n| n > 0)
        .context("batch_size must be > 0")?;
    let max_bytes = crate::max_output_gb_to_bytes(args.max_output_gb.0)?;
    let max_hits = match args.max_hits {
        Some(n) if n < 1 => {
            return Err(usage_error(
                "--max_hits must be at least 1. Leave it out to return all hits.",
            ))
        }
        n => n.and_then(|n| usize::try_from(n).ok()),
    };
    let range = validate_range(
        args.whole_contig,
        args.kb_range.map(|v| v.0),
        args.kb_up.map(|v| v.0),
        args.kb_down.map(|v| v.0),
        args.cds_range,
        args.cds_up,
        args.cds_down,
    )
    .map_err(|e| anyhow!(e))?;

    let molecule_types = input_molecule_types(&args.input, &args.alphabet)?;
    let dbs = read_hmm_references(&args.reference).context("reading references")?;
    let protein = molecule_types.contains("protein");
    if protein && dbs.has_nucleotide() {
        anyhow::bail!("Cannot search protein inputs with a nucleotide reference.");
    }
    if molecule_types.len() > 1 && protein && !args.translate {
        anyhow::bail!(
            "Inputs are a combination of DNA and protein sequences, but --translate is not \
             set. This would lead to an output genbank file with mixed protein and DNA \
             sequences."
        );
    }
    if (protein || args.translate) && (!matches!(range, Range::None) || args.pad) {
        anyhow::bail!(
            "Protein sequence output is not compatible with neighborhood extraction or padding."
        );
    }
    if args.both_strands {
        if !args.deduplicate_by_sequence {
            return Err(usage_error(
                "--both_strands requires --deduplicate_by_sequence.",
            ));
        }
        if protein || args.translate {
            return Err(usage_error(
                "--both_strands is only supported for nucleotide output.",
            ));
        }
    }

    let allowed_taxids = crate::compile_allowed_taxids(
        args.include_taxids.as_deref().unwrap_or_default(),
        args.exclude_taxids.as_deref().unwrap_or_default(),
        args.taxonomy_expr.as_deref(),
        args.ncbi_taxonomy_path.as_deref(),
    )?;
    let decoys = parse_name_list(
        args.decoys.as_deref().unwrap_or_default(),
        args.decoys_file.as_deref(),
    )?;

    let opts = DomainSearchOptions {
        z: args.z as f64,
        evalue: args.evalue.0,
        min_evalue: args.min_evalue.0,
        max_overlap: args.max_overlap.0,
        add_annotations: args.add_annotations,
        max_hits,
        batch_size,
        alphabet: args.alphabet.clone(),
        cpu,
        range,
        normalize_direction: !args.keep_direction,
        strand: match args.strand.as_deref() {
            Some("f") => Some(1),
            Some("r") => Some(-1),
            _ => None,
        },
        max_region_overlap: args.max_region_overlap.0,
        translate: args.translate,
        max_mode: args.max_mode,
        allowed_taxids,
        decoys,
        max_hits_per_contig: args
            .max_hits_per_contig
            .and_then(|n| usize::try_from(n).ok()),
        pad: args.pad,
        anchor: args.anchor.parse().map_err(|e: String| anyhow!(e))?,
        deduplicate: args.deduplicate && !args.no_deduplicate,
        deduplicate_by_sequence: args.deduplicate_by_sequence,
        both_strands: args.both_strands,
        partial: args.partial.parse().map_err(|e: String| anyhow!(e))?,
        gene_call: crate::gene_call_request(args.gene_call.as_deref(), cpu)?,
    };
    let hits =
        domain_search::domain_search(&args.input, &dbs, &opts).context("running domain_search")?;
    write_seq_outputs_capped(
        &hits,
        args.genbank_out.as_deref(),
        args.fasta_out.as_deref(),
        max_bytes.map(|max_bytes| OutputCap {
            max_bytes,
            mitigation: &[
                "--max_hits",
                "--max_hits_per_contig",
                "--cds_range",
                "--kb_range",
            ],
            extra_guidance: args.whole_contig.then_some(
                "Avoid --whole_contig on broad searches unless you expect very few hits.",
            ),
        }),
    )
}
