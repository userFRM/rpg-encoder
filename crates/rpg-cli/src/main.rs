//! CLI binary for RPG-Encoder: build, query, and evolve semantic code graphs.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rpg_core::config::RpgConfig;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "rpg-encoder", about = "Repository Planning Graph encoder")]
struct Cli {
    /// Project root directory (defaults to current directory)
    #[arg(short, long, global = true)]
    project: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a full RPG from the codebase
    Build {
        /// Primary language (auto-detected if not specified)
        #[arg(short, long)]
        lang: Option<String>,

        /// Glob patterns to include files (repeatable)
        #[arg(long)]
        include: Vec<String>,

        /// Glob patterns to exclude files (repeatable)
        #[arg(long)]
        exclude: Vec<String>,

        /// Rebuild even if .rpg/graph.json already exists
        #[arg(long)]
        force: bool,
    },

    /// Incrementally update the RPG from git changes
    Update {
        /// Base commit to diff from (defaults to RPG's base_commit)
        #[arg(long)]
        since: Option<String>,
    },

    /// Search for entities by intent or keywords
    Search {
        /// Search query
        query: String,

        /// Search mode: features, snippets, auto
        #[arg(short, long, default_value = "auto")]
        mode: String,

        /// Restrict search to a hierarchy scope
        #[arg(long)]
        scope: Option<String>,

        /// Filter to entities within a line range (e.g., "10-50")
        #[arg(long)]
        line_range: Option<String>,

        /// Glob pattern to filter entities by file path (e.g., "src/**/*.rs")
        #[arg(long)]
        file_pattern: Option<String>,
    },

    /// Fetch detailed info about a specific entity
    Fetch {
        /// Entity ID
        entity_id: String,
    },

    /// Explore dependency graph from an entity
    Explore {
        /// Starting entity ID
        entity_id: String,

        /// Direction: up, down, both
        #[arg(short, long, default_value = "down")]
        direction: String,

        /// Maximum traversal depth
        #[arg(long, default_value = "2")]
        depth: usize,
    },

    /// Show RPG statistics
    Info,

    /// Export graph as DOT (Graphviz) or Mermaid flowchart
    Export {
        /// Output format: dot, mermaid
        #[arg(short, long, default_value = "dot")]
        format: String,
    },

    /// Show what would change without updating (dry-run)
    Diff {
        /// Base commit to diff from (defaults to RPG's base_commit)
        #[arg(long)]
        since: Option<String>,
    },

    /// Validate graph integrity (check for orphans, dangling edges, etc.)
    Validate,

    /// Install or uninstall the git pre-commit hook for auto-sync
    Hook {
        /// Action: "install" or "uninstall"
        action: String,
    },

    /// Start MCP server (use rpg-mcp-server binary instead)
    #[command(hide = true)]
    Serve,
}

fn get_project_root(cli: &Cli) -> Result<PathBuf> {
    match &cli.project {
        Some(p) => Ok(p.clone()),
        None => std::env::current_dir().context("failed to get current directory"),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let project_root = get_project_root(&cli)?;

    match cli.command {
        Commands::Build {
            lang,
            include,
            exclude,
            force,
        } => cmd_build(&project_root, lang, include, exclude, force),
        Commands::Update { since } => cmd_update(&project_root, since),
        Commands::Search {
            query,
            mode,
            scope,
            line_range,
            file_pattern,
        } => cmd_search(
            &project_root,
            &query,
            &mode,
            scope.as_deref(),
            line_range.as_deref(),
            file_pattern.as_deref(),
        ),
        Commands::Fetch { entity_id } => cmd_fetch(&project_root, &entity_id),
        Commands::Explore {
            entity_id,
            direction,
            depth,
        } => cmd_explore(&project_root, &entity_id, &direction, depth),
        Commands::Info => cmd_info(&project_root),
        Commands::Export { format } => cmd_export(&project_root, &format),
        Commands::Diff { since } => cmd_diff(&project_root, since),
        Commands::Validate => cmd_validate(&project_root),
        Commands::Hook { action } => cmd_hook(&project_root, &action),
        Commands::Serve => {
            eprintln!("MCP server not yet implemented. Use rpg-mcp binary instead.");
            Ok(())
        }
    }
}

/// Collect source files matching language and glob filters.
fn collect_source_files(
    project_root: &Path,
    languages: &[rpg_parser::languages::Language],
    include: &[String],
    exclude: &[String],
) -> Vec<(std::path::PathBuf, String)> {
    use indicatif::{ProgressBar, ProgressStyle};
    use rpg_parser::languages::Language;

    let include_set = if include.is_empty() {
        None
    } else {
        let mut builder = globset::GlobSetBuilder::new();
        for p in include {
            builder.add(globset::Glob::new(p).expect("invalid --include glob"));
        }
        Some(builder.build().expect("invalid --include glob set"))
    };
    let exclude_set = if exclude.is_empty() {
        None
    } else {
        let mut builder = globset::GlobSetBuilder::new();
        for p in exclude {
            builder.add(globset::Glob::new(p).expect("invalid --exclude glob"));
        }
        Some(builder.build().expect("invalid --exclude glob set"))
    };

    let walker = ignore::WalkBuilder::new(project_root)
        .hidden(true)
        .git_ignore(true)
        .add_custom_ignore_filename(".rpgignore")
        .build();

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("valid template"),
    );
    spinner.set_message("Scanning files...");

    let mut files_to_parse = Vec::new();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let file_lang = Language::from_extension(ext);
        if !file_lang.is_some_and(|l| languages.contains(&l)) {
            continue;
        }
        let rel_path_for_glob = path.strip_prefix(project_root).unwrap_or(path);
        if let Some(ref inc) = include_set
            && !inc.is_match(rel_path_for_glob)
        {
            continue;
        }
        if let Some(ref exc) = exclude_set
            && exc.is_match(rel_path_for_glob)
        {
            continue;
        }

        if let Ok(source) = std::fs::read_to_string(path) {
            let rel_path = path
                .strip_prefix(project_root)
                .unwrap_or(path)
                .to_path_buf();
            files_to_parse.push((rel_path, source));
            spinner.set_message(format!("{} files collected", files_to_parse.len()));
            spinner.tick();
        }
    }
    spinner.finish_and_clear();
    files_to_parse
}

/// Structural-only build: insert entities, create Module nodes, file-path hierarchy.
fn build_structural(graph: &mut rpg_core::graph::RPGraph, entities: Vec<rpg_core::graph::Entity>) {
    for entity in entities {
        graph.insert_entity(entity);
    }

    // Create Module entities for file-level nodes (paper §3.1)
    graph.create_module_entities();

    eprintln!("  Building file-path hierarchy (structural)...");
    graph.build_file_path_hierarchy();
}

fn cmd_build(
    project_root: &Path,
    lang: Option<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    force: bool,
) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};
    use rpg_parser::languages::Language;

    // Check if RPG already exists
    if rpg_core::storage::rpg_exists(project_root) && !force {
        anyhow::bail!(
            ".rpg/graph.json already exists. Use --force to rebuild, or `rpg-encoder update` for incremental changes."
        );
    }

    // Detect languages (multi-language support)
    let languages: Vec<Language> = if let Some(l) = lang {
        let lang = Language::from_name(&l)
            .or_else(|| Language::from_extension(&l))
            .ok_or_else(|| anyhow::anyhow!("unsupported language: {}", l))?;
        vec![lang]
    } else {
        let detected = Language::detect_all(project_root);
        if detected.is_empty() {
            let extensions = ["py", "rs", "ts", "js", "go", "java", "c", "cpp", "h"];
            anyhow::bail!(
                "No source files found in {}. Supported extensions: {}\nAre you in the right directory?",
                project_root.display(),
                extensions.join(", ")
            );
        }
        detected
    };

    let lang_names: Vec<&str> = languages.iter().map(|l| l.name()).collect();
    eprintln!("Detected language(s): {}", lang_names.join(", "));

    // Load config
    let config = RpgConfig::load(project_root)?;

    // Create graph (primary = first/most common language)
    let mut graph = rpg_core::graph::RPGraph::new(languages[0].name());
    graph.metadata.languages = languages.iter().map(|l| l.name().to_string()).collect();

    // Load TOML paradigm definitions + compile tree-sitter queries
    let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().map_err(|errs| {
        anyhow::anyhow!(
            "paradigm definition errors: {}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs)
        .map_err(|errs| anyhow::anyhow!("query compile errors: {}", errs.join("; ")))?;
    let active_defs =
        rpg_parser::paradigms::detect_paradigms_toml(project_root, &languages, &paradigm_defs);
    graph.metadata.paradigms = active_defs.iter().map(|d| d.name.clone()).collect();

    if !active_defs.is_empty() {
        let names: Vec<&str> = active_defs.iter().map(|d| d.name.as_str()).collect();
        eprintln!("  Paradigms detected: {}", names.join(", "));
    }

    // Collect and parse source files
    let files_to_parse = collect_source_files(project_root, &languages, &include, &exclude);
    let file_count = files_to_parse.len();

    let pb = ProgressBar::new(file_count as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  Parsing [{bar:30.cyan/blue}] {pos}/{len} files")
            .expect("valid template")
            .progress_chars("##-"),
    );

    // Parse all files in parallel with paradigm pipeline (classify/query/features)
    let all_raw_entities = if active_defs.is_empty() {
        rpg_parser::parse_files_parallel(files_to_parse)
    } else {
        rpg_parser::parse_files_with_paradigms(files_to_parse, &active_defs, &qcache)
    };
    pb.finish_and_clear();

    eprintln!(
        "  Parsed {} entities across {} files",
        all_raw_entities.len(),
        file_count
    );

    // Convert to graph entities
    let entities: Vec<rpg_core::graph::Entity> = all_raw_entities
        .iter()
        .map(|raw| raw.clone().into_entity())
        .collect();

    // Build structural graph
    build_structural(&mut graph, entities);

    // Hierarchy node enrichment
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();

    // Artifact Grounding
    eprintln!("  Artifact grounding...");
    let paradigm_ctx = rpg_encoder::grounding::ParadigmContext {
        active_defs,
        qcache: &qcache,
    };
    rpg_encoder::grounding::populate_entity_deps(
        &mut graph,
        project_root,
        config.encoding.broadcast_imports,
        None,
        Some(&paradigm_ctx),
    );
    rpg_encoder::grounding::ground_hierarchy(&mut graph);
    rpg_encoder::grounding::resolve_dependencies(&mut graph);

    // Set git commit if available
    if let Ok(sha) = rpg_encoder::evolution::get_head_sha(project_root) {
        graph.base_commit = Some(sha);
    }

    // Refresh metadata and save
    graph.refresh_metadata();
    rpg_core::storage::save_with_config(project_root, &graph, &config.storage)?;

    // Handle gitignore
    let _ = rpg_core::storage::ensure_gitignore(project_root);

    let (lifted, total) = graph.lifting_coverage();
    eprintln!("\nRPG built successfully!");
    eprintln!("  Entities: {}", graph.metadata.total_entities);
    eprintln!("  Files: {}", graph.metadata.total_files);
    eprintln!("  Lifted: {}/{}", lifted, total);
    eprintln!(
        "  Hierarchy: {}",
        if graph.metadata.semantic_hierarchy {
            "semantic"
        } else {
            "structural (file-path)"
        }
    );
    eprintln!("  Functional areas: {}", graph.metadata.functional_areas);
    eprintln!("  Dependency edges: {}", graph.metadata.dependency_edges);
    eprintln!("  Containment edges: {}", graph.metadata.containment_edges);
    eprintln!("  Total edges: {}", graph.metadata.total_edges);
    eprintln!("  Saved to: .rpg/graph.json");
    if total > 0 && lifted == 0 {
        eprintln!(
            "\nTip: Use the MCP server for semantic lifting (get_entities_for_lifting + submit_lift_results)."
        );
    }

    Ok(())
}

fn cmd_update(project_root: &Path, since: Option<String>) -> Result<()> {
    if !rpg_core::storage::rpg_exists(project_root) {
        anyhow::bail!("No RPG found. Run `rpg-encoder build` first.");
    }

    let mut graph = rpg_core::storage::load(project_root)?;
    let config = RpgConfig::load(project_root)?;

    // Detect paradigms for framework-aware entity classification
    // Backward compat: fall back to singular `language` field for older graphs
    let detected_langs: Vec<rpg_parser::languages::Language> =
        if !graph.metadata.languages.is_empty() {
            graph
                .metadata
                .languages
                .iter()
                .filter_map(|l| rpg_parser::languages::Language::from_name(l))
                .collect()
        } else {
            rpg_parser::languages::Language::from_name(&graph.metadata.language)
                .into_iter()
                .collect()
        };
    let paradigm_defs = rpg_parser::paradigms::defs::load_builtin_defs().map_err(|errs| {
        anyhow::anyhow!(
            "paradigm definition errors: {}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&paradigm_defs)
        .map_err(|errs| anyhow::anyhow!("query compile errors: {}", errs.join("; ")))?;
    let active_defs =
        rpg_parser::paradigms::detect_paradigms_toml(project_root, &detected_langs, &paradigm_defs);
    graph.metadata.paradigms = active_defs.iter().map(|d| d.name.clone()).collect();
    let paradigm_pipeline = rpg_encoder::evolution::ParadigmPipeline {
        active_defs,
        qcache: &qcache,
    };

    eprintln!("Running incremental update...");
    let summary = rpg_encoder::evolution::run_update(
        &mut graph,
        project_root,
        since.as_deref(),
        Some(&paradigm_pipeline),
    )?;

    rpg_core::storage::save_with_config(project_root, &graph, &config.storage)?;

    if summary.entities_added == 0
        && summary.entities_modified == 0
        && summary.entities_removed == 0
    {
        eprintln!("RPG is up to date. No source changes detected.");
    } else {
        eprintln!("RPG updated:");
        eprintln!("  Entities added: {}", summary.entities_added);
        eprintln!("  Entities modified: {}", summary.entities_modified);
        eprintln!("  Entities removed: {}", summary.entities_removed);
        eprintln!("  Edges added: {}", summary.edges_added);
        eprintln!("  Edges removed: {}", summary.edges_removed);
    }

    Ok(())
}

fn cmd_search(
    project_root: &Path,
    query: &str,
    mode: &str,
    scope: Option<&str>,
    line_range: Option<&str>,
    file_pattern: Option<&str>,
) -> Result<()> {
    let graph = rpg_core::storage::load(project_root)?;
    let config = RpgConfig::load(project_root)?;
    let search_mode = match mode {
        "features" => rpg_nav::search::SearchMode::Features,
        "snippets" => rpg_nav::search::SearchMode::Snippets,
        _ => rpg_nav::search::SearchMode::Auto,
    };

    let limit = config.navigation.search_result_limit;

    // Parse line range if provided
    let line_nums = line_range.and_then(|lr| {
        let parts: Vec<&str> = lr.split('-').collect();
        if parts.len() == 2 {
            let start = parts[0].parse::<usize>().ok()?;
            let end = parts[1].parse::<usize>().ok()?;
            Some((start, end))
        } else {
            None
        }
    });

    let results = rpg_nav::search::search_with_params(
        &graph,
        &rpg_nav::search::SearchParams {
            query,
            mode: search_mode,
            scope,
            limit,
            line_nums,
            file_pattern,
            entity_type_filter: None,
        },
    );

    if results.is_empty() {
        eprintln!("No results found for: {}", query);
        return Ok(());
    }

    for (i, result) in results.iter().enumerate() {
        println!(
            "{}. {} [{}:{}] (score: {:.2})",
            i + 1,
            result.entity_name,
            result.file,
            result.line_start,
            result.score
        );
        if !result.matched_features.is_empty() {
            println!("   features: {}", result.matched_features.join(", "));
        }
    }

    Ok(())
}

fn cmd_fetch(project_root: &Path, entity_id: &str) -> Result<()> {
    let graph = rpg_core::storage::load(project_root)?;
    let output = rpg_nav::fetch::fetch(&graph, entity_id, project_root)?;

    match output {
        rpg_nav::fetch::FetchOutput::Entity(result) => {
            println!("Entity: {}", result.entity.name);
            println!("Type: {:?}", result.entity.kind);
            println!(
                "File: {}:{}-{}",
                result.entity.file.display(),
                result.entity.line_start,
                result.entity.line_end
            );
            println!("Hierarchy: {}", result.entity.hierarchy_path);

            if !result.entity.semantic_features.is_empty() {
                println!("Features: {}", result.entity.semantic_features.join(", "));
            }

            if let Some(code) = &result.source_code {
                println!("\n--- Source ---\n{}", code);
            }

            if !result.entity.deps.invokes.is_empty() {
                println!("\nInvokes: {}", result.entity.deps.invokes.join(", "));
            }
            if !result.entity.deps.invoked_by.is_empty() {
                println!("Invoked by: {}", result.entity.deps.invoked_by.join(", "));
            }
        }
        rpg_nav::fetch::FetchOutput::Hierarchy(result) => {
            println!("Hierarchy Node: {}", result.node.name);
            println!("ID: {}", result.node.id);
            println!("Entities: {}", result.entity_count);
            if !result.node.grounded_paths.is_empty() {
                let paths: Vec<String> = result
                    .node
                    .grounded_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                println!("Grounded paths: {}", paths.join(", "));
            }
            if !result.child_names.is_empty() {
                println!("Children: {}", result.child_names.join(", "));
            }
            if !result.node.semantic_features.is_empty() {
                println!(
                    "Features: {}",
                    result
                        .node
                        .semantic_features
                        .iter()
                        .take(10)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }

    Ok(())
}

fn cmd_explore(project_root: &Path, entity_id: &str, direction: &str, depth: usize) -> Result<()> {
    let graph = rpg_core::storage::load(project_root)?;
    let dir = match direction {
        "up" | "upstream" => rpg_nav::explore::Direction::Upstream,
        "down" | "downstream" => rpg_nav::explore::Direction::Downstream,
        "both" => rpg_nav::explore::Direction::Both,
        _ => rpg_nav::explore::Direction::Downstream,
    };

    match rpg_nav::explore::explore(&graph, entity_id, dir, depth, None) {
        Some(tree) => {
            print!("{}", rpg_nav::explore::format_tree(&tree, 0));
        }
        None => {
            eprintln!("Entity not found: {}", entity_id);
        }
    }

    Ok(())
}

const PRECOMMIT_HOOK: &str = r#"#!/bin/sh
# RPG-Encoder: auto-update semantic graph before commit
# Installed by: rpg-encoder hook install
if [ -f ".rpg/graph.json" ]; then
    if command -v rpg-encoder >/dev/null 2>&1; then
        rpg-encoder update 2>&1 | while IFS= read -r line; do echo "  [rpg] $line"; done
        git add .rpg/graph.json 2>/dev/null
    elif command -v npx >/dev/null 2>&1; then
        npx -y rpg-encoder update 2>&1 | while IFS= read -r line; do echo "  [rpg] $line"; done
        git add .rpg/graph.json 2>/dev/null
    fi
fi
"#;

fn cmd_hook(project_root: &Path, action: &str) -> Result<()> {
    let git_dir = project_root.join(".git");
    if !git_dir.exists() {
        anyhow::bail!("Not a git repository. Run from a git project root.");
    }
    let hooks_dir = git_dir.join("hooks");
    let hook_path = hooks_dir.join("pre-commit");

    match action {
        "install" => {
            std::fs::create_dir_all(&hooks_dir)?;
            if hook_path.exists() {
                let existing = std::fs::read_to_string(&hook_path)?;
                if existing.contains("rpg-encoder") {
                    eprintln!("Pre-commit hook already installed.");
                    return Ok(());
                }
                // Append to existing hook
                let mut content = existing;
                content.push('\n');
                content.push_str(PRECOMMIT_HOOK);
                std::fs::write(&hook_path, content)?;
            } else {
                std::fs::write(&hook_path, PRECOMMIT_HOOK)?;
            }
            // Make executable (Unix)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
            }
            eprintln!("Pre-commit hook installed at .git/hooks/pre-commit");
            eprintln!("The RPG graph will auto-update and stage on every commit.");
        }
        "uninstall" => {
            if !hook_path.exists() {
                eprintln!("No pre-commit hook found.");
                return Ok(());
            }
            let content = std::fs::read_to_string(&hook_path)?;
            if !content.contains("rpg-encoder") {
                eprintln!("Pre-commit hook exists but was not installed by rpg-encoder.");
                return Ok(());
            }
            // If the hook is only our content, remove the file; otherwise strip our section
            let cleaned = content.replace(PRECOMMIT_HOOK, "");
            let cleaned = cleaned.trim();
            if cleaned.is_empty() || cleaned == "#!/bin/sh" {
                std::fs::remove_file(&hook_path)?;
            } else {
                std::fs::write(&hook_path, cleaned)?;
            }
            eprintln!("Pre-commit hook uninstalled.");
        }
        _ => anyhow::bail!("Unknown action: {}. Use 'install' or 'uninstall'.", action),
    }
    Ok(())
}

fn cmd_export(project_root: &Path, format: &str) -> Result<()> {
    if !rpg_core::storage::rpg_exists(project_root) {
        anyhow::bail!("No RPG found. Run `rpg-encoder build` first.");
    }

    let graph = rpg_core::storage::load(project_root)?;

    let export_format = match format {
        "dot" | "graphviz" => rpg_nav::export::ExportFormat::Dot,
        "mermaid" | "md" => rpg_nav::export::ExportFormat::Mermaid,
        _ => anyhow::bail!("Unknown export format: {}. Use 'dot' or 'mermaid'.", format),
    };

    let output = rpg_nav::export::export(&graph, export_format);
    print!("{}", output);

    Ok(())
}

fn cmd_diff(project_root: &Path, since: Option<String>) -> Result<()> {
    use rpg_encoder::evolution::FileChange;

    if !rpg_core::storage::rpg_exists(project_root) {
        anyhow::bail!("No RPG found. Run `rpg-encoder build` first.");
    }

    let graph = rpg_core::storage::load(project_root)?;

    let changes = rpg_encoder::evolution::detect_changes(project_root, &graph, since.as_deref())?;
    let changes = rpg_encoder::evolution::filter_rpgignore_changes(project_root, changes);

    if changes.is_empty() {
        eprintln!("No changes detected since last build.");
        return Ok(());
    }

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut renamed = Vec::new();

    for change in &changes {
        match change {
            FileChange::Added(p) => added.push(p),
            FileChange::Modified(p) => modified.push(p),
            FileChange::Deleted(p) => deleted.push(p),
            FileChange::Renamed { from, to } => renamed.push((from, to)),
        }
    }

    if !added.is_empty() {
        println!("+{} added file(s):", added.len());
        for f in &added {
            println!("  + {}", f.display());
        }
    }
    if !modified.is_empty() {
        println!("~{} modified file(s):", modified.len());
        for f in &modified {
            println!("  ~ {}", f.display());
        }
    }
    if !deleted.is_empty() {
        println!("-{} deleted file(s):", deleted.len());
        for f in &deleted {
            println!("  - {}", f.display());
        }
    }
    if !renamed.is_empty() {
        println!(">{} renamed file(s):", renamed.len());
        for (from, to) in &renamed {
            println!("  {} -> {}", from.display(), to.display());
        }
    }

    println!(
        "\nSummary: +{} added, ~{} modified, -{} deleted, >{} renamed",
        added.len(),
        modified.len(),
        deleted.len(),
        renamed.len()
    );
    eprintln!("Run `rpg-encoder update` to apply these changes.");

    Ok(())
}

fn cmd_validate(project_root: &Path) -> Result<()> {
    if !rpg_core::storage::rpg_exists(project_root) {
        anyhow::bail!("No RPG found. Run `rpg-encoder build` first.");
    }

    let graph = rpg_core::storage::load(project_root)?;
    let mut issues = 0;

    // 1. Dangling edge targets (edge references entity ID not in entities or hierarchy)
    for edge in &graph.edges {
        let source_exists = graph.entities.contains_key(&edge.source)
            || graph.find_hierarchy_node_by_id(&edge.source).is_some();
        let target_exists = graph.entities.contains_key(&edge.target)
            || graph.find_hierarchy_node_by_id(&edge.target).is_some();

        if !source_exists {
            println!("WARN: dangling edge source: {}", edge.source);
            issues += 1;
        }
        if !target_exists {
            println!("WARN: dangling edge target: {}", edge.target);
            issues += 1;
        }
    }

    // 2. Orphan entity references in hierarchy
    for (area_name, node) in &graph.hierarchy {
        check_hierarchy_orphans(node, area_name, &graph, &mut issues);
    }

    // 3. Entity IDs not matching file:name format
    for (id, entity) in &graph.entities {
        if entity.kind != rpg_core::graph::EntityKind::Module && !id.contains(':') {
            println!("WARN: entity ID missing file:name format: {}", id);
            issues += 1;
        }
    }

    // 4. File index consistency
    for (file, ids) in &graph.file_index {
        for id in ids {
            if !graph.entities.contains_key(id) {
                println!(
                    "WARN: file_index references missing entity: {} in {}",
                    id,
                    file.display()
                );
                issues += 1;
            }
        }
    }

    if issues == 0 {
        eprintln!("Graph is valid. No integrity issues found.");
        eprintln!(
            "  {} entities, {} edges, {} files",
            graph.entities.len(),
            graph.edges.len(),
            graph.file_index.len()
        );
    } else {
        eprintln!("\nFound {} integrity issue(s).", issues);
    }

    Ok(())
}

fn check_hierarchy_orphans(
    node: &rpg_core::graph::HierarchyNode,
    path: &str,
    graph: &rpg_core::graph::RPGraph,
    issues: &mut usize,
) {
    for entity_id in &node.entities {
        if !graph.entities.contains_key(entity_id) {
            println!(
                "WARN: hierarchy node '{}' references missing entity: {}",
                path, entity_id
            );
            *issues += 1;
        }
    }
    for (child_name, child) in &node.children {
        let child_path = format!("{}/{}", path, child_name);
        check_hierarchy_orphans(child, &child_path, graph, issues);
    }
}

fn cmd_info(project_root: &Path) -> Result<()> {
    if !rpg_core::storage::rpg_exists(project_root) {
        eprintln!("No RPG found. Run `rpg-encoder build` first.");
        return Ok(());
    }

    let graph = rpg_core::storage::load(project_root)?;

    println!("RPG v{}", graph.version);
    println!("Language: {}", graph.metadata.language);
    println!("Created: {}", graph.created_at);
    println!("Updated: {}", graph.updated_at);
    if let Some(sha) = &graph.base_commit {
        println!("Base commit: {}", &sha[..8.min(sha.len())]);
    }
    println!();
    println!("Entities: {}", graph.metadata.total_entities);
    println!("Files: {}", graph.metadata.total_files);
    let (lifted, total) = graph.lifting_coverage();
    println!("Lifted: {}/{}", lifted, total);
    println!(
        "Hierarchy: {}",
        if graph.metadata.semantic_hierarchy {
            "semantic"
        } else {
            "structural (file-path)"
        }
    );
    println!("Functional areas: {}", graph.metadata.functional_areas);
    println!("Dependency edges: {}", graph.metadata.dependency_edges);
    println!("Containment edges: {}", graph.metadata.containment_edges);
    println!("Total edges: {}", graph.metadata.total_edges);
    if let Some(summary) = &graph.metadata.repo_summary {
        println!("\nSummary: {}", summary);
    }

    if !graph.hierarchy.is_empty() {
        println!("\nHierarchy:");
        for (name, area) in &graph.hierarchy {
            println!("  {} ({} entities)", name, area.entity_count());
            for (cat_name, cat) in &area.children {
                println!("    {} ({} entities)", cat_name, cat.entity_count());
            }
        }
    }

    Ok(())
}
