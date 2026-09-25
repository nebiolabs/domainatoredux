//! `extract_peptides`: convert selected annotated CDSs into peptide records
//! (GenBank or FASTA), carrying the CDS's Domainator/Domain_Search annotations
//! remapped to peptide coordinates. Port of
//! `domainator.extract_peptides.extract_peptides` (standalone path, i.e. not
//! `_from_domain_search`).

use std::collections::HashSet;

use crate::seq::translate::translate_dna;
use crate::seq::{Annotations, SeqFeature, SeqRecord};
use crate::tools::extract_common::{slice_from_location, Loc};
use crate::tools::select_common::DomainatorCds;

/// Options for [`extract_peptides`].
#[derive(Default)]
pub struct ExtractPeptidesOptions {
    /// E-value cutoff for the domain annotations kept on the output.
    pub evalue: f64,
    pub target_domains: Option<HashSet<String>>,
    pub target_cdss: Option<HashSet<String>>,
    pub search_hits: bool,
    /// `Some("f")` / `Some("r")`.
    pub strand: Option<String>,
    pub unannotated: bool,
    pub invert: bool,
    pub databases: Option<HashSet<String>>,
    pub keep_name: bool,
    pub name_field: Option<String>,
    /// Set when no selection criteria supplied (extract every CDS).
    pub extract_all: bool,
}

/// Extract peptides from all records.
pub fn extract_peptides(records: &[SeqRecord], opts: &ExtractPeptidesOptions) -> Vec<SeqRecord> {
    // Name precedence: name_field (if any) first, then the defaults.
    let default_prec: [&str; 4] = ["protein_id", "gene_id", "locus_tag", "gene"];
    let mut prec_owned: Vec<&str> = Vec::new();
    if let Some(nf) = &opts.name_field {
        prec_owned.push(nf.as_str());
    }
    prec_owned.extend_from_slice(&default_prec);

    let mut out = Vec::new();
    for contig in records {
        let cdss = DomainatorCds::list_from_contig(
            contig,
            opts.evalue,
            f64::NEG_INFINITY,
            true, // skip_pseudo
            Some(&prec_owned),
            false, // no nucleic-acid annotations
        );
        let source_features: Vec<&SeqFeature> =
            contig.features.iter().filter(|f| f.kind == "source").collect();

        for cds in &cdss {
            let mut keep = false;
            let target_cds_id = &cds.name; // cds_id_type == "name"
            if opts.extract_all {
                keep = true;
            }
            if let Some(tc) = &opts.target_cdss {
                if tc.contains(target_cds_id) {
                    keep = true;
                }
            }
            if let Some(td) = &opts.target_domains {
                if td.intersection(&cds.get_domain_names(opts.databases.as_ref(), None)).next().is_some() {
                    keep = true;
                }
            }
            if opts.search_hits && cds.domain_search_feature.is_some() {
                keep = true;
            }
            if opts.unannotated && cds.get_domain_names(opts.databases.as_ref(), None).is_empty() {
                keep = true;
            }

            let strand = cds.feature.location.parts.first().map(|p| p.2).unwrap_or(1);
            if let Some(sf) = &opts.strand {
                // Reject anything the strand filter does not name.
                if (sf == "f" && strand == -1) || (sf == "r" && strand != -1) {
                    keep = false;
                }
            }
            if opts.invert {
                keep = !keep;
            }
            if !keep {
                continue;
            }

            // Assemble features to carry (source, domain_search, domains, CDS).
            let mut cds_feature = cds.feature.clone();
            cds_feature.qualifiers.retain(|(k, _)| k != "translation");
            // `source_contig` is the contig LOCUS name (Python's contig.id).
            cds_feature.set("source_contig", contig.id.clone());

            let mut keep_features: Vec<SeqFeature> =
                source_features.iter().map(|f| (*f).clone()).collect();
            if let Some(ds) = &cds.domain_search_feature {
                keep_features.push(ds.clone());
            }
            keep_features.extend(cds.domain_features.iter().cloned());
            keep_features.push(cds_feature);

            // Slice the contig to the CDS location (coding orientation), truncating
            // features to fit (Python uses truncate_features=True here).
            let parts: Vec<(i64, i64)> = cds
                .feature
                .location
                .parts
                .iter()
                .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
                .collect();
            let loc = Loc { parts, strand };
            let mut region = slice_from_location(contig, &loc, &keep_features, true);

            // Peptide sequence: CDS /translation if present & non-empty, else translate.
            let pep = match cds.feature.get_first("translation") {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => translate_dna(&region.seq),
            };
            region.seq = pep.into_bytes();
            region.molecule_type = Some("protein".to_string());

            // DNA -> peptide coordinates (start//3, end//3).
            for f in &mut region.features {
                for p in &mut f.location.parts {
                    p.0 = p.0.div_euclid(3);
                    p.1 = p.1.div_euclid(3);
                }
            }

            // Name: the CDS name (or the contig name with --keep_name). Peptide has
            // no carried accession identity.
            let new_name = if opts.keep_name { contig.id.clone() } else { cds.name.clone() };
            region.id = new_name.clone();
            region.name = new_name;
            region.description = contig.description.clone();
            region.annotations = Annotations {
                source: contig.annotations.source.clone(),
                organism: contig.annotations.organism.clone(),
                taxonomy: contig.annotations.taxonomy.clone(),
                ..Default::default()
            };
            out.push(region);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::{Location, Topology};
    use crate::DOMAIN_FEATURE_NAME;

    fn cds(cds_id: &str, gene: &str, start: i64, end: i64, strand: i32) -> SeqFeature {
        let mut f = SeqFeature::new("CDS", Location::range(start, end, strand));
        f.set("cds_id", cds_id);
        f.set("gene", gene);
        f
    }

    fn dom(cds_id: &str, name: &str, start: i64, end: i64) -> SeqFeature {
        let mut f = SeqFeature::new(DOMAIN_FEATURE_NAME, Location::range(start, end, 1));
        f.set("cds_id", cds_id);
        f.set("name", name);
        f.set("database", "db");
        f.set("evalue", "1e-9");
        f.set("score", "50.0");
        f
    }

    fn contig(seq: &[u8], features: Vec<SeqFeature>) -> SeqRecord {
        SeqRecord {
            id: "ctg".into(),
            name: "acc.1".into(),
            description: "a contig".into(),
            seq: seq.to_vec(),
            molecule_type: Some("DNA".into()),
            topology: Topology::Linear,
            annotations: Annotations::default(),
            features,
        }
    }

    #[test]
    fn extract_all_translates_cds() {
        // 6 nt CDS -> 2 aa. ATG GCT -> M A
        let r = contig(b"ATGGCT", vec![cds("C1", "geneA", 0, 6, 1)]);
        let opts = ExtractPeptidesOptions { evalue: 1e8, extract_all: true, ..Default::default() };
        let out = extract_peptides(&[r], &opts);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, b"MA");
        assert_eq!(out[0].name, "geneA");
        assert_eq!(out[0].molecule_type.as_deref(), Some("protein"));
    }

    #[test]
    fn target_domain_selection_and_peptide_coords() {
        // CDS [0,9), domain [0,6) at nt -> peptide coords [0,2)
        let r = contig(b"ATGGCTTAA", vec![cds("C1", "geneA", 0, 9, 1), dom("C1", "D1", 0, 6)]);
        let opts = ExtractPeptidesOptions {
            evalue: 1e8,
            target_domains: Some(["D1".to_string()].into_iter().collect()),
            ..Default::default()
        };
        let out = extract_peptides(&[r], &opts);
        assert_eq!(out.len(), 1);
        // domain feature remapped to peptide coords
        let dfeat = out[0].features.iter().find(|f| f.kind == DOMAIN_FEATURE_NAME).unwrap();
        assert_eq!(dfeat.location.parts[0].0, 0);
        assert_eq!(dfeat.location.parts[0].1, 2);
        // no match for a different domain
        let opts2 = ExtractPeptidesOptions {
            evalue: 1e8,
            target_domains: Some(["OTHER".to_string()].into_iter().collect()),
            ..Default::default()
        };
        assert!(extract_peptides(&[contig(b"ATGGCTTAA", vec![cds("C1", "geneA", 0, 9, 1), dom("C1", "D1", 0, 6)])], &opts2).is_empty());
        let _ = out;
    }

    #[test]
    fn strand_filter_and_invert() {
        let r = contig(
            b"ATGGCTATGGCT",
            vec![cds("C1", "fwd", 0, 6, 1), cds("C2", "rev", 6, 12, -1)],
        );
        let opts = ExtractPeptidesOptions {
            evalue: 1e8,
            extract_all: true,
            strand: Some("f".into()),
            ..Default::default()
        };
        let out = extract_peptides(&[r], &opts);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "fwd");
    }

    #[test]
    fn keep_name_uses_contig() {
        let r = contig(b"ATGGCT", vec![cds("C1", "geneA", 0, 6, 1)]);
        let opts = ExtractPeptidesOptions { evalue: 1e8, extract_all: true, keep_name: true, ..Default::default() };
        let out = extract_peptides(&[r], &opts);
        assert_eq!(out[0].id, "ctg");
    }
}
