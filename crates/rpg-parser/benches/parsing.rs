use criterion::{Criterion, criterion_group, criterion_main};
use rpg_parser::entities::extract_entities;
use rpg_parser::languages::Language;
use std::hint::black_box;
use std::path::PathBuf;

const SAMPLE_PYTHON: &str = r#"
import os
from typing import List, Optional

class UserManager:
    """Manages user accounts and authentication."""

    def __init__(self, db_url: str):
        self.db_url = db_url
        self.connection = None

    def connect(self) -> bool:
        """Establish database connection."""
        try:
            self.connection = create_connection(self.db_url)
            return True
        except ConnectionError:
            return False

    def get_user(self, user_id: int) -> Optional[dict]:
        """Retrieve user by ID."""
        if not self.connection:
            raise RuntimeError("Not connected")
        return self.connection.execute("SELECT * FROM users WHERE id = ?", (user_id,))

    def list_users(self, limit: int = 100) -> List[dict]:
        """List all users with pagination."""
        return self.connection.execute("SELECT * FROM users LIMIT ?", (limit,))

    def create_user(self, name: str, email: str) -> int:
        """Create a new user and return their ID."""
        result = self.connection.execute(
            "INSERT INTO users (name, email) VALUES (?, ?)",
            (name, email),
        )
        return result.lastrowid

    def delete_user(self, user_id: int) -> bool:
        """Delete a user by ID."""
        result = self.connection.execute("DELETE FROM users WHERE id = ?", (user_id,))
        return result.rowcount > 0


def parse_config(path: str) -> dict:
    """Parse a TOML configuration file."""
    with open(path) as f:
        return toml.load(f)


def validate_email(email: str) -> bool:
    """Validate an email address format."""
    import re
    pattern = r'^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$'
    return bool(re.match(pattern, email))
"#;

const SAMPLE_RUST: &str = r"
use std::collections::HashMap;
use std::path::Path;

pub struct Config {
    pub entries: HashMap<String, String>,
    pub path: String,
}

impl Config {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            entries: HashMap::new(),
            path: path.into(),
        }
    }

    pub fn load(&mut self) -> Result<(), std::io::Error> {
        let content = std::fs::read_to_string(&self.path)?;
        for line in content.lines() {
            if let Some((key, val)) = line.split_once('=') {
                self.entries.insert(key.trim().to_string(), val.trim().to_string());
            }
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|s| s.as_str())
    }
}

pub fn process_files(dir: &Path) -> Vec<String> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                results.push(entry.path().display().to_string());
            }
        }
    }
    results
}
";

fn bench_python_extraction(c: &mut Criterion) {
    let path = PathBuf::from("test.py");
    c.bench_function("extract_entities_python", |b| {
        b.iter(|| extract_entities(black_box(&path), black_box(SAMPLE_PYTHON), Language::PYTHON))
    });
}

fn bench_rust_extraction(c: &mut Criterion) {
    let path = PathBuf::from("test.rs");
    c.bench_function("extract_entities_rust", |b| {
        b.iter(|| extract_entities(black_box(&path), black_box(SAMPLE_RUST), Language::RUST))
    });
}

fn bench_parallel_parsing(c: &mut Criterion) {
    // Generate multiple files to parse in parallel
    let files: Vec<(PathBuf, String)> = (0..50)
        .map(|i| {
            (
                PathBuf::from(format!("file_{}.py", i)),
                SAMPLE_PYTHON.to_string(),
            )
        })
        .collect();

    c.bench_function("parse_files_parallel_50", |b| {
        b.iter(|| rpg_parser::parse_files_parallel(black_box(files.clone())))
    });
}

criterion_group!(
    benches,
    bench_python_extraction,
    bench_rust_extraction,
    bench_parallel_parsing,
);
criterion_main!(benches);
