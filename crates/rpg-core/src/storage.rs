//! Read/write RPG graph files from disk.

use crate::graph::RPGraph;
use crate::schema;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const RPG_DIR: &str = ".rpg";
const RPG_FILE: &str = "graph.json";

/// Get the path to the RPG directory for a given project root.
pub fn rpg_dir(project_root: &Path) -> PathBuf {
    project_root.join(RPG_DIR)
}

/// Get the path to the RPG graph file for a given project root.
pub fn rpg_file(project_root: &Path) -> PathBuf {
    rpg_dir(project_root).join(RPG_FILE)
}

/// Check if an RPG exists for the given project root.
pub fn rpg_exists(project_root: &Path) -> bool {
    rpg_file(project_root).exists()
}

/// Load an RPG from disk.
pub fn load(project_root: &Path) -> Result<RPGraph> {
    let path = rpg_file(project_root);
    let json = fs::read_to_string(&path)
        .with_context(|| format!("failed to read RPG from {}", path.display()))?;
    schema::from_json(&json)
}

/// Save an RPG to disk, creating the .rpg directory if needed.
pub fn save(project_root: &Path, graph: &RPGraph) -> Result<()> {
    let dir = rpg_dir(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create RPG directory {}", dir.display()))?;

    let path = rpg_file(project_root);
    let json = schema::to_json(graph)?;
    fs::write(&path, json).with_context(|| format!("failed to write RPG to {}", path.display()))?;

    Ok(())
}

/// Ensure .rpg is in .gitignore. Returns true if it was already there or was added.
pub fn ensure_gitignore(project_root: &Path) -> Result<bool> {
    let gitignore = project_root.join(".gitignore");

    if gitignore.exists() {
        let content = fs::read_to_string(&gitignore)?;
        if content
            .lines()
            .any(|line| line.trim() == ".rpg" || line.trim() == ".rpg/")
        {
            return Ok(true); // already ignored
        }
        // Append
        let mut new_content = content;
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("\n# RPG-Encoder graph\n.rpg/\n");
        fs::write(&gitignore, new_content)?;
    } else {
        fs::write(&gitignore, "# RPG-Encoder graph\n.rpg/\n")?;
    }

    Ok(false)
}
