//! Tests for `.rpgignore` filtering of file changes.

use rpg_encoder::evolution::{FileChange, filter_rpgignore_changes};
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
        // Renamed into an ignored directory — should be excluded
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
    assert_eq!(filtered.len(), 1);
    match &filtered[0] {
        FileChange::Renamed { to, .. } => assert_eq!(to, &PathBuf::from("src/new.py")),
        other => panic!("unexpected change type: {other:?}"),
    }
}
