//! Tests for `.rpgignore` filtering of file changes.

use rpg_core::graph::*;
use rpg_encoder::evolution::{FileChange, filter_rpgignore_changes, find_newly_ignored_files};
use std::path::PathBuf;
use tempfile::TempDir;

fn make_changes() -> Vec<FileChange> {
    vec![
        FileChange::Added(PathBuf::from("src/main.rs")),
        FileChange::Modified(PathBuf::from("benchmarks/search_quality.py")),
        FileChange::Deleted(PathBuf::from("tests/fixtures/model.py")),
        FileChange::Added(PathBuf::from("src/lib.rs")),
    ]
}

#[test]
fn test_filter_rpgignore_no_file() {
    let tmp = TempDir::new().unwrap();
    // No .rpgignore file exists — all changes pass through
    let changes = make_changes();
    let original_len = changes.len();
    let filtered = filter_rpgignore_changes(tmp.path(), changes);
    assert_eq!(filtered.len(), original_len);
}

#[test]
fn test_filter_rpgignore_excludes_matching() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "benchmarks/\ntests/\n").unwrap();

    let changes = make_changes();
    let filtered = filter_rpgignore_changes(tmp.path(), changes);

    // Only src/main.rs and src/lib.rs should remain
    assert_eq!(filtered.len(), 2);
    for change in &filtered {
        let path = match change {
            FileChange::Added(p) | FileChange::Modified(p) | FileChange::Deleted(p) => p,
            FileChange::Renamed { to, .. } => to,
        };
        assert!(
            path.starts_with("src/"),
            "unexpected path in filtered results: {path:?}"
        );
    }
}

#[test]
fn test_filter_rpgignore_glob_pattern() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "*.test.ts\n").unwrap();

    let changes = vec![
        FileChange::Added(PathBuf::from("src/app.ts")),
        FileChange::Added(PathBuf::from("src/app.test.ts")),
        FileChange::Modified(PathBuf::from("lib/utils.test.ts")),
    ];

    let filtered = filter_rpgignore_changes(tmp.path(), changes);
    assert_eq!(filtered.len(), 1);
    match &filtered[0] {
        FileChange::Added(p) => assert_eq!(p, &PathBuf::from("src/app.ts")),
        other => panic!("unexpected change type: {other:?}"),
    }
}

#[test]
fn test_filter_rpgignore_empty_file() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "").unwrap();

    let changes = make_changes();
    let original_len = changes.len();
    let filtered = filter_rpgignore_changes(tmp.path(), changes);
    assert_eq!(filtered.len(), original_len);
}

#[test]
fn test_filter_rpgignore_renamed_file() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "benchmarks/\n").unwrap();

    let changes = vec![
        // Renamed into an ignored directory — should become Deleted(from)
        FileChange::Renamed {
            from: PathBuf::from("src/old.py"),
            to: PathBuf::from("benchmarks/moved.py"),
        },
        // Renamed into a non-ignored directory — should pass through
        FileChange::Renamed {
            from: PathBuf::from("benchmarks/old.py"),
            to: PathBuf::from("src/new.py"),
        },
    ];

    let filtered = filter_rpgignore_changes(tmp.path(), changes);
    assert_eq!(filtered.len(), 2);

    // First: rename into ignored → converted to Deleted(from)
    match &filtered[0] {
        FileChange::Deleted(p) => assert_eq!(p, &PathBuf::from("src/old.py")),
        other => panic!("expected Deleted(src/old.py), got: {other:?}"),
    }
    // Second: rename into non-ignored → passes through as Renamed
    match &filtered[1] {
        FileChange::Renamed { from, to } => {
            assert_eq!(from, &PathBuf::from("benchmarks/old.py"));
            assert_eq!(to, &PathBuf::from("src/new.py"));
        }
        other => panic!("expected Renamed, got: {other:?}"),
    }
}

#[test]
fn test_filter_rpgignore_rename_into_ignored_becomes_deletion() {
    // Regression test: renaming a file into an ignored path must prune the
    // old entities by emitting Deleted(from), not silently dropping the event.
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "vendor/\n").unwrap();

    let changes = vec![FileChange::Renamed {
        from: PathBuf::from("src/utils.rs"),
        to: PathBuf::from("vendor/utils.rs"),
    }];

    let filtered = filter_rpgignore_changes(tmp.path(), changes);
    assert_eq!(filtered.len(), 1);
    match &filtered[0] {
        FileChange::Deleted(p) => assert_eq!(p, &PathBuf::from("src/utils.rs")),
        other => panic!("expected Deleted(src/utils.rs), got: {other:?}"),
    }
}

// --- find_newly_ignored_files tests ---

fn make_test_entity(id: &str, name: &str, file: &str) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Function,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 10,
        parent_class: None,
        semantic_features: vec![],
        hierarchy_path: String::new(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_find_newly_ignored_detects_matching_files() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "benchmarks/\ntests/\n").unwrap();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_test_entity("src/main.rs:foo", "foo", "src/main.rs"));
    graph.insert_entity(make_test_entity(
        "benchmarks/bench.rs:run",
        "run",
        "benchmarks/bench.rs",
    ));
    graph.insert_entity(make_test_entity(
        "tests/test.rs:check",
        "check",
        "tests/test.rs",
    ));

    let ignored = find_newly_ignored_files(tmp.path(), &graph);
    assert_eq!(ignored.len(), 2);

    let paths: Vec<PathBuf> = ignored
        .iter()
        .map(|c| match c {
            FileChange::Deleted(p) => p.clone(),
            _ => panic!("expected Deleted"),
        })
        .collect();
    assert!(paths.contains(&PathBuf::from("benchmarks/bench.rs")));
    assert!(paths.contains(&PathBuf::from("tests/test.rs")));
}

#[test]
fn test_find_newly_ignored_empty_without_rpgignore() {
    let tmp = TempDir::new().unwrap();
    // No .rpgignore file

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_test_entity("src/main.rs:foo", "foo", "src/main.rs"));

    let ignored = find_newly_ignored_files(tmp.path(), &graph);
    assert!(ignored.is_empty());
}

#[test]
fn test_find_newly_ignored_no_matches() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".rpgignore"), "benchmarks/\n").unwrap();

    let mut graph = RPGraph::new("rust");
    graph.insert_entity(make_test_entity("src/main.rs:foo", "foo", "src/main.rs"));
    graph.insert_entity(make_test_entity("src/lib.rs:bar", "bar", "src/lib.rs"));

    let ignored = find_newly_ignored_files(tmp.path(), &graph);
    assert!(ignored.is_empty());
}
