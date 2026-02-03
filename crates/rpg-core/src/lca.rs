//! LCA-based directory grounding using trie-based branching analysis.
//! Implements Algorithm 1 from the RPG-Encoder paper.

use std::collections::HashMap;
use std::path::PathBuf;

/// A trie node for path prefix analysis.
#[derive(Debug, Default)]
struct TrieNode {
    children: HashMap<String, TrieNode>,
    is_terminal: bool,
}

impl TrieNode {
    fn insert(&mut self, segments: &[&str]) {
        if segments.is_empty() {
            self.is_terminal = true;
            return;
        }
        self.children
            .entry(segments[0].to_string())
            .or_default()
            .insert(&segments[1..]);
    }

    fn is_branching(&self) -> bool {
        self.children.len() > 1
    }
}

/// Compute the Lowest Common Ancestor directories for a set of file paths.
///
/// Uses trie-based branching analysis (Algorithm 1 from the paper):
/// 1. Insert all parent directory paths into a prefix trie.
/// 2. Walk the trie and retain branching nodes (multiple children) or terminal nodes.
/// 3. Return the minimal set of directory LCAs that cover the input paths.
pub fn compute_lca(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return Vec::new();
    }

    // Extract parent directories
    let dirs: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| p.parent().map(|d| d.to_path_buf()))
        .collect();

    if dirs.is_empty() {
        return Vec::new();
    }

    // If all paths are in the same directory, return that directory
    if dirs.iter().all(|d| d == &dirs[0]) {
        return vec![dirs[0].clone()];
    }

    // Build trie from directory paths
    let mut root = TrieNode::default();
    for dir in &dirs {
        let segments: Vec<&str> = dir
            .components()
            .map(|c| c.as_os_str().to_str().unwrap_or(""))
            .filter(|s| !s.is_empty())
            .collect();
        root.insert(&segments);
    }

    // Walk trie to find branching/terminal nodes (the LCA boundaries)
    let mut results = Vec::new();
    collect_lca_paths(&root, &mut PathBuf::new(), &mut results);

    if results.is_empty() {
        // Fallback: common prefix of all paths
        if let Some(prefix) = common_prefix(&dirs) {
            results.push(prefix);
        }
    }

    results
}

fn collect_lca_paths(node: &TrieNode, current: &mut PathBuf, results: &mut Vec<PathBuf>) {
    if node.is_branching() || (node.is_terminal && !node.children.is_empty()) {
        // This is a meaningful boundary
        if !current.as_os_str().is_empty() {
            results.push(current.clone());
        }
        return;
    }

    if node.children.is_empty() {
        // Leaf: this is the exact directory
        if !current.as_os_str().is_empty() {
            results.push(current.clone());
        }
        return;
    }

    // Single child: keep walking down
    for (segment, child) in &node.children {
        current.push(segment);
        collect_lca_paths(child, current, results);
        current.pop();
    }
}

/// Find the longest common prefix of a set of paths.
fn common_prefix(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }

    let first: Vec<_> = paths[0].components().collect();
    let mut prefix_len = first.len();

    for path in &paths[1..] {
        let components: Vec<_> = path.components().collect();
        prefix_len = prefix_len.min(components.len());
        for i in 0..prefix_len {
            if first[i] != components[i] {
                prefix_len = i;
                break;
            }
        }
    }

    if prefix_len == 0 {
        None
    } else {
        Some(first[..prefix_len].iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_directory() {
        let paths = vec![
            PathBuf::from("src/data/loader.py"),
            PathBuf::from("src/data/parser.py"),
        ];
        let lca = compute_lca(&paths);
        assert_eq!(lca, vec![PathBuf::from("src/data")]);
    }

    #[test]
    fn test_branching_directories() {
        let paths = vec![
            PathBuf::from("src/data/loaders/csv.py"),
            PathBuf::from("src/data/loaders/json.py"),
            PathBuf::from("src/models/train.py"),
        ];
        let lca = compute_lca(&paths);
        assert!(lca.len() == 2 || lca.contains(&PathBuf::from("src")));
    }

    #[test]
    fn test_empty_paths() {
        let paths: Vec<PathBuf> = vec![];
        let lca = compute_lca(&paths);
        assert!(lca.is_empty());
    }

    #[test]
    fn test_common_prefix() {
        let paths = vec![
            PathBuf::from("src/a/b"),
            PathBuf::from("src/a/c"),
            PathBuf::from("src/a/d"),
        ];
        let prefix = common_prefix(&paths);
        assert_eq!(prefix, Some(PathBuf::from("src/a")));
    }
}
