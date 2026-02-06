//! Read/write RPG graph files from disk.

use crate::config::StorageConfig;
use crate::graph::RPGraph;
use crate::schema;
use anyhow::{Context, Result};
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

const RPG_DIR: &str = ".rpg";
const RPG_FILE: &str = "graph.json";
const RPG_BACKUP_FILE: &str = "graph.backup.json";

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

/// Get the path to the RPG backup file for a given project root.
pub fn rpg_backup_file(project_root: &Path) -> PathBuf {
    rpg_dir(project_root).join(RPG_BACKUP_FILE)
}

/// Create a backup of the current graph before destructive operations.
/// Returns the backup path if created, or None if no graph exists.
pub fn create_backup(project_root: &Path) -> Result<Option<PathBuf>> {
    if !rpg_exists(project_root) {
        return Ok(None);
    }

    let source = rpg_file(project_root);
    let dest = rpg_backup_file(project_root);

    fs::copy(&source, &dest).with_context(|| {
        format!(
            "failed to backup {} to {}",
            source.display(),
            dest.display()
        )
    })?;

    Ok(Some(dest))
}

/// Zstd magic bytes: 0x28 0xB5 0x2F 0xFD.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Load an RPG from disk.
/// Automatically detects zstd-compressed graph files by magic bytes.
pub fn load(project_root: &Path) -> Result<RPGraph> {
    let path = rpg_file(project_root);
    let raw =
        fs::read(&path).with_context(|| format!("failed to read RPG from {}", path.display()))?;

    let json = if raw.len() >= 4 && raw[..4] == ZSTD_MAGIC {
        // Decompress zstd
        let mut decoder = zstd::Decoder::new(&raw[..]).context("failed to init zstd decoder")?;
        let mut decompressed = String::new();
        decoder
            .read_to_string(&mut decompressed)
            .context("failed to decompress graph.json")?;
        decompressed
    } else {
        String::from_utf8(raw).context("graph.json is not valid UTF-8")?
    };

    let mut graph = schema::from_json(&json)?;

    // Rebuild performance indexes (skipped during deserialization)
    graph.rebuild_edge_index();
    graph.rebuild_hierarchy_index();

    Ok(graph)
}

/// Save an RPG to disk. Also creates `.rpg/.gitignore` and
/// `.rpg/README.md` on first save.
///
/// Pass `storage_config` to enable compression. When `None`, saves uncompressed.
pub fn save(project_root: &Path, graph: &RPGraph) -> Result<()> {
    save_with_config(project_root, graph, &StorageConfig::default())
}

/// Save with explicit storage configuration.
pub fn save_with_config(
    project_root: &Path,
    graph: &RPGraph,
    storage_config: &StorageConfig,
) -> Result<()> {
    let dir = rpg_dir(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create RPG directory {}", dir.display()))?;

    let json = schema::to_json(graph)?;

    if storage_config.compress {
        let compressed = zstd::encode_all(json.as_bytes(), 3)
            .context("failed to compress graph.json with zstd")?;
        fs::write(rpg_file(project_root), compressed)
            .with_context(|| "failed to write graph.json")?;
    } else {
        fs::write(rpg_file(project_root), json).with_context(|| "failed to write graph.json")?;
    }

    // Create .rpg/.gitignore (keeps config local)
    let inner_gitignore = dir.join(".gitignore");
    if !inner_gitignore.exists() {
        let _ = fs::write(&inner_gitignore, "config.toml\n");
    }

    // Create README on first save so people discovering .rpg/ know what it is
    let readme = dir.join("README.md");
    if !readme.exists() {
        let _ = fs::write(&readme, RPG_README);
    }

    Ok(())
}

/// Ensure the .rpg directory has its internal .gitignore.
/// The graph itself is intentionally committed — only local config is ignored.
pub fn ensure_gitignore(project_root: &Path) -> Result<bool> {
    let dir = rpg_dir(project_root);
    fs::create_dir_all(&dir)?;
    let inner_gitignore = dir.join(".gitignore");
    if inner_gitignore.exists() {
        return Ok(true);
    }
    fs::write(&inner_gitignore, "config.toml\n")?;
    Ok(false)
}

const RPG_README: &str = include_str!("templates/rpg_readme.md");
