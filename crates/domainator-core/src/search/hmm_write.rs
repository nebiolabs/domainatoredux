//! HMMER3 ASCII profile writer.
//!
//! `hmmer_pure_rs::hmmfile::write_hmm` emits a profile that HMMER can read, but
//! not the layout `hmmbuild`/pyhmmer produce: it announces the 2010 format
//! revision, lays the model matrix out one column narrower than HMMER does, and
//! — the part that actually costs something — **drops the `GA`/`TC`/`NC` Pfam
//! score cutoffs entirely**, so `hmmsearch --cut_ga` against a profile that went
//! through it cannot work. The data survives the read (`Hmm::cutoff` and the
//! `P7H_GA`/`TC`/`NC` flags are populated); only the writer loses it.
//!
//! This is `p7_hmmfile_WriteASCII`'s layout, field for field. The gate is
//! `hmmer-select`, which only *selects* whole profiles and so must reproduce its
//! input byte for byte; upstream's does, because pyhmmer round-trips exactly.
//!
//! Layout, from the fixtures (`test/data/pdonr_hmms.hmm` and friends):
//!
//! ```text
//! STATS LOCAL MSV      -10.4088  0.70362      label %-7s, then two %9s fields
//!   COMPO   2.42172  4.56629 ...              8-char prefix, then %9s per value
//!       1   2.28885  4.32525 ...      1 e - - X
//!           ^ %7d + space                     ^ MAP %6d, then CONS/RF/MM/CS
//! ```
//!
//! Every model-matrix value occupies nine columns: a seven-character number (or
//! a right-justified `*` for zero probability) preceded by two spaces.

use std::io::{self, Write};

use hmmer_pure_rs::hmm::{
    Hmm, NTRANSITIONS, P7H_CHKSUM, P7H_COMPO, P7H_CONS, P7H_CS, P7H_GA, P7H_MAP, P7H_MMASK,
    P7H_NC, P7H_RF, P7H_STATS, P7H_TC, P7_FLAMBDA, P7_FTAU, P7_GA1, P7_GA2, P7_MLAMBDA, P7_MMU,
    P7_NC1, P7_NC2, P7_TC1, P7_TC2, P7_VLAMBDA, P7_VMU,
};
use hmmer_pure_rs::alphabet::{Alphabet, AlphabetType};

/// The format revision and writing-program stamp on line 1.
///
/// `HMMER3/f` is the *file format* version; the bracketed half names the program
/// that wrote the file. Upstream writes profiles through pyhmmer, which stamps
/// HMMER 3.4 — and since `hmmer-select` must reproduce its input byte for byte,
/// matching that string is the whole point. The layout below is that format.
const MAGIC: &str = "HMMER3/f [3.4 | Aug 2023]";

/// One model-matrix value: seven characters, `*` when the probability is zero.
///
/// HMMER stores `-ln(p)`, computed in **single** precision — `logf`, not `log` —
/// so the last digit matches only if the log is taken on an `f32`.
fn prob(p: f32) -> String {
    if p <= 0.0 {
        "      *".to_string()
    } else if p == 1.0 {
        // `-ln(1.0)` is `-0.0`, which would format as an eight-character
        // "-0.00000" and shift the rest of the line.
        format!("{:7.5}", 0.0)
    } else {
        format!("{:7.5}", -p.ln())
    }
}

/// Write one nine-column model-matrix field.
fn field<W: Write>(w: &mut W, p: f32) -> io::Result<()> {
    write!(w, "{:>9}", prob(p))
}

fn yes_no(flags: u32, bit: u32) -> &'static str {
    if flags & bit != 0 {
        "yes"
    } else {
        "no"
    }
}

/// Write `hmm` as an HMMER3 ASCII profile.
pub fn write_hmm<W: Write>(w: &mut W, hmm: &Hmm) -> io::Result<()> {
    let k = hmm.abc_k;

    writeln!(w, "{MAGIC}")?;
    writeln!(w, "NAME  {}", hmm.name)?;
    if let Some(acc) = &hmm.acc {
        writeln!(w, "ACC   {acc}")?;
    }
    if let Some(desc) = &hmm.desc {
        writeln!(w, "DESC  {desc}")?;
    }
    writeln!(w, "LENG  {}", hmm.m)?;
    if hmm.max_length >= 0 {
        writeln!(w, "MAXL  {}", hmm.max_length)?;
    }
    writeln!(
        w,
        "ALPH  {}",
        match hmm.abc_type {
            AlphabetType::Amino => "amino",
            AlphabetType::Dna => "DNA",
            AlphabetType::Rna => "RNA",
            AlphabetType::Unknown => "unknown",
        }
    )?;
    writeln!(w, "RF    {}", yes_no(hmm.flags, P7H_RF))?;
    writeln!(w, "MM    {}", yes_no(hmm.flags, P7H_MMASK))?;
    writeln!(w, "CONS  {}", yes_no(hmm.flags, P7H_CONS))?;
    writeln!(w, "CS    {}", yes_no(hmm.flags, P7H_CS))?;
    writeln!(w, "MAP   {}", yes_no(hmm.flags, P7H_MAP))?;
    if let Some(ctime) = &hmm.ctime {
        writeln!(w, "DATE  {ctime}")?;
    }
    if let Some(comlog) = &hmm.comlog {
        // The reader joins multiple `COM   [n] ...` lines with newlines and
        // strips their numbering; re-number them on the way out.
        for (i, cmd) in comlog.lines().enumerate() {
            writeln!(w, "COM   [{}] {}", i + 1, cmd)?;
        }
    }
    if hmm.nseq >= 0 {
        writeln!(w, "NSEQ  {}", hmm.nseq)?;
    }
    if hmm.eff_nseq >= 0.0 {
        writeln!(w, "EFFN  {:.6}", hmm.eff_nseq)?;
    }
    if hmm.flags & P7H_CHKSUM != 0 {
        writeln!(w, "CKSUM {}", hmm.checksum)?;
    }
    // The Pfam score cutoffs, between CKSUM and STATS. Losing these is what
    // breaks `hmmsearch --cut_ga` downstream.
    for (bit, label, lo, hi) in [
        (P7H_GA, "GA", P7_GA1, P7_GA2),
        (P7H_TC, "TC", P7_TC1, P7_TC2),
        (P7H_NC, "NC", P7_NC1, P7_NC2),
    ] {
        if hmm.flags & bit != 0 {
            writeln!(w, "{label}    {:.2} {:.2}", hmm.cutoff[lo], hmm.cutoff[hi])?;
        }
    }
    if hmm.flags & P7H_STATS != 0 {
        for (label, mu, lambda) in [
            ("MSV", P7_MMU, P7_MLAMBDA),
            ("VITERBI", P7_VMU, P7_VLAMBDA),
            ("FORWARD", P7_FTAU, P7_FLAMBDA),
        ] {
            writeln!(
                w,
                "STATS LOCAL {label:<7} {:>9.4}{:>9.5}",
                hmm.evparam[mu], hmm.evparam[lambda]
            )?;
        }
    }

    // Alphabet header: symbols sit in the same nine-column grid as the values,
    // and the line keeps its three trailing spaces.
    let abc = Alphabet::new(hmm.abc_type);
    write!(w, "HMM     ")?;
    for sym in abc.sym.iter().take(k) {
        write!(w, "     {}   ", *sym as char)?;
    }
    writeln!(w)?;
    writeln!(
        w,
        "            m->m     m->i     m->d     i->m     i->i     d->m     d->d"
    )?;

    if hmm.flags & P7H_COMPO != 0 {
        write!(w, "  COMPO ")?;
        for i in 0..k {
            field(w, hmm.compo[i])?;
        }
        writeln!(w)?;
    }

    // Node 0 carries the begin transitions and the insert state's emissions.
    write!(w, "        ")?;
    for i in 0..k {
        field(w, hmm.ins[0][i])?;
    }
    writeln!(w)?;
    write!(w, "        ")?;
    for i in 0..NTRANSITIONS {
        field(w, hmm.t[0][i])?;
    }
    writeln!(w)?;

    for node in 1..=hmm.m {
        write!(w, "{node:>7} ")?;
        for i in 0..k {
            field(w, hmm.mat[node][i])?;
        }
        // The annotation columns are all-or-nothing: when MAP is present HMMER
        // writes every column, using `-` for the ones this profile lacks.
        if let Some(map) = &hmm.map {
            let at = |v: &Option<Vec<u8>>| v.as_ref().map_or('-', |s| s[node] as char);
            write!(
                w,
                " {:>6} {} {} {} {}",
                map[node],
                at(&hmm.consensus),
                at(&hmm.rf),
                at(&hmm.mm),
                at(&hmm.cs)
            )?;
        } else {
            for chars in [&hmm.consensus, &hmm.rf, &hmm.cs].into_iter().flatten() {
                write!(w, " {}", chars[node] as char)?;
            }
        }
        writeln!(w)?;

        write!(w, "        ")?;
        for i in 0..k {
            field(w, hmm.ins[node][i])?;
        }
        writeln!(w)?;
        write!(w, "        ")?;
        for i in 0..NTRANSITIONS {
            field(w, hmm.t[node][i])?;
        }
        writeln!(w)?;
    }

    writeln!(w, "//")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmmer_pure_rs::hmm::CUTOFF_UNSET;

    /// A two-node DNA model, enough to exercise every line kind.
    fn model() -> Hmm {
        let mut hmm = Hmm::new(2, AlphabetType::Dna, 4);
        hmm.name = "tiny".into();
        hmm.nseq = -1;
        hmm.eff_nseq = -1.0;
        for node in 0..=2 {
            hmm.mat[node] = vec![0.25, 0.25, 0.25, 0.25];
            hmm.ins[node] = vec![0.25, 0.25, 0.25, 0.25];
            hmm.t[node] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        }
        hmm
    }

    fn render(hmm: &Hmm) -> Vec<String> {
        let mut out = Vec::new();
        write_hmm(&mut out, hmm).unwrap();
        String::from_utf8(out).unwrap().lines().map(str::to_string).collect()
    }

    /// Column positions read off `test/data/pdonr_hmms.hmm`, which `hmmbuild`
    /// wrote: every model value occupies nine columns and the first one ends at
    /// column 17. Getting this one column narrow is what made `hmmer-select`
    /// rewrite its input instead of reproducing it.
    #[test]
    fn model_values_sit_on_a_nine_column_grid() {
        let lines = render(&model());
        let match_line = lines.iter().find(|l| l.starts_with("      1 ")).unwrap();
        let ins_line = lines
            .iter()
            .find(|l| l.starts_with("          ") && l.contains('.') && !l.contains("->"))
            .unwrap();
        for line in [match_line, ins_line] {
            // Where each emission value starts. The node index on a match line is
            // not one of them, so key on the decimal point.
            let starts: Vec<usize> = line
                .match_indices('.')
                .map(|(i, _)| i - 1)
                .collect();
            // First value at 10, then every nine columns.
            assert_eq!(starts, vec![10, 19, 28, 37], "line: {line:?}");
        }
        // A zero probability is a right-justified `*` in the same nine columns, so
        // field n ends at 8 + 9(n+1): fields 1, 2, 4 and 6 of this model are zero.
        let trans = lines.iter().find(|l| l.contains('*')).unwrap();
        let stars: Vec<usize> = trans.match_indices('*').map(|(i, _)| i).collect();
        assert_eq!(stars, vec![25, 34, 52, 70], "line: {trans:?}");
        assert_eq!(trans.len(), 8 + 7 * 9);
    }

    #[test]
    fn the_pfam_cutoffs_are_written() {
        let mut hmm = model();
        hmm.cutoff = [28.60, 28.60, 28.60, 28.80, 28.50, 28.40];
        hmm.flags |= P7H_GA | P7H_TC | P7H_NC;
        let lines = render(&hmm);
        assert!(lines.contains(&"GA    28.60 28.60".to_string()), "{lines:?}");
        assert!(lines.contains(&"TC    28.60 28.80".to_string()), "{lines:?}");
        assert!(lines.contains(&"NC    28.50 28.40".to_string()), "{lines:?}");
    }

    #[test]
    fn a_model_without_cutoffs_writes_none() {
        let mut hmm = model();
        hmm.cutoff = [CUTOFF_UNSET; 6];
        let lines = render(&hmm);
        assert!(!lines.iter().any(|l| l.starts_with("GA ") || l.starts_with("TC ")));
    }

    /// `STATS LOCAL <label> <mu> <lambda>`: the label is left-justified in seven
    /// columns and both numbers are right-justified, so mu always ends at column
    /// 29 whether it is `-10.4088` or `-5.1274`.
    #[test]
    fn stats_lines_are_column_aligned() {
        let mut hmm = model();
        hmm.flags |= P7H_STATS;
        hmm.evparam = [-10.4088, 0.70362, -11.2734, 0.70362, -5.1274, 0.70362];
        let lines = render(&hmm);
        let stats: Vec<&String> = lines.iter().filter(|l| l.starts_with("STATS")).collect();
        assert_eq!(stats.len(), 3);
        for line in &stats {
            assert_eq!(line.len(), 38, "line: {line:?}");
        }
        assert_eq!(stats[0].as_str(), "STATS LOCAL MSV      -10.4088  0.70362");
        assert_eq!(stats[2].as_str(), "STATS LOCAL FORWARD   -5.1274  0.70362");
    }

    #[test]
    fn multiple_com_lines_are_renumbered() {
        let mut hmm = model();
        hmm.comlog = Some("first --run\nsecond --run".into());
        let lines = render(&hmm);
        assert!(lines.contains(&"COM   [1] first --run".to_string()), "{lines:?}");
        assert!(lines.contains(&"COM   [2] second --run".to_string()), "{lines:?}");
    }
}
