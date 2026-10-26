//! Cross-check `matrix::hierarchy` against scipy: each stdin line is a linkage method
//! followed by a condensed distance vector; each stdout line is a JSON object with the
//! linkage matrix (`z`), `leaves_list` (`leaves`) and the optimally ordered leaves
//! (`optimal`), for comparison with `scipy.cluster.hierarchy` on the same input.
//!
//! ```text
//! echo "average 1 2 3" | cargo run -p domainator-core --example hierarchy_probe
//! ```

use std::io::BufRead;

use domainator_core::matrix::hierarchy::{leaves_list, linkage, optimal_leaf_ordering, Method};

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("reading stdin");
        let mut words = line.split_whitespace();
        let Some(method) = words.next() else {
            continue;
        };
        let method: Method = method.parse().expect("a linkage method");
        let condensed: Vec<f64> = words.map(|w| w.parse().expect("a distance")).collect();
        let z = linkage(&condensed, method);
        let rows: Vec<String> = z
            .iter()
            .map(|m| format!("[{},{},{:?},{}]", m.left, m.right, m.height, m.size))
            .collect();
        let optimal = leaves_list(&optimal_leaf_ordering(&z, &condensed));
        println!(
            "{{\"z\":[{}],\"leaves\":{:?},\"optimal\":{:?}}}",
            rows.join(","),
            leaves_list(&z),
            optimal
        );
    }
}
