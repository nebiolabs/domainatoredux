//! Categorical color assignment, mirroring `domainator.utils`'s palette helpers.
//!
//! Shared by every tool that colors by a metadata value (`build_ssn`,
//! `color_genbank`, the contig viewer), so a value gets the same color whichever
//! tool drew it.
//!
//! Upstream replaced seaborn's chained `tab20`/`tab20b`/`tab20c` with the hand-built
//! [`DISTINCT_COLORS`] table (`fbd6927`), which changes the colors of existing
//! outputs; the updated `ssn_FeSOD*.xgmml` fixtures encode the new assignment, so it
//! is the parity contract rather than a preference.

/// 64 distinguishable colors, greedy-maximin selected in CIELAB and seeded with
/// matplotlib's `tab10` (minimum pairwise ΔE 23.3). Regenerated upstream by
/// `scripts/generate_distinct_palette.py`; hex is **uppercase**, which the fixtures
/// check.
pub const DISTINCT_COLORS: [&str; 64] = [
    "#1F77B4", "#FF7F0E", "#2CA02C", "#D62728", "#9467BD", "#8C564B", "#E377C2", "#7F7F7F",
    "#BCBD22", "#17BECF", "#0000FF", "#FF1AFF", "#00F200", "#0026B3", "#1A4D1A", "#FFBF80",
    "#FF0080", "#00E6B3", "#730D4D", "#806600", "#99B373", "#A60DA6", "#FF808C", "#0080FF",
    "#BFBFFF", "#99E626", "#9940FF", "#00F280", "#80001A", "#A64D0D", "#004D4D", "#4D4066",
    "#008C73", "#FFBF00", "#1A338C", "#FFBFD9", "#FF73FF", "#99E680", "#A673FF", "#4D7300",
    "#FF00BF", "#FF0000", "#33BFFF", "#A67399", "#BF1A80", "#99D9BF", "#4D401A", "#CCB399",
    "#FF8059", "#CC8C1A", "#A6D9F2", "#BF0D4D", "#1ABF00", "#661A80", "#D98C73", "#4DB373",
    "#008099", "#004DFF", "#B34D66", "#4D4040", "#66A6FF", "#8C804D", "#CCD973", "#FF1A59",
];

/// Neutral gray for values with no color of their own: missing data, and the groups
/// beyond `max_groups`.
pub const OTHER_COLOR: &str = "#BFBFBF";

/// A value as it reaches the palette: present (a string) or missing.
///
/// Values arrive already rendered as strings by the callers (a metadata cell, a
/// domain name), so numeric-looking ones are re-parsed here rather than carried as a
/// separate type.
fn as_number(value: &str) -> Option<f64> {
    let n: f64 = value.trim().parse().ok()?;
    n.is_finite().then_some(n)
}

/// Deterministically order values for color assignment.
///
/// Numeric and numeric-looking values (cluster ids, or the strings read back out of a
/// color table) sort numerically; everything else sorts lexically by its string form.
/// Unlike sorting the raw values this is safe when a column mixes types, and it is why
/// numerically adjacent values always get different colors.
pub fn sort_palette_values(values: &[String]) -> Vec<String> {
    let mut numeric: Vec<(f64, &String)> = Vec::new();
    let mut other: Vec<&String> = Vec::new();
    for value in values {
        match as_number(value) {
            Some(n) => numeric.push((n, value)),
            None => other.push(value),
        }
    }
    numeric.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
    other.sort();
    numeric
        .into_iter()
        .map(|(_, v)| v)
        .chain(other)
        .cloned()
        .collect()
}

/// Map values to distinct colors.
///
/// Colors are assigned in [`sort_palette_values`] order and cycle over
/// [`DISTINCT_COLORS`], so numerically adjacent values always differ. Values 64 apart
/// do reuse a color; `max_groups` exists for when that is still too many to read.
///
/// `counts`, when given, ranks groups by frequency for `max_groups` (Python passes a
/// pandas Series to get this); without it, groups are ranked in sort order. Groups
/// beyond the cap share [`OTHER_COLOR`].
pub fn get_palette(
    values: &[String],
    max_groups: Option<usize>,
    counts: Option<&std::collections::HashMap<String, usize>>,
) -> std::collections::HashMap<String, String> {
    use std::collections::{HashMap, HashSet};

    // Unique, preserving nothing: order comes from sort_palette_values below.
    let mut seen = HashSet::new();
    let present: Vec<String> = values
        .iter()
        .filter(|v| seen.insert((*v).clone()))
        .cloned()
        .collect();

    let keep: Option<HashSet<String>> = match max_groups {
        Some(max) if max < present.len() => {
            let mut ranked: Vec<String> = match counts {
                // Most common first; ties by value so the choice is deterministic.
                Some(c) => {
                    let mut r = present.clone();
                    r.sort_by(|a, b| {
                        c.get(b)
                            .unwrap_or(&0)
                            .cmp(c.get(a).unwrap_or(&0))
                            .then(a.cmp(b))
                    });
                    r
                }
                None => sort_palette_values(&present),
            };
            ranked.truncate(max);
            Some(ranked.into_iter().collect())
        }
        _ => None,
    };

    let mut out = HashMap::new();
    let mut colored = 0usize;
    for value in sort_palette_values(&present) {
        let color = match &keep {
            Some(k) if !k.contains(&value) => OTHER_COLOR.to_string(),
            _ => {
                let c = DISTINCT_COLORS[colored % DISTINCT_COLORS.len()].to_string();
                colored += 1;
                c
            }
        };
        out.insert(value, color);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn numeric_values_sort_numerically_not_lexically() {
        // The whole point: "10" must not land between "1" and "2".
        assert_eq!(
            sort_palette_values(&s(&["10", "2", "1"])),
            s(&["1", "2", "10"])
        );
        // Mixed types are safe: numbers first, then everything else lexically.
        assert_eq!(
            sort_palette_values(&s(&["beta", "10", "alpha", "2"])),
            s(&["2", "10", "alpha", "beta"])
        );
    }

    #[test]
    fn colors_are_assigned_in_sorted_order_and_cycle() {
        let p = get_palette(&s(&["2", "1", "3"]), None, None);
        assert_eq!(p["1"], DISTINCT_COLORS[0]);
        assert_eq!(p["2"], DISTINCT_COLORS[1]);
        assert_eq!(p["3"], DISTINCT_COLORS[2]);
        // 65 distinct values wrap onto the first color again.
        let many: Vec<String> = (0..65).map(|i| format!("{i:03}")).collect();
        let p = get_palette(&many, None, None);
        assert_eq!(p["000"], p["064"]);
    }

    #[test]
    fn max_groups_ranks_by_frequency_and_grays_the_rest() {
        let values = s(&["a", "b", "c"]);
        let counts: HashMap<String, usize> =
            [("a".into(), 1usize), ("b".into(), 9), ("c".into(), 5)]
                .into_iter()
                .collect();
        let p = get_palette(&values, Some(2), Some(&counts));
        // b and c are the two largest groups, so only `a` is grayed out.
        assert_eq!(p["a"], OTHER_COLOR);
        assert_ne!(p["b"], OTHER_COLOR);
        assert_ne!(p["c"], OTHER_COLOR);
        // Colors still follow sort order among the kept groups, not frequency order.
        assert_eq!(p["b"], DISTINCT_COLORS[0]);
        assert_eq!(p["c"], DISTINCT_COLORS[1]);
        // A cap at or above the group count colors everything.
        assert!(!get_palette(&values, Some(3), Some(&counts))
            .values()
            .any(|c| c == OTHER_COLOR));
    }

    #[test]
    fn palette_is_uppercase_hex() {
        // The ssn_FeSOD fixtures check the spelling, not just the color.
        for c in DISTINCT_COLORS {
            assert!(c.starts_with('#') && c.len() == 7, "{c}");
            assert_eq!(c, c.to_uppercase(), "{c} should be uppercase");
        }
        assert_eq!(OTHER_COLOR, "#BFBFBF");
    }
}
