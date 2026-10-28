//! `build_tree`: a UPGMA tree from a distance or similarity matrix, or a tree read from
//! a Newick file, written as Newick, as a Cytoscape network laid out as a rectangular
//! tree, or as the Domainator Viewer's tree view. Mirrors `domainator.build_tree`.
//!
//! UPGMA is scipy's `average` linkage, which [`crate::matrix::hierarchy`] ports kernel
//! for kernel; the tree is then read off the linkage the way `scipy`'s `to_tree` builds
//! it, each node's right subtree written before its left one, as upstream walks it.

use std::collections::HashSet;
use std::io;
use std::path::Path;

use serde_json::{json, Value};

use crate::io::alignment::{guess_alphabet, read_alignment, sequences_for};
use crate::io::newick::{self, Tree};
use crate::matrix::cytoscape::NodeTable;
use crate::matrix::hierarchy::{linkage, Method};
use crate::matrix::transform::transform_values;
use crate::matrix::DataMatrix;
use crate::palette::OTHER_COLOR;
use crate::tools::node_table::read_node_table;

/// The xgmml edges' attribute: the branch length.
pub const SCORE_COLUMN: &str = "TREE_DIST";
/// The xgmml layout: the deepest leaf is this far from the root, and leaves are this
/// far apart.
pub const XGMML_TREE_WIDTH: f64 = 1000.0;
pub const XGMML_LEAF_SPACING: f64 = 30.0;

/// How to turn matrix values into distances (`--distance`).
pub const DISTANCE_CONVERSIONS: [&str; 3] = ["auto", "precomputed", "score_dist"];

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.into())
}

/// A scipy condensed distance vector for a square matrix (`condensed_distances`).
///
/// `precomputed`: the values are distances. `score_dist`: the values are similarity
/// scores, distance = 1 - (score / min(row_max, col_max)), and a row or column with no
/// scores at all (0/0) is distance 1 from everything. `auto`: precomputed if the data
/// type is a distance (ends in `_dist`) or not recorded (e.g. dense text), otherwise
/// score_dist. Each pair uses the smaller of its two directional distances.
pub fn condensed_distances(matrix: &DataMatrix, distance: &str) -> io::Result<Vec<f64>> {
    if !DISTANCE_CONVERSIONS.contains(&distance) {
        return Err(invalid(format!(
            "distance must be one of {DISTANCE_CONVERSIONS:?}, not '{distance}'"
        )));
    }
    if !matrix.symmetric_labels() {
        return Err(invalid(
            "Input does not have symmetric axis labels. Can only build a tree from a square \
             matrix comparing a set of sequences to itself.",
        ));
    }
    let data_type = matrix.data_type.as_str();
    let distance = if distance == "auto" {
        if data_type.is_empty() || data_type.ends_with("_dist") {
            "precomputed"
        } else {
            "score_dist"
        }
    } else {
        distance
    };
    let n = matrix.nrows();
    let dense = if distance == "precomputed" {
        matrix.densify()
    } else {
        let mut scored = DataMatrix {
            rows: matrix.rows.clone(),
            cols: matrix.cols.clone(),
            row_lengths: None,
            col_lengths: None,
            data_type: matrix.data_type.clone(),
            storage: crate::matrix::Storage::Dense(matrix.densify()),
        };
        transform_values(&mut scored, "score_dist");
        scored
            .densify()
            .into_iter()
            .map(|v| if v.is_nan() { 1.0 } else { v })
            .collect()
    };
    let mut condensed = Vec::with_capacity(n * n.saturating_sub(1) / 2);
    for i in 0..n {
        for j in i + 1..n {
            condensed.push(dense[i * n + j].min(dense[j * n + i]));
        }
    }
    Ok(condensed)
}

/// The tree of scipy's `to_tree(linkage)`, each node's right subtree before its left
/// one (`_tree_from_linkage`).
fn tree_from_linkage(merges: &[crate::matrix::hierarchy::Merge], names: &[String]) -> Tree {
    let n = names.len();
    // Cluster id -> (left, right, height); leaves are ids below n.
    let node = |id: usize| -> Option<(usize, usize, f64)> {
        (id >= n).then(|| {
            let m = &merges[id - n];
            (m.left, m.right, m.height)
        })
    };
    let height = |id: usize| node(id).map_or(0.0, |(_, _, h)| h);
    let root = n + merges.len() - 1;
    let mut tree = Tree::default();
    let mut stack: Vec<(usize, i64, f64)> = vec![(root, -1, height(root))];
    while let Some((id, parent, parent_dist)) = stack.pop() {
        let index = tree.parent.len() as i64;
        tree.parent.push(parent);
        tree.length.push(if parent >= 0 {
            parent_dist - height(id)
        } else {
            0.0
        });
        tree.name.push((id < n).then(|| names[id].clone()));
        tree.support.push(None);
        if let Some((left, right, h)) = node(id) {
            stack.push((left, index, h));
            stack.push((right, index, h));
        }
    }
    tree
}

/// The UPGMA tree of a distance or similarity matrix's rows (`tree_from_matrix`).
pub fn tree_from_matrix(matrix: &DataMatrix, algorithm: &str, distance: &str) -> io::Result<Tree> {
    if algorithm != "upgma" {
        return Err(invalid(format!(
            "algorithm must be upgma, not '{algorithm}'"
        )));
    }
    if matrix.nrows() == 1 {
        return Ok(Tree {
            parent: vec![-1],
            length: vec![0.0],
            name: vec![Some(matrix.rows[0].clone())],
            support: vec![None],
        });
    }
    let condensed = condensed_distances(matrix, distance)?;
    if matrix.nrows() == 0 {
        return Err(invalid("The matrix has no rows to build a tree from."));
    }
    if let Some(bad) = condensed.iter().find(|v| !v.is_finite()) {
        return Err(invalid(format!(
            "The distance matrix must contain only finite values, not {bad}."
        )));
    }
    let merges = linkage(&condensed, Method::Average);
    Ok(tree_from_linkage(&merges, &matrix.rows))
}

/// The first tree in a Newick file (`_tree_from_newick`). Every leaf needs a unique
/// name.
pub fn tree_from_newick(path: &str) -> io::Result<Tree> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| invalid(format!("Could not read a Newick tree from {path}: {e}")))?;
    let tree = newick::parse(&text)
        .map_err(|e| invalid(format!("Could not read a Newick tree from {path}: {e}")))?;
    let names = tree.leaf_names();
    if names.iter().any(String::is_empty) {
        return Err(invalid(format!(
            "Every leaf of the tree in {path} needs a name."
        )));
    }
    let mut seen = HashSet::new();
    let mut duplicates: Vec<&String> = names.iter().filter(|n| !seen.insert(*n)).collect();
    duplicates.sort();
    duplicates.dedup();
    if !duplicates.is_empty() {
        let shown: Vec<&str> = duplicates.iter().take(10).map(|s| s.as_str()).collect();
        return Err(invalid(format!(
            "Leaf names in {path} must be unique; repeated: {}",
            shown.join(", ")
        )));
    }
    Ok(tree)
}

/// The tree of the leaves named in `keep` (`subset_tree`): other leaves are removed,
/// then internal nodes left with no children, and an internal node left with one child
/// is merged into it, adding their branch lengths.
pub fn subset_tree(tree: &Tree, keep: &HashSet<String>) -> io::Result<Tree> {
    let children = tree.children();
    let mut kept = vec![false; tree.len()];
    for node in (0..tree.len()).rev() {
        kept[node] = if children[node].is_empty() {
            tree.name[node].as_ref().is_some_and(|n| keep.contains(n))
        } else {
            children[node].iter().any(|&c| kept[c])
        };
    }
    if !kept[0] {
        return Err(invalid("None of the tree's leaves are in the subset."));
    }
    let mut out = Tree::default();
    // (node, new parent, length carried down from merged single-child ancestors)
    let mut stack: Vec<(usize, i64, f64)> = vec![(0, -1, 0.0)];
    while let Some((node, parent, carried)) = stack.pop() {
        let kept_children: Vec<usize> = children[node]
            .iter()
            .copied()
            .filter(|&c| kept[c])
            .collect();
        let length = carried + if node > 0 { tree.length[node] } else { 0.0 };
        if kept_children.len() == 1 && !children[node].is_empty() {
            stack.push((kept_children[0], parent, length));
            continue;
        }
        let index = out.parent.len() as i64;
        out.parent.push(parent);
        out.length.push(if parent >= 0 { length } else { 0.0 });
        out.name.push(tree.name[node].clone());
        out.support.push(tree.support[node]);
        for &child in kept_children.iter().rev() {
            stack.push((child, index, 0.0));
        }
    }
    Ok(out)
}

/// Rectangular layout coordinates (`tree_layout`): `(x, y)` for every node, `x` the
/// distance from the root and `y` the leaf's place in preorder, an internal node
/// halfway between its first and last child.
pub fn tree_layout(tree: &Tree) -> (Vec<f64>, Vec<f64>) {
    let children = tree.children();
    let mut x = vec![0.0; tree.len()];
    for node in 1..tree.len() {
        x[node] = tree.parent_of(node).map_or(0.0, |p| x[p]) + tree.length[node];
    }
    let mut y = vec![0.0; tree.len()];
    let mut leaf_rank = 0.0;
    for node in 0..tree.len() {
        if children[node].is_empty() {
            y[node] = leaf_rank;
            leaf_rank += 1.0;
        }
    }
    for node in (0..tree.len()).rev() {
        if let (Some(&first), Some(&last)) = (children[node].first(), children[node].last()) {
            y[node] = (y[first] + y[last]) / 2.0;
        }
    }
    (x, y)
}

/// An internal node's name in the xgmml: its place in the tree in preorder.
pub fn internal_node_name(node: usize) -> String {
    format!("__INTERNAL_NODE_{node}__")
}

/// The xgmml network of a tree (`_xgmml_frames`).
pub struct TreeNetwork {
    /// Every node, leaves first, then internal nodes, with its metadata.
    pub table: NodeTable,
    /// Each table row's tree node.
    pub nodes: Vec<usize>,
    /// Each row's `(x, y)` position, `y` as upstream stores it (negated by the writer).
    pub positions: Vec<(f64, f64, Option<f64>)>,
    /// Every branch, `(parent row, child row, length)`, in preorder of the child.
    pub edges: Vec<(usize, usize, f64)>,
}

/// The tree as an xgmml network: every node, laid out as a rectangular tree, with its
/// metadata, and every branch with its length.
pub fn tree_network(tree: &Tree, metadata_files: &[String]) -> io::Result<TreeNetwork> {
    let children = tree.children();
    let names: Vec<String> = (0..tree.len())
        .map(|n| {
            if children[n].is_empty() {
                tree.name[n].clone().unwrap_or_default()
            } else {
                internal_node_name(n)
            }
        })
        .collect();
    let (x, y) = tree_layout(tree);
    let depth = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let depth = if depth == 0.0 || !depth.is_finite() {
        1.0
    } else {
        depth
    };
    let order: Vec<usize> = tree
        .leaves()
        .into_iter()
        .chain((0..tree.len()).filter(|&n| !children[n].is_empty()))
        .collect();
    let mut row_of = vec![0usize; tree.len()];
    for (row, &node) in order.iter().enumerate() {
        row_of[node] = row;
    }
    let row_names: Vec<String> = order.iter().map(|&n| names[n].clone()).collect();
    let table = read_node_table(&row_names, Vec::new(), metadata_files)?;
    let positions = order
        .iter()
        .map(|&n| {
            (
                x[n] * XGMML_TREE_WIDTH / depth,
                -y[n] * XGMML_LEAF_SPACING,
                None,
            )
        })
        .collect();
    let edges = (1..tree.len())
        .filter_map(|n| Some((row_of[tree.parent_of(n)?], row_of[n], tree.length[n])))
        .collect();
    Ok(TreeNetwork {
        table,
        nodes: order,
        positions,
        edges,
    })
}

/// The xgmml fills of a tree network's rows: each leaf's color from `leaf_fills`
/// (aligned to the tree's leaves in preorder), and the gray of a missing value for
/// every internal node.
pub fn network_fills(
    network: &TreeNetwork,
    tree: &Tree,
    leaf_fills: &[Option<String>],
) -> Vec<Option<String>> {
    let leaves = tree.leaves();
    let leaf_index: std::collections::HashMap<usize, usize> =
        leaves.iter().enumerate().map(|(i, &n)| (n, i)).collect();
    network
        .nodes
        .iter()
        .map(|n| match leaf_index.get(n) {
            Some(&i) => leaf_fills.get(i).cloned().flatten(),
            None => Some(OTHER_COLOR.to_string()),
        })
        .collect()
}

/// The tree view's `msa` (`read_msa`): `{alphabet, length, sequences}`, `sequences`
/// holding each leaf's aligned sequence (`null` for a leaf the alignment lacks). Also
/// returns the warnings upstream raises about leaves and sequences that do not match.
pub fn read_msa(path: &str, leaf_names: &[String]) -> io::Result<(Value, Vec<String>)> {
    let alignment = read_alignment(Path::new(path))?;
    let (sequences, warnings) = sequences_for(&alignment, leaf_names, path);
    let alphabet = guess_alphabet(alignment.sequences.iter().map(Vec::as_slice));
    Ok((
        json!({
            "alphabet": alphabet,
            "length": alignment.length,
            "sequences": sequences,
        }),
        warnings,
    ))
}

/// The bundle's `views.tree` (`tree_view`): the tree's arrays, which node each of the
/// bundle's nodes (the leaves, in preorder) is, and the alignment.
pub fn tree_view(tree: &Tree, msa: Option<Value>) -> Value {
    let children = tree.children();
    let label: Vec<Option<&String>> = (0..tree.len())
        // Leaves are named by the bundle's nodes; only internal labels are kept here.
        .map(|n| {
            if children[n].is_empty() {
                None
            } else {
                tree.name[n].as_ref()
            }
        })
        .collect();
    json!({
        "parent": tree.parent,
        "length": tree.length,
        "label": label,
        "support": tree.support,
        "leaf_nodes": tree.leaves(),
        "msa": msa,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::Storage;

    fn matrix(names: &[&str], values: &[f64], data_type: &str) -> DataMatrix {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        DataMatrix {
            rows: names.clone(),
            cols: names,
            row_lengths: None,
            col_lengths: None,
            data_type: data_type.into(),
            storage: Storage::Dense(values.to_vec()),
        }
    }

    #[test]
    fn upgma_tree_in_upstreams_order() {
        // a-b merge at 2, c joins at mean(6, 6) = 6 (scipy's heights, not halved): the
        // root's right subtree (the a-b cluster) comes first in preorder, then c. The
        // expected values are upstream's for this matrix.
        let m = matrix(
            &["a", "b", "c"],
            &[0.0, 2.0, 6.0, 2.0, 0.0, 6.0, 6.0, 6.0, 0.0],
            "",
        );
        let tree = tree_from_matrix(&m, "upgma", "auto").unwrap();
        assert_eq!(tree.parent, [-1, 0, 1, 1, 0]);
        assert_eq!(
            tree.name,
            [
                None,
                None,
                Some("b".into()),
                Some("a".into()),
                Some("c".into())
            ]
        );
        assert_eq!(tree.length, [0.0, 4.0, 2.0, 2.0, 6.0]);
        assert_eq!(newick::write(&tree, |_, _| {}), "((b:2,a:2):4,c:6);");
    }

    #[test]
    fn similarity_scores_become_distances() {
        // score_dist: 1 - s / min(row_max, col_max), the smaller direction per pair.
        let m = matrix(&["a", "b"], &[10.0, 5.0, 4.0, 8.0], "score");
        let d = condensed_distances(&m, "auto").unwrap();
        assert_eq!(d, [1.0 - 5.0 / 8.0]);
        assert_eq!(condensed_distances(&m, "precomputed").unwrap(), [4.0]);
        // A row with no scores at all is distance 1 from everything.
        let m = matrix(&["a", "b"], &[0.0, 0.0, 0.0, 0.0], "score");
        assert_eq!(condensed_distances(&m, "score_dist").unwrap(), [1.0]);
        let asym = DataMatrix {
            cols: vec!["x".into(), "y".into()],
            ..matrix(&["a", "b"], &[0.0; 4], "")
        };
        assert!(condensed_distances(&asym, "auto").is_err());
    }

    #[test]
    fn subsetting_merges_single_child_nodes() {
        let tree = newick::parse("((a:1,b:2)x:3,c:4);").unwrap();
        let keep: HashSet<String> = ["a".to_string(), "c".to_string()].into();
        let sub = subset_tree(&tree, &keep).unwrap();
        assert_eq!(newick::write(&sub, |_, _| {}), "(a:4,c:4);");
        assert!(subset_tree(&tree, &HashSet::new()).is_err());
    }

    #[test]
    fn rectangular_layout_and_network() {
        let tree = newick::parse("((a:1,b:1):1,c:4);").unwrap();
        let (x, y) = tree_layout(&tree);
        assert_eq!(x, [0.0, 1.0, 2.0, 2.0, 4.0]);
        assert_eq!(y, [1.25, 0.5, 0.0, 1.0, 2.0]);
        let network = tree_network(&tree, &[]).unwrap();
        assert_eq!(
            network.table.names,
            ["a", "b", "c", "__INTERNAL_NODE_0__", "__INTERNAL_NODE_1__"]
        );
        assert_eq!(network.positions[0], (500.0, -0.0, None));
        assert_eq!(network.positions[2], (1000.0, -60.0, None));
        // Branches in preorder of the child: root->n1, n1->a, n1->b, root->c.
        assert_eq!(
            network.edges,
            [(3, 4, 1.0), (4, 0, 1.0), (4, 1, 1.0), (3, 2, 4.0)]
        );
    }

    #[test]
    fn the_view_keeps_only_internal_labels() {
        let tree = newick::parse("((a,b)90,(c,d)x);").unwrap();
        let view = tree_view(&tree, None);
        assert_eq!(
            view["label"],
            json!([null, null, null, null, "x", null, null])
        );
        assert_eq!(
            view["support"],
            json!([null, 90.0, null, null, null, null, null])
        );
        assert_eq!(view["leaf_nodes"], json!([2, 3, 5, 6]));
        assert_eq!(view["msa"], Value::Null);
    }
}
