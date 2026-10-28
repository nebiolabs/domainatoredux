//! Rooted trees, and the Newick format.
//!
//! [`Tree`] is upstream `build_tree.Tree`: arrays over the nodes in preorder. [`parse`]
//! reads Newick the way `Bio.Phylo.read(path, "newick")` does (Biopython's `NewickIO`
//! tokenizer and parser, followed rule for rule), and [`write()`] is upstream's
//! `get_newick`.

use std::fmt;

use crate::search::results::py_g;

/// A rooted tree as arrays over its nodes in preorder, so node 0 is the root and every
/// node comes after its parent; children are in the order they were written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    /// Each node's parent, `-1` for the root.
    pub parent: Vec<i64>,
    /// Each node's branch length to its parent (0 for the root).
    pub length: Vec<f64>,
    /// Each leaf's name; an internal node's label, or `None`.
    pub name: Vec<Option<String>>,
    /// Each internal node's support value, or `None`.
    pub support: Vec<Option<f64>>,
}

impl Tree {
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    fn push(&mut self, parent: i64, length: f64, name: Option<String>, support: Option<f64>) {
        self.parent.push(parent);
        self.length.push(length);
        self.name.push(name);
        self.support.push(support);
    }

    /// A node's parent, or `None` for the root.
    pub fn parent_of(&self, node: usize) -> Option<usize> {
        usize::try_from(self.parent[node]).ok()
    }

    /// Each node's children, in order.
    pub fn children(&self) -> Vec<Vec<usize>> {
        let mut children = vec![Vec::new(); self.len()];
        for node in 0..self.len() {
            if let Some(parent) = self.parent_of(node) {
                children[parent].push(node);
            }
        }
        children
    }

    /// The leaf nodes, in preorder (the order the tree draws them).
    pub fn leaves(&self) -> Vec<usize> {
        let children = self.children();
        (0..self.len())
            .filter(|&n| children[n].is_empty())
            .collect()
    }

    /// The leaves' names, in preorder.
    pub fn leaf_names(&self) -> Vec<String> {
        self.leaves()
            .into_iter()
            .map(|n| self.name[n].clone().unwrap_or_default())
            .collect()
    }
}

/// A Newick file Biopython would refuse, with its reason.
#[derive(Debug, Clone, PartialEq)]
pub struct NewickError(pub String);

impl fmt::Display for NewickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NewickError {}

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Open,
    Close,
    Label(&'a str),
    Length(&'a str),
    Comma,
    Comment,
    Quoted(&'a str),
    Semicolon,
}

/// `NewickIO.tokenizer`: at each position, the first of its alternatives that matches;
/// characters no alternative matches (spaces, a `:` with no number after it) are
/// skipped, as `re.finditer` skips them.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match match_at(text, i) {
            Some((token, end)) => {
                out.push(token);
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

fn is_label_byte(b: u8) -> bool {
    !(b.is_ascii_whitespace() || b"()[]':;,".contains(&b))
}

fn match_at(text: &str, i: usize) -> Option<(Token<'_>, usize)> {
    let bytes = text.as_bytes();
    let b = bytes[i];
    match b {
        b'(' => return Some((Token::Open, i + 1)),
        b')' => return Some((Token::Close, i + 1)),
        _ => {}
    }
    // Unquoted node label: [^\s\(\)\[\]\'\:\;\,]+ (non-ASCII bytes are label bytes, so
    // a multi-byte character is never split).
    if is_label_byte(b) {
        let end = (i..bytes.len())
            .find(|&j| !is_label_byte(bytes[j]))
            .unwrap_or(bytes.len());
        return Some((Token::Label(&text[i..end]), end));
    }
    if b == b':' {
        if let Some(end) = match_length(bytes, i) {
            return Some((Token::Length(&text[i..end]), end));
        }
    }
    match b {
        b',' => return Some((Token::Comma, i + 1)),
        b'[' => {
            // \[(\\.|[^\]])*\]
            let end = match_delimited(bytes, i, b']')?;
            return Some((Token::Comment, end));
        }
        b'\'' => {
            // \'(\\.|[^\'])*\'
            let end = match_delimited(bytes, i, b'\'')?;
            return Some((Token::Quoted(&text[i..end]), end));
        }
        b';' => return Some((Token::Semicolon, i + 1)),
        _ => {}
    }
    None
}

/// `\:\ ?[+-]?[0-9]*\.?[0-9]+([eE][+-]?[0-9]+)?` at `i` (a `:`): the end of the match.
fn match_length(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if bytes.get(j) == Some(&b' ') {
        j += 1;
    }
    if matches!(bytes.get(j), Some(b'+' | b'-')) {
        j += 1;
    }
    let digits = |from: usize| {
        (from..bytes.len())
            .find(|&k| !bytes[k].is_ascii_digit())
            .unwrap_or(bytes.len())
    };
    // [0-9]*\.?[0-9]+ : backtracking can only move the last digit of an integer part
    // into the mandatory [0-9]+, so it matches iff there is a digit after the optional
    // dot, or the integer part is not empty.
    let int_end = digits(j);
    let mut end = if bytes.get(int_end) == Some(&b'.') {
        let frac_end = digits(int_end + 1);
        if frac_end > int_end + 1 {
            frac_end
        } else if int_end > j {
            int_end
        } else {
            return None;
        }
    } else if int_end > j {
        int_end
    } else {
        return None;
    };
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut k = end + 1;
        if matches!(bytes.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        let exp_end = digits(k);
        if exp_end > k {
            end = exp_end;
        }
    }
    Some(end)
}

/// A token from `bytes[i]` (the opening delimiter) to the next unescaped `close`,
/// where a backslash escapes the next byte: the end of the match, or `None` when it is
/// never closed.
fn match_delimited(bytes: &[u8], i: usize, close: u8) -> Option<usize> {
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        if bytes[j] == close {
            return Some(j + 1);
        }
        j += 1;
    }
    None
}

/// `NewickIO._parse_confidence`: the label as a number (Python's `float`, which like
/// Rust's parser takes `inf` and `nan` in any case and ignores surrounding space), or
/// nothing.
fn parse_confidence(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok()
}

/// A clade while it is being parsed.
#[derive(Default)]
struct Clade {
    name: Option<String>,
    length: Option<f64>,
    confidence: Option<f64>,
    children: Vec<usize>,
    parent: Option<usize>,
}

/// One tree's text, parsed as `NewickIO.Parser._parse_tree` parses it.
fn parse_tree(text: &str) -> Result<Tree, NewickError> {
    let mut clades: Vec<Clade> = vec![Clade::default()];
    let mut root = 0usize;
    let mut current = 0usize;
    let mut open = 0usize;
    let mut close = 0usize;

    // `process_clade`: a named internal clade whose name is a number has it as its
    // support value instead. Then the clade joins its parent's children.
    fn process(clades: &mut [Clade], clade: usize) -> Option<usize> {
        let c = &mut clades[clade];
        if c.name.as_deref().is_some_and(|n| !n.is_empty())
            && c.confidence.is_none()
            && !c.children.is_empty()
        {
            c.confidence = parse_confidence(c.name.as_deref().unwrap_or_default());
            if c.confidence.is_some() {
                c.name = None;
            }
        }
        let parent = c.parent.take()?;
        clades[parent].children.push(clade);
        Some(parent)
    }
    let new_clade = |clades: &mut Vec<Clade>, parent: Option<usize>| {
        clades.push(Clade {
            parent,
            ..Default::default()
        });
        clades.len() - 1
    };

    let tokens = tokenize(text.trim());
    let mut rest = tokens.into_iter();
    let mut ended = false;
    for token in rest.by_ref() {
        match token {
            Token::Quoted(q) => {
                let c = &mut clades[current];
                match &mut c.name {
                    Some(name) if !name.is_empty() => {
                        // Two consecutive quoted labels are an escaped quote: 'a''b'.
                        name.push_str(&q[..q.len() - 1]);
                    }
                    _ => c.name = Some(q[1..q.len() - 1].to_string()),
                }
            }
            Token::Comment => {}
            Token::Open => {
                current = new_clade(&mut clades, Some(current));
                open += 1;
            }
            Token::Comma => {
                // Missing external parentheses: a new root above the current one.
                if current == root {
                    root = new_clade(&mut clades, None);
                    clades[current].parent = Some(root);
                }
                let parent = process(&mut clades, current)
                    .ok_or_else(|| NewickError("Parenthesis mismatch.".into()))?;
                current = new_clade(&mut clades, Some(parent));
            }
            Token::Close => {
                let parent = process(&mut clades, current)
                    .ok_or_else(|| NewickError("Parenthesis mismatch.".into()))?;
                current = parent;
                close += 1;
            }
            Token::Semicolon => {
                ended = true;
                break;
            }
            Token::Length(l) => {
                let value: f64 = l[1..].trim().parse().map_err(|_| {
                    NewickError(format!("could not convert string to float: '{}'", &l[1..]))
                })?;
                clades[current].length = Some(value);
            }
            Token::Label(l) => clades[current].name = Some(l.to_string()),
        }
    }
    if open != close {
        return Err(NewickError(format!(
            "Mismatch, {open} open vs {close} close parentheses."
        )));
    }
    if ended {
        if let Some(next) = rest.next() {
            return Err(NewickError(format!(
                "Text after semicolon in Newick tree: {next:?}"
            )));
        }
    }
    process(&mut clades, current);
    process(&mut clades, root);

    // Preorder, children in the order they were written.
    let mut tree = Tree::default();
    let mut stack: Vec<(usize, i64)> = vec![(root, -1)];
    let mut rows: Vec<(usize, i64)> = Vec::new();
    while let Some((clade, parent)) = stack.pop() {
        let index = rows.len() as i64;
        rows.push((clade, parent));
        for &child in clades[clade].children.iter().rev() {
            stack.push((child, index));
        }
    }
    // A tree with no branch lengths at all draws as a cladogram, length 1 everywhere;
    // otherwise a missing length is 0.
    let has_lengths = rows[1..].iter().any(|&(c, _)| clades[c].length.is_some());
    for (i, &(clade, parent)) in rows.iter().enumerate() {
        let c = &clades[clade];
        let length = if i == 0 {
            0.0
        } else if has_lengths {
            c.length.unwrap_or(0.0)
        } else {
            1.0
        };
        let support = if c.children.is_empty() {
            None
        } else {
            c.confidence
        };
        tree.push(parent, length, c.name.clone(), support);
    }
    Ok(tree)
}

/// The tree in a Newick file's text, as `Bio.Phylo.read(path, "newick")` reads it:
/// numeric internal labels are support values. The file must hold exactly one tree.
pub fn parse(text: &str) -> Result<Tree, NewickError> {
    // `Parser.parse`: lines are joined (right-stripped) until one ends a tree.
    let mut trees = Vec::new();
    let mut buf = String::new();
    for line in text.split_inclusive('\n') {
        buf.push_str(line.trim_end());
        if buf.ends_with(';') {
            trees.push(parse_tree(&buf)?);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        // The last tree is missing its terminal ';' -- that's OK.
        trees.push(parse_tree(&buf)?);
    }
    let tree = match trees.len() {
        0 => return Err(NewickError("There are no trees in this file.".into())),
        1 => trees.pop().unwrap(),
        _ => {
            return Err(NewickError(
                "There are multiple trees in this file; use parse() instead.".into(),
            ))
        }
    };
    Ok(tree)
}

/// `clean_name_for_newick`: each of ` ;:,()'` becomes `_`. Returns whether the name
/// changed, so the caller can warn.
pub fn clean_name(name: &str) -> (String, bool) {
    let out: String = name
        .chars()
        .map(|c| if " ;:,()'".contains(c) { '_' } else { c })
        .collect();
    let changed = out != name;
    (out, changed)
}

/// The tree in Newick format (`get_newick`), with branch lengths to six significant
/// digits. Internal nodes are written with their labels, or failing that their support
/// values. `on_rename` is told about each name changed for Newick.
pub fn write(tree: &Tree, mut on_rename: impl FnMut(&str, &str)) -> String {
    let mut clean = |name: &str| {
        let (out, changed) = clean_name(name);
        if changed {
            on_rename(name, &out);
        }
        out
    };
    let children = tree.children();
    if children[0].is_empty() {
        // Only one leaf: the tree is just a leaf.
        return format!("({}:0);", clean(tree.name[0].as_deref().unwrap_or("")));
    }
    enum Item {
        Node(usize),
        Text(String),
    }
    let mut out = String::new();
    let mut stack = vec![Item::Node(0)];
    while let Some(item) = stack.pop() {
        let node = match item {
            Item::Text(t) => {
                out.push_str(&t);
                continue;
            }
            Item::Node(n) => n,
        };
        let branch = if node == 0 {
            String::new()
        } else {
            format!(":{}", py_g(tree.length[node]))
        };
        if children[node].is_empty() {
            out.push_str(&clean(tree.name[node].as_deref().unwrap_or("")));
            out.push_str(&branch);
            continue;
        }
        let label = match (&tree.name[node], tree.support[node]) {
            (Some(name), _) => clean(name),
            (None, Some(support)) => py_g(support),
            (None, None) => String::new(),
        };
        out.push('(');
        stack.push(Item::Text(format!("){label}{branch}")));
        for (i, &child) in children[node].iter().rev().enumerate() {
            if i > 0 {
                stack.push(Item::Text(",".into()));
            }
            stack.push(Item::Node(child));
        }
    }
    out.push(';');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_values_labels_and_lengths() {
        // Upstream's tree_support.newick shape: numeric internal labels are support, a
        // non-numeric one stays a label, and a multifurcation keeps its child order.
        let tree = parse("((A:0.1,B:0.2)90:0.3,(C:0.1,D:0.1,E:0.2)clade_x:0.4,(F:1,G:2)75:0.5);\n")
            .unwrap();
        assert_eq!(tree.leaf_names(), ["A", "B", "C", "D", "E", "F", "G"]);
        assert_eq!(tree.parent[..4], [-1, 0, 1, 1]);
        assert_eq!(tree.support[1], Some(90.0));
        assert_eq!(tree.name[1], None);
        assert_eq!(tree.name[4].as_deref(), Some("clade_x"));
        assert_eq!(tree.support[4], None);
        assert_eq!(tree.length[2], 0.1);
        assert_eq!(
            write(&tree, |_, _| {}),
            "((A:0.1,B:0.2)90:0.3,(C:0.1,D:0.1,E:0.2)clade_x:0.4,(F:1,G:2)75:0.5);"
        );
    }

    #[test]
    fn a_tree_without_lengths_is_a_cladogram() {
        let tree = parse("((a,b),c);").unwrap();
        assert_eq!(tree.length, [0.0, 1.0, 1.0, 1.0, 1.0]);
        // Any length at all makes the missing ones 0.
        let tree = parse("((a:2,b),c);").unwrap();
        assert_eq!(tree.length, [0.0, 0.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn biopython_tokenizer_edge_cases() {
        // Quoted labels with spaces and an escaped quote, comments, whitespace and line
        // breaks inside the tree, and a missing final semicolon.
        let tree = parse("('a b':1,\n 'it''s'[&&NHX:S=x] : 2 , c:3e-1)").unwrap();
        assert_eq!(tree.leaf_names(), ["a b", "it's", "c"]);
        assert_eq!(tree.length[2], 2.0);
        assert_eq!(tree.length[3], 0.3);
        // Missing external parentheses get a root above them.
        let tree = parse("a:1,b:2;").unwrap();
        assert_eq!(tree.leaf_names(), ["a", "b"]);
        assert_eq!(tree.parent, [-1, 0, 0]);
    }

    #[test]
    fn malformed_trees_are_refused() {
        assert!(parse("((a,b),c;")
            .unwrap_err()
            .0
            .contains("Mismatch, 2 open vs 1 close"));
        assert!(parse("(a,b);\n(c,d);")
            .unwrap_err()
            .0
            .contains("multiple trees"));
        // On one line, Biopython reads both as one tree with text after its end.
        assert!(parse("(a,b);(c,d);")
            .unwrap_err()
            .0
            .contains("Text after semicolon"));
        assert!(parse("(a,(b,c));x")
            .unwrap_err()
            .0
            .contains("Text after semicolon"));
        // An empty leaf parses; whether that is acceptable is the caller's rule.
        assert_eq!(parse("(a,);").unwrap().leaf_names(), ["a", ""]);
    }

    #[test]
    fn names_are_cleaned_for_newick() {
        let tree = Tree {
            parent: vec![-1, 0, 0],
            length: vec![0.0, 0.123456789, 1234567.0],
            name: vec![None, Some("a b".into()), Some("c:d".into())],
            support: vec![None; 3],
        };
        let mut renamed = Vec::new();
        let text = write(&tree, |from, to| {
            renamed.push((from.to_string(), to.to_string()))
        });
        assert_eq!(text, "(a_b:0.123457,c_d:1.23457e+06);");
        assert_eq!(renamed.len(), 2);
        let leaf = Tree {
            parent: vec![-1],
            length: vec![0.0],
            name: vec![Some("x".into())],
            support: vec![None],
        };
        assert_eq!(write(&leaf, |_, _| {}), "(x:0);");
    }
}
