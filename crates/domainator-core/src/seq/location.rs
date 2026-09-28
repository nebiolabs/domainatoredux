//! Feature locations, modeled as the Python suite's "lean" parts.
//!
//! Coordinates are 0-based half-open. A `Part` is `(start, end, strand, before,
//! after)` where `before`/`after` carry the GenBank `<`/`>` fuzzy markers. For
//! complement joins, parts are pre-reversed so part order matches Biopython's,
//! which keeps hit coordinates and `cds_id` generation consistent (lifted from the
//! prototype's `parser.rs::lean_location`).

/// A single span: `(start, end, strand, before, after)`, 0-based half-open.
pub type Part = (i64, i64, i32, bool, bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    Join,
    Order,
}

impl CompoundOp {
    pub fn as_str(self) -> &'static str {
        match self {
            CompoundOp::Join => "join",
            CompoundOp::Order => "order",
        }
    }
}

/// A feature location. `Compound` holds the flattened, strand-resolved parts plus
/// the operator; `between` marks the GenBank `n^m` between-bases form.
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub operator: Option<CompoundOp>,
    pub between: bool,
    pub parts: Vec<Part>,
}

impl Location {
    /// Single ungapped span.
    pub fn range(start: i64, end: i64, strand: i32) -> Self {
        Location {
            operator: None,
            between: false,
            parts: vec![(start, end, strand, false, false)],
        }
    }

    /// Overall start = min part start. Returns 0 for an empty location.
    pub fn start(&self) -> i64 {
        self.parts.iter().map(|p| p.0.min(p.1)).min().unwrap_or(0)
    }

    /// Overall end = max part end.
    pub fn end(&self) -> i64 {
        self.parts.iter().map(|p| p.0.max(p.1)).max().unwrap_or(0)
    }

    /// Net strand: the strand of the first part (parts are pre-reversed for
    /// complement joins, so the first part reflects the feature's reading order).
    pub fn strand(&self) -> i32 {
        self.parts.first().map(|p| p.2).unwrap_or(1)
    }

    /// Flatten to normalized `(start, end)` half-open intervals (start <= end),
    /// for coverage tests.
    pub fn intervals(&self) -> Vec<(i64, i64)> {
        self.parts
            .iter()
            .map(|&(s, e, ..)| if s <= e { (s, e) } else { (e, s) })
            .collect()
    }
}

/// Split `s` on top-level commas, respecting nested parentheses.
fn split_top_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Parse a single GenBank span like `<123..>456`, `123`, or `123^124` into a
/// `Part` (0-based half-open), with the given `strand`.
fn parse_span(s: &str, strand: i32) -> Option<Part> {
    let s = s.trim();
    if let Some((a, b)) = s.split_once("..") {
        let (before, a) = match a.strip_prefix('<') {
            Some(rest) => (true, rest),
            None => (false, a),
        };
        let (after, b) = match b.strip_prefix('>') {
            Some(rest) => (true, rest),
            None => (false, b),
        };
        let start: i64 = a.trim().parse().ok()?;
        let end: i64 = b.trim().parse().ok()?;
        Some((start - 1, end, strand, before, after))
    } else if let Some((a, _b)) = s.split_once('^') {
        let pos: i64 = a.trim().parse().ok()?;
        Some((pos, pos, strand, false, false))
    } else {
        let (before, rest) = match s.strip_prefix('<') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (after, rest) = match rest.strip_prefix('>') {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let pos: i64 = rest.trim().parse().ok()?;
        Some((pos - 1, pos, strand, before, after))
    }
}

fn parse_loc(s: &str, strand: i32) -> Option<Location> {
    let s = s.trim();
    if let Some(inner) = s
        .strip_prefix("complement(")
        .and_then(|x| x.strip_suffix(')'))
    {
        return parse_loc(inner, -strand);
    }
    for (kw, op) in [("join(", CompoundOp::Join), ("order(", CompoundOp::Order)] {
        if let Some(inner) = s.strip_prefix(kw).and_then(|x| x.strip_suffix(')')) {
            let mut groups: Vec<Vec<Part>> = Vec::new();
            for sub in split_top_commas(inner) {
                groups.push(parse_loc(sub, strand)?.parts);
            }
            if strand == -1 {
                groups.reverse();
            }
            let between = false;
            return Some(Location {
                operator: Some(op),
                between,
                parts: groups.into_iter().flatten().collect(),
            });
        }
    }
    let between = s.contains('^');
    let part = parse_span(s, strand)?;
    Some(Location {
        operator: None,
        between,
        parts: vec![part],
    })
}

/// A 0-based coordinate as a slice or list index.
///
/// For coordinates the caller has already clamped or proven non-negative. Parsed
/// locations can be negative (the parser accepts `0..9`), so a negative value here
/// means a missed check upstream; it is reported as that, rather than becoming an
/// index near `usize::MAX` that fails somewhere less obvious.
pub fn to_index(pos: i64) -> usize {
    usize::try_from(pos).unwrap_or_else(|_| panic!("negative coordinate {pos} used as an index"))
}

/// Parse a GenBank location string (e.g. `complement(join(1..2,4..5))`) into our
/// lean model. Falls back to an empty single-point location on parse failure
/// (the reader never rejects a record over a location it can't read — plan E4).
pub fn parse_location(s: &str) -> Location {
    parse_loc(s, 1).unwrap_or(Location {
        operator: None,
        between: false,
        parts: vec![(0, 0, 1, false, false)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_span_is_zero_based_half_open() {
        let l = parse_location("100..200");
        assert_eq!(l.parts, vec![(99, 200, 1, false, false)]);
        assert_eq!(l.start(), 99);
        assert_eq!(l.end(), 200);
        assert_eq!(l.strand(), 1);
    }

    #[test]
    fn complement_flips_strand() {
        let l = parse_location("complement(100..200)");
        assert_eq!(l.strand(), -1);
        assert_eq!((l.start(), l.end()), (99, 200));
    }

    #[test]
    fn join_keeps_parts_and_operator() {
        let l = parse_location("join(1..3,7..9)");
        assert_eq!(l.operator, Some(CompoundOp::Join));
        assert_eq!(l.start(), 0);
        assert_eq!(l.end(), 9);
        assert_eq!(l.strand(), 1);
        assert_eq!(l.parts.len(), 2);
    }

    #[test]
    fn complement_join_is_prereversed() {
        // Reading order: the higher span comes first on the reverse strand.
        let l = parse_location("complement(join(1..3,7..9))");
        assert_eq!(l.strand(), -1);
        assert_eq!((l.start(), l.end()), (0, 9));
        assert_eq!(l.parts.first().map(|p| (p.0, p.1)), Some((6, 9)));
    }

    #[test]
    fn fuzzy_markers_parse() {
        let l = parse_location("<1..>9");
        let p = l.parts[0];
        assert_eq!((p.0, p.1, p.3, p.4), (0, 9, true, true));
    }
}
