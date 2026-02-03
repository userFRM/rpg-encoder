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

        /// Generate embeddings for semantic search
        #[arg(long)]
        embed: bool,

        /// Perform full LLM semantic lifting during build (slow for large projects)
        #[arg(long)]
        lift: bool,
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

        /// Search mode: features, snippets, auto, semantic, hybrid
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

    /// Semantically lift entities in a specific area (on-demand)
    Lift {
        /// Scope: file glob (e.g., "src/auth/**"), hierarchy path, entity IDs, or "all"
        scope: String,
    },

    /// Show RPG statistics
    Info,

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

#[tokio::main]
async fn main() -> Result<()> {
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
            embed,
            lift,
        } => cmd_build(&project_root, lang, include, exclude, embed, lift).await,
        Commands::Update { since } => cmd_update(&project_root, since).await,
        Commands::Search {
            query,
            mode,
            scope,
            line_range,
            file_pattern,
        } => {
            cmd_search(
                &project_root,
                &query,
                &mode,
                scope.as_deref(),
                line_range.as_deref(),
                file_pattern.as_deref(),
            )
            .await
        }
        Commands::Lift { scope } => cmd_lift(&project_root, &scope).await,
        Commands::Fetch { entity_id } => cmd_fetch(&project_root, &entity_id),
        Commands::Explore {
            entity_id,
            direction,
            depth,
        } => cmd_explore(&project_root, &entity_id, &direction, depth),
        Commands::Info => cmd_info(&project_root),
        Commands::Serve => {
            eprintln!("MCP server not yet implemented. Use rpg-mcp binary instead.");
            Ok(())
        }
    }
}

async fn cmd_build(
    project_root: &PathBuf,
    lang: Option<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    embed: bool,
    lift: bool,
) -> Result<()> {
    use rpg_parser::languages::Language;

    // Check if RPG already exists
    if rpg_core::storage::rpg_exists(project_root) {
        eprintln!("Warning: .rpg/graph.json already exists. Rebuilding from scratch.");
    }

    // Detect language
    let language = if let Some(l) = lang {
        Language::from_name(&l)
            .or_else(|| Language::from_extension(&l))
            .ok_or_else(|| anyhow::anyhow!("unsupported language: {}", l))?
    } else {
        Language::detect_primary(project_root)
            .ok_or_else(|| anyhow::anyhow!("could not detect language; specify with --lang"))?
    };

    eprintln!("Detected language: {}", language.name());

    // Load config
    let config = RpgConfig::load(project_root)?;

    // Create graph
    let mut graph = rpg_core::graph::RPGraph::new(language.name());

    // Parse code entities
    eprintln!("  Parsing code entities...");
    let include_set = if include.is_empty() {
        None
    } else {
        let mut builder = globset::GlobSetBuilder::new();
        for p in &include {
            builder.add(globset::Glob::new(p).expect("invalid --include glob"));
        }
        Some(builder.build().expect("invalid --include glob set"))
    };
    let exclude_set = if exclude.is_empty() {
        None
    } else {
        let mut builder = globset::GlobSetBuilder::new();
        for p in &exclude {
            builder.add(globset::Glob::new(p).expect("invalid --exclude glob"));
        }
        Some(builder.build().expect("invalid --exclude glob set"))
    };

    let walker = ignore::WalkBuilder::new(project_root)
        .hidden(true)
        .git_ignore(true)
        .build();

    let mut all_raw_entities = Vec::new();
    let mut file_count = 0;

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if Language::from_extension(ext) != Some(language) {
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

        let source = std::fs::read_to_string(path)?;
        let rel_path = path.strip_prefix(project_root).unwrap_or(path);

        let entities = rpg_parser::entities::extract_entities(rel_path, &source, language);

        all_raw_entities.extend(entities);
        file_count += 1;
    }

    eprintln!(
        "  Found {} entities across {} files",
        all_raw_entities.len(),
        file_count
    );

    // Convert to graph entities and insert into graph
    let mut entities: Vec<rpg_core::graph::Entity> = all_raw_entities
        .iter()
        .map(|raw| raw.clone().into_entity())
        .collect();

    if lift {
        // --lift: Full LLM semantic lifting (old behavior, slow for large projects)
        eprintln!("  Semantic lifting via LLM...");
        match rpg_encoder::llm::LlmClient::from_env_with_config_async(&config.llm).await {
            Ok(client) => {
                eprintln!(
                    "  Using LLM provider: {} ({})",
                    client.provider_name(),
                    client.model_name()
                );
                let batch_ranges = rpg_encoder::lift::build_token_aware_batches(
                    &all_raw_entities,
                    config.encoding.batch_size,
                    config.encoding.max_batch_tokens,
                );
                let repo_name = project_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let total_batches = batch_ranges.len();

                for (batch_idx, range) in batch_ranges.iter().enumerate() {
                    let batch = &all_raw_entities[range.0..range.1];
                    eprintln!(
                        "  Processing batch {}/{} ({} entities)...",
                        batch_idx + 1,
                        total_batches,
                        batch.len()
                    );

                    match rpg_encoder::semantic_lifting::lift_batch(
                        &client, batch, repo_name, "", None,
                    )
                    .await
                    {
                        Ok(features) => {
                            rpg_encoder::semantic_lifting::apply_features(&mut entities, &features);
                        }
                        Err(e) => {
                            eprintln!("  Warning: LLM batch {} failed: {}", batch_idx + 1, e);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("  Skipping LLM semantic lifting: {}", e);
            }
        }

        // Insert entities into graph
        for entity in entities {
            graph.insert_entity(entity);
        }

        // Structure Reorganization (LLM-driven)
        eprintln!("  Building semantic hierarchy...");
        match rpg_encoder::llm::LlmClient::from_env_with_config_async(&config.llm).await {
            Ok(client) => {
                let repo_name = project_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let entities_vec: Vec<_> = graph.entities.values().cloned().collect();

                match rpg_encoder::hierarchy::discover_domains(&client, &entities_vec, repo_name)
                    .await
                {
                    Ok(areas) => {
                        eprintln!("  Discovered {} functional areas: {:?}", areas.len(), areas);
                        match rpg_encoder::hierarchy::build_hierarchy(
                            &client,
                            &entities_vec,
                            &areas,
                            repo_name,
                            config.encoding.hierarchy_chunk_size,
                            config.encoding.hierarchy_concurrency,
                        )
                        .await
                        {
                            Ok(assignments) => {
                                rpg_encoder::hierarchy::apply_hierarchy(&mut graph, &assignments);
                                graph.metadata.semantic_hierarchy = true;
                                eprintln!("  Assigned {} entities to hierarchy", assignments.len());

                                // Generate repo-level summary
                                let repo_name = project_root
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("unknown");
                                match rpg_encoder::hierarchy::generate_repo_summary(
                                    &client, &graph, repo_name,
                                )
                                .await
                                {
                                    Ok(summary) => {
                                        eprintln!("  Generated repo summary");
                                        graph.metadata.repo_summary = Some(summary);
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "  Warning: repo summary generation failed: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("  Warning: hierarchy construction failed: {}", e);
                            }
                        }
                    }
                    Err(e) => eprintln!("  Warning: domain discovery failed: {}", e),
                }
            }
            Err(_) => {
                eprintln!("  Skipping (no LLM provider available).");
            }
        }
    } else {
        // Default: structural-only build (fast, no LLM)
        for entity in entities {
            graph.insert_entity(entity);
        }

        // Create Module entities for file-level nodes (paper §3.1)
        graph.create_module_entities();

        eprintln!("  Building file-path hierarchy (structural)...");
        graph.build_file_path_hierarchy();
    }

    // Hierarchy node enrichment
    graph.assign_hierarchy_ids();
    graph.aggregate_hierarchy_features();
    graph.materialize_containment_edges();

    // Artifact Grounding
    eprintln!("  Artifact grounding...");
    rpg_encoder::grounding::populate_entity_deps(&mut graph, project_root, language);
    rpg_encoder::grounding::ground_hierarchy(&mut graph);
    rpg_encoder::grounding::resolve_dependencies(&mut graph);

    // Set git commit if available
    if let Ok(sha) = rpg_encoder::evolution::get_head_sha(project_root) {
        graph.base_commit = Some(sha);
    }

    // Optional: Generate embeddings
    if embed {
        eprintln!("Phase 4: Generating embeddings...");
        match rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings) {
            Ok(generator) => {
                eprintln!("  Using embedding provider: {}", generator.provider_name());
                match generator
                    .embed_entities(&mut graph, config.embeddings.batch_size)
                    .await
                {
                    Ok(count) => eprintln!("  Generated embeddings for {} entities", count),
                    Err(e) => eprintln!("  Warning: embedding generation failed: {}", e),
                }
            }
            Err(e) => {
                eprintln!("  Skipping embeddings: {}", e);
            }
        }
    }

    // Refresh metadata and save
    graph.refresh_metadata();
    rpg_core::storage::save(project_root, &graph)?;

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
            "semantic (LLM)"
        } else {
            "structural (file-path)"
        }
    );
    eprintln!("  Functional areas: {}", graph.metadata.functional_areas);
    eprintln!("  Dependency edges: {}", graph.metadata.dependency_edges);
    eprintln!("  Containment edges: {}", graph.metadata.containment_edges);
    eprintln!("  Total edges: {}", graph.metadata.total_edges);
    if embed {
        let emb_count = rpg_nav::embedding_search::embedding_count(&graph);
        eprintln!("  Embeddings: {}", emb_count);
    }
    eprintln!("  Saved to: .rpg/graph.json");
    if !lift && total > 0 && lifted == 0 {
        eprintln!(
            "\nTip: Use `rpg-encoder lift \"src/**\"` to progressively add semantic features."
        );
        eprintln!("     Or use `rpg-encoder build --lift` for full semantic lifting.");
    }

    Ok(())
}

async fn cmd_update(project_root: &Path, since: Option<String>) -> Result<()> {
    if !rpg_core::storage::rpg_exists(project_root) {
        anyhow::bail!("No RPG found. Run `rpg-encoder build` first.");
    }

    let mut graph = rpg_core::storage::load(project_root)?;
    let config = RpgConfig::load(project_root)?;

    let client = rpg_encoder::llm::LlmClient::from_env_with_config_async(&config.llm)
        .await
        .ok();
    if let Some(ref c) = client {
        eprintln!(
            "  Using LLM provider: {} ({})",
            c.provider_name(),
            c.model_name()
        );
    } else {
        eprintln!("Note: No LLM provider available. Updates will skip semantic lifting.");
        eprintln!("  Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or install Ollama to enable.");
    }

    let embedder =
        rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings).ok();

    eprintln!("Running incremental update...");
    let summary = rpg_encoder::evolution::run_update(
        &mut graph,
        project_root,
        client.as_ref(),
        since.as_deref(),
        config.encoding.drift_threshold,
        config.encoding.hierarchy_chunk_size,
        embedder.as_ref(),
    )
    .await?;

    rpg_core::storage::save(project_root, &graph)?;

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

async fn cmd_search(
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
        "semantic" => rpg_nav::search::SearchMode::Semantic,
        "hybrid" => rpg_nav::search::SearchMode::Hybrid,
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

    // For semantic/hybrid modes, try to generate query embeddings
    let results = match mode {
        "semantic" | "hybrid" if rpg_nav::embedding_search::has_embeddings(&graph) => {
            match rpg_encoder::embeddings::EmbeddingGenerator::from_config(&config.embeddings) {
                Ok(generator) => match generator.generate_single(query).await {
                    Ok(emb) => rpg_nav::search::search_with_params(
                        &graph,
                        &rpg_nav::search::SearchParams {
                            query,
                            mode: search_mode,
                            scope,
                            limit,
                            line_nums,
                            file_pattern,
                            query_embedding: Some(&emb),
                            semantic_weight: config.embeddings.semantic_weight,
                            entity_type_filter: None,
                        },
                    ),
                    Err(e) => {
                        eprintln!(
                            "Warning: embedding generation failed: {}. Falling back to keyword search.",
                            e
                        );
                        rpg_nav::search::search_with_params(
                            &graph,
                            &rpg_nav::search::SearchParams {
                                query,
                                mode: search_mode,
                                scope,
                                limit,
                                line_nums,
                                file_pattern,
                                query_embedding: None,
                                semantic_weight: 0.5,
                                entity_type_filter: None,
                            },
                        )
                    }
                },
                Err(e) => {
                    eprintln!(
                        "Warning: no embedding API key: {}. Falling back to keyword search.",
                        e
                    );
                    rpg_nav::search::search_with_params(
                        &graph,
                        &rpg_nav::search::SearchParams {
                            query,
                            mode: search_mode,
                            scope,
                            limit,
                            line_nums,
                            file_pattern,
                            query_embedding: None,
                            semantic_weight: 0.5,
                            entity_type_filter: None,
                        },
                    )
                }
            }
        }
        _ => rpg_nav::search::search_with_params(
            &graph,
            &rpg_nav::search::SearchParams {
                query,
                mode: search_mode,
                scope,
                limit,
                line_nums,
                file_pattern,
                query_embedding: None,
                semantic_weight: 0.5,
                entity_type_filter: None,
            },
        ),
    };

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

async fn cmd_lift(project_root: &Path, scope: &str) -> Result<()> {
    if !rpg_core::storage::rpg_exists(project_root) {
        anyhow::bail!("No RPG found. Run `rpg-encoder build` first.");
    }

    let mut graph = rpg_core::storage::load(project_root)?;
    let config = RpgConfig::load(project_root)?;

    let resolved = rpg_encoder::lift::resolve_scope(&graph, scope);
    if resolved.entity_ids.is_empty() {
        eprintln!("No entities matched scope: {}", scope);
        return Ok(());
    }

    eprintln!(
        "Lifting {} entities matching '{}'...",
        resolved.entity_ids.len(),
        scope
    );

    let client = rpg_encoder::llm::LlmClient::from_env_with_config_async(&config.llm).await?;
    eprintln!(
        "  Using LLM provider: {} ({})",
        client.provider_name(),
        client.model_name()
    );

    let result =
        rpg_encoder::lift::lift_area(&mut graph, &resolved, &client, project_root, &config).await?;

    rpg_core::storage::save(project_root, &graph)?;

    let (lifted, total) = graph.lifting_coverage();
    eprintln!("\nLifting complete:");
    eprintln!("  Entities lifted: {}", result.entities_lifted);
    if result.entities_repaired > 0 {
        eprintln!("  Entities repaired: {}", result.entities_repaired);
    }
    if result.entities_failed > 0 {
        eprintln!("  Entities failed: {}", result.entities_failed);
    }
    if result.hierarchy_updated {
        eprintln!("  Hierarchy: updated to semantic");
    }
    eprintln!("  Total coverage: {}/{}", lifted, total);

    Ok(())
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
            "semantic (LLM)"
        } else {
            "structural (file-path)"
        }
    );
    println!("Functional areas: {}", graph.metadata.functional_areas);
    println!("Dependency edges: {}", graph.metadata.dependency_edges);
    println!("Containment edges: {}", graph.metadata.containment_edges);
    println!("Total edges: {}", graph.metadata.total_edges);
    let emb_count = rpg_nav::embedding_search::embedding_count(&graph);
    if emb_count > 0 {
        println!("Embeddings: {}", emb_count);
    }

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
