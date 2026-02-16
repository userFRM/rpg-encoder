//! Generation workflow helpers — business logic for code generation tools.
//!
//! These functions are called by the #[tool] handlers in tools.rs to keep
//! the tool implementations thin and maintainable.

use chrono::Utc;
use rpg_core::storage;
use rpg_gen::{
    EntitySkeleton, EntitySkeletonKind, FeatureTree, FileSkeletonSet, GenerationPlan,
    GenerationState, GenerationTask, InterfaceDesign, PlannedTask, TaskGraph, TaskKind, TaskStatus,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::types::GenerationSession;

/// Token budget per batch for interface design and task generation.
const BATCH_TOKEN_BUDGET: usize = 8000;

/// Outcome from executing a test/runtime command.
struct CommandExecution {
    command: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    duration_ms: u64,
}

/// Build candidate commands for a simple automatic test adaptation loop.
fn adaptive_test_commands(base: &str, max_auto_adapt: usize) -> Vec<String> {
    let mut commands = vec![base.to_string()];
    if max_auto_adapt == 0 {
        return commands;
    }

    if base.starts_with("cargo test") && !base.contains("--nocapture") {
        commands.push(format!("{base} -- --nocapture"));
    }
    if base.contains("pytest") && !base.contains(" -q") {
        commands.push(format!("{base} -q"));
    }
    if base.contains("npm test") && !base.contains("--runInBand") {
        commands.push(format!("{base} -- --runInBand"));
    }
    if base.starts_with("go test") && !base.contains("-count=1") {
        commands.push(format!("{base} -count=1"));
    }

    commands.truncate(max_auto_adapt + 1);
    commands
}

/// Execute a command under a selected sandbox mode.
fn run_command(
    project_root: &Path,
    command: &str,
    working_dir: Option<&str>,
    sandbox_mode: rpg_gen::SandboxMode,
    docker_image: Option<&str>,
) -> Result<CommandExecution, String> {
    let start = Instant::now();
    let mut cmd = match sandbox_mode {
        rpg_gen::SandboxMode::None => {
            return Ok(CommandExecution {
                command: command.to_string(),
                stdout: String::new(),
                stderr: "sandbox_mode=none: command execution skipped".to_string(),
                exit_code: Some(127),
                duration_ms: 0,
            });
        }
        rpg_gen::SandboxMode::Local => {
            let mut c = Command::new("sh");
            c.arg("-lc").arg(command);
            c
        }
        rpg_gen::SandboxMode::Docker => {
            let image = docker_image.ok_or_else(|| {
                "docker_image is required for sandbox_mode=docker (set params.docker_image or .rpg/config.toml [generation.docker_images]).".to_string()
            })?;
            let workspace = project_root.display().to_string();
            let workdir = if let Some(wd) = working_dir {
                format!("/workspace/{}", wd.trim_start_matches('/'))
            } else {
                "/workspace".to_string()
            };

            let mut c = Command::new("docker");
            c.arg("run")
                .arg("--rm")
                .arg("-v")
                .arg(format!("{workspace}:/workspace"))
                .arg("-w")
                .arg(workdir)
                .arg(image)
                .arg("sh")
                .arg("-lc")
                .arg(command);
            c
        }
    };

    if !matches!(sandbox_mode, rpg_gen::SandboxMode::Docker) {
        let dir = if let Some(wd) = working_dir {
            project_root.join(wd)
        } else {
            project_root.to_path_buf()
        };
        cmd.current_dir(dir);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("failed to execute command '{}': {}", command, e))?;

    Ok(CommandExecution {
        command: command.to_string(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Determine a coarse test runner family from the command line.
fn detect_runner(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.starts_with("cargo test") {
        Some("cargo".into())
    } else if trimmed.starts_with("pytest") || trimmed.contains(" pytest") {
        Some("pytest".into())
    } else if trimmed.contains("jest") {
        Some("jest".into())
    } else if trimmed.starts_with("go test") {
        Some("go test".into())
    } else {
        None
    }
}

/// Estimate model cost in USD from token counts.
///
/// Rates are intentionally approximate and used for relative efficiency trends.
fn estimate_cost_usd(model: Option<&str>, prompt_tokens: usize, completion_tokens: usize) -> f64 {
    let name = model.unwrap_or("").to_lowercase();
    let (in_per_million, out_per_million) = if name.contains("gpt-5") {
        (1.25, 5.0)
    } else if name.contains("gpt-4.1") {
        (5.0, 15.0)
    } else if name.contains("claude") {
        (3.0, 15.0)
    } else if name.contains("gemini") {
        (1.5, 6.0)
    } else {
        (1.0, 3.0)
    };

    (prompt_tokens as f64 / 1_000_000.0) * in_per_million
        + (completion_tokens as f64 / 1_000_000.0) * out_per_million
}

fn parse_first_usize_before(haystack: &str, needle: &str) -> Option<usize> {
    let idx = haystack.find(needle)?;
    let prefix = &haystack[..idx];
    let digits_rev: String = prefix
        .chars()
        .rev()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits_rev.is_empty() {
        return None;
    }
    digits_rev.chars().rev().collect::<String>().parse().ok()
}

/// Parse test summary counts from common test runner output.
fn parse_test_results(output: &str) -> Option<rpg_gen::TestResults> {
    let mut total = None;
    let mut passed = None;
    let mut failed = None;

    for line in output.lines() {
        if line.contains("test result:") && line.contains("passed;") {
            passed = parse_first_usize_before(line, "passed;");
            failed = parse_first_usize_before(line, "failed;");
        }
        if line.contains(" failed") && line.contains(" passed") {
            // pytest style: "2 failed, 3 passed in ..."
            if failed.is_none() {
                failed = parse_first_usize_before(line, "failed");
            }
            if passed.is_none() {
                passed = parse_first_usize_before(line, "passed");
            }
        }
    }

    if let (Some(p), Some(f)) = (passed, failed) {
        total = Some(p + f);
    }

    total.map(|t| rpg_gen::TestResults {
        total: t,
        passed: passed.unwrap_or(0),
        failed: failed.unwrap_or(0),
        test_file: None,
    })
}

/// Classify command output into a TDD iteration outcome.
fn classify_execution_outcome(execution: &CommandExecution) -> rpg_gen::OutcomeKind {
    if execution.exit_code == Some(0) {
        return rpg_gen::OutcomeKind::Pass;
    }

    let combined = format!("{}\n{}", execution.stdout, execution.stderr).to_lowercase();
    if combined.contains("permission denied")
        || combined.contains("command not found")
        || combined.contains("no such file or directory")
        || combined.contains("docker: ")
        || combined.contains("execution skipped")
    {
        return rpg_gen::OutcomeKind::EnvError {
            error_message: first_error_line(&combined),
        };
    }

    if combined.contains("syntaxerror")
        || combined.contains("cannot find")
        || combined.contains("failed to compile")
        || combined.contains("error[e")
        || combined.contains("compilation failed")
    {
        return rpg_gen::OutcomeKind::CodeError {
            error_message: first_error_line(&combined),
        };
    }

    if combined.contains("fixture")
        || combined.contains("importerror")
        || combined.contains("module not found in tests")
    {
        return rpg_gen::OutcomeKind::TestError {
            error_message: first_error_line(&combined),
        };
    }

    let failing_count = parse_test_results(&combined).map_or(1, |r| r.failed.max(1));
    rpg_gen::OutcomeKind::TestFailure {
        failing_count,
        summary: first_error_line(&combined),
    }
}

fn first_error_line(text: &str) -> String {
    text.lines()
        .find(|l| {
            l.contains("error")
                || l.contains("failed")
                || l.contains("panic")
                || l.contains("traceback")
        })
        .map_or_else(|| "execution failed".to_string(), |s| s.trim().to_string())
}

/// Check that the submitted revision matches the plan revision.
/// Returns an error if they don't match (stale submission).
fn check_revision(plan: &GenerationPlan, submitted: Option<&str>) -> Result<(), String> {
    if let Some(rev) = submitted
        && rev != plan.revision
    {
        return Err(format!(
            "Stale revision. Expected '{}', got '{}'. \
             The plan was modified since your last read. \
             Call generation_status to get the current revision.",
            plan.revision, rev
        ));
    }
    // If no revision submitted, skip check (backward compatibility / convenience)
    Ok(())
}

/// Initialize a new generation session.
pub fn init_generation(
    project_root: &Path,
    spec: String,
    language: String,
    _reference_repo: Option<String>,
) -> Result<(GenerationPlan, String), String> {
    // Check if a session already exists
    if storage::generation_plan_exists(project_root) {
        return Err(
            "Generation session already exists. Call reset_generation to start fresh, or continue with get_feature_tree.".into()
        );
    }

    let plan = GenerationPlan::new(project_root.to_path_buf(), spec.clone(), language.clone());

    // Save to disk
    let plan_json = serde_json::to_string_pretty(&plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    // Build response with next action
    let output = format!(
        "## GENERATION INITIALIZED\n\n\
        spec: {} ({} chars)\n\
        language: {}\n\
        revision: {}\n\n\
        ## NEXT_ACTION\n\n\
        Decompose the specification into features using the spec_decomposition prompt.\n\
        Then call `submit_feature_tree` with the JSON result.\n\n\
        ### Spec Decomposition Prompt\n\n\
        {}\n",
        &spec[..spec.len().min(100)],
        spec.len(),
        language,
        plan.revision,
        rpg_gen::SPEC_DECOMPOSITION_PROMPT,
    );

    Ok((plan, output))
}

/// Get the current feature tree for review/editing.
pub fn get_feature_tree(plan: &GenerationPlan) -> Result<String, String> {
    match &plan.state {
        GenerationState::Initializing => {
            Err("No feature tree yet. Call init_generation first, then submit_feature_tree.".into())
        }
        GenerationState::Planning { tasks } => {
            // Convert tasks back to a displayable format
            let mut output = format!("## FEATURE TREE (from {} planned tasks)\n\n", tasks.len());

            // Group by file path as a proxy for area
            let mut by_area: HashMap<String, Vec<&PlannedTask>> = HashMap::new();
            for task in tasks {
                let area = task
                    .file_path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "root".to_string());
                by_area.entry(area).or_default().push(task);
            }

            for (area, area_tasks) in &by_area {
                output.push_str(&format!("### {}\n", area));
                for task in area_tasks {
                    output.push_str(&format!(
                        "- `{}` ({:?}): {}\n",
                        task.id,
                        task.kind,
                        task.semantic_features.join(", ")
                    ));
                }
                output.push('\n');
            }

            output.push_str("## NEXT_ACTION\n\n");
            output.push_str("Review the feature tree. If satisfied, call `get_interfaces_for_design(batch_index=0)` to start interface design.\n");

            Ok(output)
        }
        GenerationState::DesigningInterfaces { feature_tree, .. } => {
            // Show the feature tree
            let json = serde_json::to_string_pretty(feature_tree)
                .map_err(|e| format!("Failed to serialize feature tree: {}", e))?;
            Ok(format!(
                "## FEATURE TREE\n\n```json\n{}\n```\n\n## NEXT_ACTION\n\nCall `get_interfaces_for_design(batch_index=0)` to continue interface design.\n",
                json
            ))
        }
        _ => Err(format!(
            "Cannot get feature tree in {} phase. Use generation_status to see current state.",
            plan.phase()
        )),
    }
}

/// Submit the decomposed feature tree.
pub fn submit_feature_tree(
    project_root: &Path,
    plan: &mut GenerationPlan,
    features_json: &str,
    revision: Option<&str>,
) -> Result<String, String> {
    // Check revision for staleness
    check_revision(plan, revision)?;

    // Parse the feature tree
    let feature_tree: FeatureTree = serde_json::from_str(features_json)
        .map_err(|e| format!("Invalid feature tree JSON: {}", e))?;

    // Validate
    if feature_tree.functional_areas.is_empty() {
        return Err("Feature tree must have at least one functional area.".into());
    }

    // Count features
    let total_features: usize = feature_tree
        .functional_areas
        .values()
        .map(|a| a.features.len())
        .sum();

    // Transition to DesigningInterfaces state
    plan.start_interface_design(feature_tree.clone());
    plan.stats.total_features = total_features;

    // Save
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    Ok(format!(
        "## FEATURE TREE SUBMITTED\n\n\
        areas: {}\n\
        features: {}\n\
        revision: {}\n\n\
        ## NEXT_ACTION\n\n\
        Call `get_interfaces_for_design(batch_index=0)` to start interface design.\n",
        feature_tree.functional_areas.len(),
        total_features,
        plan.revision,
    ))
}

/// Build token-aware batches for interface design.
pub fn build_interface_batches(feature_tree: &FeatureTree) -> Vec<(usize, usize)> {
    let areas: Vec<_> = feature_tree.functional_areas.keys().collect();
    let mut batches = Vec::new();
    let mut current_start = 0;
    let mut current_tokens = 0;

    for (i, area_name) in areas.iter().enumerate() {
        let area = &feature_tree.functional_areas[*area_name];
        // Estimate tokens: ~50 per feature (conservative)
        let area_tokens = area.features.len() * 50 + 100;

        if current_tokens + area_tokens > BATCH_TOKEN_BUDGET && current_start < i {
            batches.push((current_start, i));
            current_start = i;
            current_tokens = 0;
        }
        current_tokens += area_tokens;
    }

    if current_start < areas.len() {
        batches.push((current_start, areas.len()));
    }

    batches
}

/// Get a batch of features for interface design.
pub fn get_interfaces_for_design(
    plan: &GenerationPlan,
    session: &GenerationSession,
    batch_index: usize,
) -> Result<String, String> {
    let GenerationState::DesigningInterfaces {
        feature_tree,
        interface_design,
    } = &plan.state
    else {
        return Err(format!(
            "Cannot get interfaces in {} phase. Submit feature tree first.",
            plan.phase()
        ));
    };

    let batches = &session.interface_batches;
    if batch_index >= batches.len() {
        return Err(format!(
            "Batch index {} out of range. {} batches available.",
            batch_index,
            batches.len()
        ));
    }

    let (start, end) = batches[batch_index];
    let area_names: Vec<_> = feature_tree.functional_areas.keys().collect();
    let batch_areas: Vec<_> = area_names[start..end].to_vec();

    let mut output = format!(
        "## INTERFACE DESIGN BATCH {}/{}\n\n",
        batch_index + 1,
        batches.len()
    );

    // Show features for this batch
    for area_name in &batch_areas {
        let area = &feature_tree.functional_areas[*area_name];
        output.push_str(&format!("### {} ({})\n\n", area_name, area.description));

        for feature in &area.features {
            output.push_str(&format!(
                "- **{}** (`{}`)\n  - Features: {}\n  - Complexity: {}\n",
                feature.name,
                feature.id,
                feature.semantic_features.join(", "),
                feature.estimated_complexity,
            ));
            if !feature.acceptance_criteria.is_empty() {
                output.push_str(&format!(
                    "  - Criteria: {}\n",
                    feature.acceptance_criteria.join("; ")
                ));
            }
        }
        output.push('\n');
    }

    // Include interface design prompt
    output.push_str("## INSTRUCTIONS\n\n");
    output.push_str(rpg_gen::INTERFACE_DESIGN_PROMPT);
    output.push('\n');

    // Show existing interfaces if any
    if let Some(design) = interface_design {
        output.push_str(&format!(
            "\n## EXISTING INTERFACES ({} modules)\n\n",
            design.modules.len()
        ));
        for (path, module) in design.modules.iter().take(5) {
            output.push_str(&format!(
                "- {} ({} functions)\n",
                path,
                module.public_api.len()
            ));
        }
        if design.modules.len() > 5 {
            output.push_str(&format!("... and {} more\n", design.modules.len() - 5));
        }
    }

    output.push_str(&format!(
        "\n## NEXT_ACTION\n\n\
        Design interfaces for the {} areas above.\n\
        Call `submit_interface_design` with the JSON result.\n",
        batch_areas.len()
    ));

    Ok(output)
}

/// Submit interface design for a batch.
/// Returns (output_message, task_batches) - task_batches is Some when transitioning to Executing.
#[allow(clippy::type_complexity)]
pub fn submit_interface_design(
    project_root: &Path,
    plan: &mut GenerationPlan,
    interfaces_json: &str,
    revision: Option<&str>,
) -> Result<(String, Option<Vec<(usize, usize)>>), String> {
    // Check revision for staleness
    check_revision(plan, revision)?;

    let GenerationState::DesigningInterfaces {
        feature_tree: _,
        interface_design,
    } = &mut plan.state
    else {
        return Err(format!(
            "Cannot submit interfaces in {} phase.",
            plan.phase()
        ));
    };

    // Parse the submitted interfaces
    let new_design: InterfaceDesign = serde_json::from_str(interfaces_json)
        .map_err(|e| format!("Invalid interface design JSON: {}", e))?;

    // Merge with existing design
    let design = interface_design.get_or_insert_with(InterfaceDesign::default);
    for (path, module) in new_design.modules {
        design.modules.insert(path, module);
    }
    for (name, dtype) in new_design.data_types {
        design.data_types.insert(name, dtype);
    }
    for dep in new_design.dependency_graph {
        if !design.dependency_graph.contains(&dep) {
            design.dependency_graph.push(dep);
        }
    }

    plan.stats.total_interfaces = design.modules.len();
    plan.touch();

    // Save intermediate state
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    let (feature_tree, interface_design) = match &plan.state {
        GenerationState::DesigningInterfaces {
            feature_tree,
            interface_design,
        } => (feature_tree.clone(), interface_design.clone()),
        _ => unreachable!(),
    };

    // Check if we have all interfaces
    let expected_modules = feature_tree.functional_areas.len();
    let current_modules = interface_design.as_ref().map_or(0, |d| d.modules.len());

    if current_modules >= expected_modules {
        // All interfaces done - transition to Executing state
        let interface_design = interface_design.unwrap_or_default();
        let skeletons = FileSkeletonSet::new(&plan.language);

        // Build task graph and batches
        let (task_graph, task_batches) = build_task_batches(&interface_design, &skeletons);

        plan.stats.total_tasks = task_graph.tasks.len();

        // Transition to Executing state
        plan.start_execution(feature_tree, interface_design, skeletons, task_graph);

        // Save final state
        let plan_json = serde_json::to_string_pretty(plan)
            .map_err(|e| format!("Failed to serialize plan: {}", e))?;
        storage::save_generation_plan(project_root, &plan_json)
            .map_err(|e| format!("Failed to save plan: {}", e))?;

        Ok((
            format!(
                "## INTERFACES SUBMITTED\n\n\
                modules: {}\n\
                tasks: {}\n\
                batches: {}\n\n\
                All interfaces designed! Transitioned to Executing phase.\n\n\
                ## NEXT_ACTION\n\n\
                Call `get_tasks_for_generation(batch_index=0)` to start code generation.\n",
                current_modules,
                plan.stats.total_tasks,
                task_batches.len(),
            ),
            Some(task_batches),
        ))
    } else {
        Ok((
            format!(
                "## INTERFACES SUBMITTED\n\n\
                modules: {}/{}\n\
                data_types: {}\n\n\
                ## NEXT_ACTION\n\n\
                Continue with next batch: `get_interfaces_for_design(batch_index=N)`\n",
                current_modules,
                expected_modules,
                interface_design.as_ref().map_or(0, |d| d.data_types.len()),
            ),
            None,
        ))
    }
}

/// Build topologically-ordered task batches from interface design.
///
/// Uses the interface design's dependency_graph to build a proper DAG.
/// Tasks are ordered so dependencies are generated before dependents.
pub fn build_task_batches(
    interface_design: &InterfaceDesign,
    _skeletons: &FileSkeletonSet,
) -> (TaskGraph, Vec<(usize, usize)>) {
    use std::collections::{HashMap, HashSet, VecDeque};

    // Build task graph from interface design
    let mut task_graph = TaskGraph::new();
    let mut task_ids: Vec<String> = Vec::new();

    // Map file paths to their task IDs for dependency resolution
    let mut file_to_tasks: HashMap<String, Vec<String>> = HashMap::new();

    for (file_path, module) in &interface_design.modules {
        for func_sig in &module.public_api {
            let task_id = format!("{}:{}", file_path, func_sig.name);

            let skeleton =
                EntitySkeleton::new(&task_id, &func_sig.name, EntitySkeletonKind::Function);

            let task = GenerationTask {
                planned_id: task_id.clone(),
                resolved_entity_id: None,
                file_path: file_path.clone().into(),
                kind: TaskKind::GenerateFunction,
                dependencies: Vec::new(), // Will be populated below
                signature: Some(func_sig.to_core_signature()),
                semantic_features: func_sig.semantic_features.clone(),
                skeleton,
                context_entities: Vec::new(),
                status: TaskStatus::Pending,
                acceptance_criteria: Vec::new(),
                iteration_history: rpg_gen::tasks::TaskIterationHistory::default(),
            };

            task_graph.tasks.insert(task_id.clone(), task);
            task_ids.push(task_id.clone());
            file_to_tasks
                .entry(file_path.clone())
                .or_default()
                .push(task_id);
        }
    }

    // Build task dependencies from interface dependency graph
    // If module A depends on module B, all tasks in A depend on all tasks in B
    for dep in &interface_design.dependency_graph {
        let source_tasks = file_to_tasks.get(&dep.source).cloned().unwrap_or_default();
        let target_tasks = file_to_tasks.get(&dep.target).cloned().unwrap_or_default();

        for source_task_id in &source_tasks {
            if let Some(task) = task_graph.tasks.get_mut(source_task_id) {
                for target_task_id in &target_tasks {
                    if !task.dependencies.contains(target_task_id) {
                        task.dependencies.push(target_task_id.clone());
                    }
                }
            }
        }
    }

    // Topological sort using Kahn's algorithm
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    for task_id in &task_ids {
        in_degree.insert(task_id.clone(), 0);
        adjacency.insert(task_id.clone(), Vec::new());
    }

    // Build reverse adjacency (who depends on whom) and compute in-degrees
    for (task_id, task) in &task_graph.tasks {
        for dep in &task.dependencies {
            if adjacency.contains_key(dep) {
                adjacency.get_mut(dep).unwrap().push(task_id.clone());
                *in_degree.get_mut(task_id).unwrap() += 1;
            }
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut topo_order: Vec<String> = Vec::new();

    for (task_id, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(task_id.clone());
        }
    }

    while let Some(task_id) = queue.pop_front() {
        topo_order.push(task_id.clone());

        if let Some(dependents) = adjacency.get(&task_id) {
            for dependent in dependents {
                if let Some(deg) = in_degree.get_mut(dependent) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }
    }

    // If topo sort didn't include all tasks, there's a cycle — fall back to insertion order
    if topo_order.len() != task_ids.len() {
        eprintln!(
            "rpg-gen: dependency cycle detected, falling back to insertion order ({} of {} tasks)",
            topo_order.len(),
            task_ids.len()
        );
        // Add remaining tasks that weren't reached
        let topo_set: HashSet<_> = topo_order.iter().cloned().collect();
        for task_id in &task_ids {
            if !topo_set.contains(task_id) {
                topo_order.push(task_id.clone());
            }
        }
    }

    task_graph.topological_order = topo_order;

    // Build batches (8 tasks per batch)
    let batch_size = 8;
    let batches: Vec<(usize, usize)> = (0..task_graph.topological_order.len())
        .step_by(batch_size)
        .map(|start| {
            (
                start,
                (start + batch_size).min(task_graph.topological_order.len()),
            )
        })
        .collect();

    (task_graph, batches)
}

/// Get a batch of tasks for code generation.
///
/// Enhanced with:
/// - Dependency source code context (reads completed dependency files from disk)
/// - TDD instructions (test-first workflow with `report_task_outcome`)
pub fn get_tasks_for_generation(
    project_root: &Path,
    plan: &GenerationPlan,
    session: &GenerationSession,
    batch_index: usize,
) -> Result<String, String> {
    let GenerationState::Executing { task_graph, .. } = &plan.state else {
        return Err(format!(
            "Cannot get tasks in {} phase. Complete interface design first.",
            plan.phase()
        ));
    };

    let batches = &session.task_batches;
    if batch_index >= batches.len() {
        return Err(format!(
            "Batch index {} out of range. {} batches available.",
            batch_index,
            batches.len()
        ));
    }

    let (start, end) = batches[batch_index];
    let batch_task_ids: Vec<_> = task_graph.topological_order[start..end].to_vec();

    let mut output = format!(
        "## GENERATION BATCH {}/{} ({} tasks)\n\n",
        batch_index + 1,
        batches.len(),
        batch_task_ids.len(),
    );

    // Collect completed dependency source code for context enrichment
    let completed_sources = collect_dependency_sources(project_root, task_graph);

    for task_id in &batch_task_ids {
        if let Some(task) = task_graph.tasks.get(task_id) {
            output.push_str(&format!("### `{}`\n\n", task.planned_id));
            output.push_str(&format!("- **File**: {}\n", task.file_path.display()));
            output.push_str(&format!("- **Kind**: {:?}\n", task.kind));
            output.push_str(&format!(
                "- **Features**: {}\n",
                task.semantic_features.join(", ")
            ));

            if let Some(sig) = &task.signature {
                let params: Vec<String> = sig
                    .parameters
                    .iter()
                    .map(|p| {
                        if let Some(ref t) = p.type_annotation {
                            format!("{}: {}", p.name, t)
                        } else {
                            p.name.clone()
                        }
                    })
                    .collect();
                let ret = sig.return_type.as_deref().unwrap_or("()");
                output.push_str(&format!(
                    "- **Signature**: `fn({}) -> {}`\n",
                    params.join(", "),
                    ret
                ));
            }

            if !task.dependencies.is_empty() {
                output.push_str(&format!(
                    "- **Dependencies**: {}\n",
                    task.dependencies.join(", ")
                ));

                // Include dependency source code context
                for dep_id in &task.dependencies {
                    if let Some(source) = completed_sources.get(dep_id.as_str()) {
                        output.push_str(&format!(
                            "\n#### Dependency: `{}`\n```\n{}\n```\n",
                            dep_id,
                            truncate_source(source, 50),
                        ));
                    }
                }
            }

            output.push('\n');
        }
    }

    // Use TDD instructions
    output.push_str("## INSTRUCTIONS\n\n");
    output.push_str(rpg_gen::TDD_TASK_INSTRUCTIONS_PROMPT);
    output.push('\n');

    output.push_str(&format!(
        "\n## NEXT_ACTION\n\n\
        For each task above:\n\
        1. Write tests first\n\
        2. Implement the code\n\
        3. Run the tests\n\
        4. Call `report_task_outcome` with the result\n\
        5. Follow the routing instructions\n\n\
        {} tasks in this batch.\n",
        batch_task_ids.len()
    ));

    Ok(output)
}

/// Collect source code from completed dependency tasks for context enrichment.
fn collect_dependency_sources<'a>(
    project_root: &Path,
    task_graph: &'a TaskGraph,
) -> HashMap<&'a str, String> {
    let mut sources = HashMap::new();

    for (task_id, task) in &task_graph.tasks {
        let path = match &task.status {
            TaskStatus::Completed { file_path, .. } => file_path.clone(),
            TaskStatus::Validated => project_root.join(&task.file_path),
            _ => continue,
        };
        if let Ok(content) = std::fs::read_to_string(&path) {
            sources.insert(task_id.as_str(), content);
        }
    }

    sources
}

/// Truncate source code to a max number of lines for context display.
fn truncate_source(source: &str, max_lines: usize) -> &str {
    let mut end = 0;
    let mut line_count = 0;
    for (i, ch) in source.char_indices() {
        if ch == '\n' {
            line_count += 1;
            if line_count >= max_lines {
                end = i;
                break;
            }
        }
        end = i + ch.len_utf8();
    }
    &source[..end]
}

/// Submit generated code completions.
pub fn submit_generated_code(
    project_root: &Path,
    plan: &mut GenerationPlan,
    completions_json: &str,
    revision: Option<&str>,
) -> Result<String, String> {
    // Check revision for staleness
    check_revision(plan, revision)?;

    // Check phase first
    let phase = plan.phase();
    if !matches!(plan.state, GenerationState::Executing { .. }) {
        return Err(format!("Cannot submit code in {} phase.", phase));
    }

    // Parse completions
    let completions: HashMap<String, String> = serde_json::from_str(completions_json)
        .map_err(|e| format!("Invalid completions JSON: {}", e))?;

    let mut completed = 0;
    let mut errors = Vec::new();
    let new_batch;

    // Process completions - extract task_graph mutably
    if let GenerationState::Executing {
        task_graph,
        current_batch,
        ..
    } = &mut plan.state
    {
        for (planned_id, file_path) in completions {
            // Find the task
            if let Some(task) = task_graph.tasks.get_mut(&planned_id) {
                // Verify file exists
                let full_path = project_root.join(&file_path);
                if full_path.exists() {
                    // Compute content hash
                    let content = std::fs::read_to_string(&full_path)
                        .map_err(|e| format!("Failed to read {}: {}", file_path, e))?;
                    let hash = compute_hash(&content);

                    task.status = TaskStatus::Completed {
                        file_path: full_path,
                        content_hash: hash,
                    };
                    completed += 1;
                } else {
                    errors.push(format!("{}: file not found at {}", planned_id, file_path));
                }
            } else {
                errors.push(format!("{}: unknown task", planned_id));
            }
        }

        *current_batch += 1;
        new_batch = *current_batch;
    } else {
        unreachable!()
    }

    // Now update plan outside the borrow
    plan.stats.tasks_completed += completed;
    plan.touch();

    // Save
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    let total = plan.stats.total_tasks;
    let done = plan.stats.tasks_completed;

    let mut output = format!(
        "## CODE SUBMITTED\n\n\
        completed: {}/{}\n\
        progress: {:.0}%\n",
        done,
        total,
        if total > 0 {
            done as f64 / total as f64 * 100.0
        } else {
            0.0
        },
    );

    if !errors.is_empty() {
        output.push_str(&format!("\n## ERRORS ({}):\n", errors.len()));
        for err in &errors {
            output.push_str(&format!("- {}\n", err));
        }
    }

    if done >= total {
        output.push_str("\n## NEXT_ACTION\n\nAll tasks completed! Call `validate_generation` to verify the generated code.\n");
    } else {
        output.push_str(&format!(
            "\n## NEXT_ACTION\n\nContinue with next batch: `get_tasks_for_generation(batch_index={})`\n",
            new_batch
        ));
    }

    Ok(output)
}

/// Validate generated code against the plan.
pub fn validate_generation(
    project_root: &Path,
    plan: &mut GenerationPlan,
    task_ids: Option<Vec<String>>,
) -> Result<String, String> {
    let GenerationState::Executing { task_graph, .. } = &mut plan.state else {
        return Err(format!("Cannot validate in {} phase.", plan.phase()));
    };

    let mut validated = 0;
    let mut failed = 0;
    let mut issues = Vec::new();

    // Collect task IDs to validate first (to avoid borrow issues)
    let task_ids_to_validate: Vec<String> = if let Some(ids) = &task_ids {
        task_graph
            .tasks
            .keys()
            .filter(|id| ids.contains(id))
            .cloned()
            .collect()
    } else {
        task_graph
            .tasks
            .iter()
            .filter(|(_, t)| matches!(t.status, TaskStatus::Completed { .. }))
            .map(|(id, _)| id.clone())
            .collect()
    };

    for task_id in task_ids_to_validate {
        let task = task_graph.tasks.get(&task_id).unwrap();

        let TaskStatus::Completed {
            file_path,
            content_hash,
        } = &task.status
        else {
            continue;
        };

        let file_path = file_path.clone();
        let content_hash = content_hash.clone();
        let semantic_features = task.semantic_features.clone();

        // Read file and verify hash
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                issues.push(format!("{}: file read error: {}", task_id, e));
                // Mark task as failed
                if let Some(task) = task_graph.tasks.get_mut(&task_id) {
                    task.status = TaskStatus::Failed {
                        error: format!("file read error: {}", e),
                        retry_count: 0,
                    };
                }
                failed += 1;
                continue;
            }
        };

        let mut task_issues = Vec::new();

        let current_hash = compute_hash(&content);
        if current_hash != content_hash {
            task_issues.push("file modified since generation (hash mismatch)".to_string());
        }

        // Parse file with rpg-parser to validate syntax and extract entities
        let rel_path = file_path
            .strip_prefix(project_root)
            .unwrap_or(&file_path)
            .to_path_buf();

        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let planned_signature = task_graph
            .tasks
            .get(&task_id)
            .and_then(|t| t.signature.clone());

        if let Some(lang) = rpg_parser::languages::Language::from_extension(ext) {
            let entities = rpg_parser::entities::extract_entities(&rel_path, &content, lang);

            if entities.is_empty() {
                task_issues.push("no entities extracted from generated code".to_string());
            } else {
                // Check that the expected function/entity exists
                let task_name = task_id.split(':').next_back().unwrap_or(&task_id);
                let matching_entity = entities.iter().find(|e| e.name == task_name);

                if let Some(entity) = matching_entity {
                    // Tier 2: Signature comparison
                    if let Some(ref planned_sig) = planned_signature {
                        validate_signature(
                            &task_id,
                            planned_sig,
                            entity.signature.as_ref(),
                            &mut task_issues,
                        );
                    }
                } else {
                    task_issues.push(format!(
                        "expected entity '{}' not found in parsed code",
                        task_name
                    ));
                }
            }
        } else {
            // Unknown language — skip parsing validation
            // Still do semantic checks below
        }

        // Semantic feature validation
        if semantic_features
            .iter()
            .any(|f| f.contains("return") || f.contains("compute"))
        {
            // Check for return statement
            if !content.contains("return") && !content.contains("->") {
                task_issues.push("expected return behavior but no return found".to_string());
            }
        }

        // Update task status based on validation results
        if task_issues.is_empty() {
            // Mark as validated
            if let Some(task) = task_graph.tasks.get_mut(&task_id) {
                task.status = TaskStatus::Validated;
            }
            validated += 1;
        } else {
            // Mark as failed with all issues
            for issue in &task_issues {
                issues.push(format!("{}: {}", task_id, issue));
            }
            if let Some(task) = task_graph.tasks.get_mut(&task_id) {
                task.status = TaskStatus::Failed {
                    error: task_issues.join("; "),
                    retry_count: 0,
                };
            }
            failed += 1;
        }
    }

    // Count total validated/failed from task graph state (not just this call)
    let total_validated = task_graph
        .tasks
        .values()
        .filter(|t| matches!(t.status, TaskStatus::Validated))
        .count();
    let total_failed = task_graph
        .tasks
        .values()
        .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
        .count();

    plan.stats.tasks_validated = total_validated;
    plan.stats.tasks_failed = total_failed;
    plan.stats.update_completion();
    plan.touch();

    // Save
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    let mut output = format!(
        "## VALIDATION RESULTS\n\n\
        validated: {}\n\
        failed: {}\n\
        pass_rate: {:.0}%\n",
        validated,
        failed,
        plan.stats.validation_pass_rate * 100.0,
    );

    if !issues.is_empty() {
        output.push_str(&format!("\n## ISSUES ({}):\n", issues.len()));
        for issue in &issues {
            output.push_str(&format!("- {}\n", issue));
        }
        output.push_str("\n## NEXT_ACTION\n\nFix the issues above, then call `retry_failed_tasks` or `validate_generation` again.\n");
    } else {
        output.push_str(
            "\n## NEXT_ACTION\n\nAll validations passed! Call `finalize_generation` to complete.\n",
        );
    }

    Ok(output)
}

/// Get generation status dashboard.
pub fn generation_status(plan: &GenerationPlan, session: Option<&GenerationSession>) -> String {
    let mut output = format!(
        "=== GENERATION STATUS ===\n\n\
        phase: {}\n\
        revision: {}\n\
        created: {}\n\
        updated: {}\n",
        plan.phase(),
        plan.revision,
        plan.created_at.format("%Y-%m-%d %H:%M:%S"),
        plan.updated_at.format("%Y-%m-%d %H:%M:%S"),
    );

    // Stats
    output.push_str(&format!(
        "\n## PROGRESS\n\
        features: {}\n\
        interfaces: {}\n\
        files: {}\n\
        tasks: {}/{} completed\n\
        validated: {}\n\
        failed: {}\n\
        iterations: {}\n\
        prompt_tokens: {}\n\
        completion_tokens: {}\n\
        estimated_cost_usd: {:.6}\n",
        plan.stats.total_features,
        plan.stats.total_interfaces,
        plan.stats.total_files,
        plan.stats.tasks_completed,
        plan.stats.total_tasks,
        plan.stats.tasks_validated,
        plan.stats.tasks_failed,
        plan.stats.total_iterations,
        plan.stats.total_prompt_tokens,
        plan.stats.total_completion_tokens,
        plan.stats.total_estimated_cost_usd,
    ));

    // Phase-specific info
    match &plan.state {
        GenerationState::Initializing => {
            output.push_str(
                "\n## NEXT STEP\n\nCall `submit_feature_tree` with the spec decomposition.\n",
            );
        }
        GenerationState::Planning { tasks } => {
            output.push_str(&format!("\n## PLANNING ({} tasks)\n", tasks.len()));
        }
        GenerationState::DesigningInterfaces {
            feature_tree,
            interface_design,
        } => {
            output.push_str(&format!(
                "\n## DESIGNING INTERFACES\n\
                areas: {}\n\
                modules designed: {}\n",
                feature_tree.functional_areas.len(),
                interface_design.as_ref().map_or(0, |d| d.modules.len()),
            ));
            if let Some(sess) = session {
                output.push_str(&format!(
                    "interface_batches: {}\n",
                    sess.interface_batches.len()
                ));
            }
        }
        GenerationState::GeneratingSkeletons { .. } => {
            output.push_str("\n## GENERATING SKELETONS\n");
        }
        GenerationState::Executing {
            task_graph,
            current_batch,
            ..
        } => {
            let pending_count = task_graph
                .tasks
                .values()
                .filter(|t| matches!(t.status, TaskStatus::Pending))
                .count();
            // Use session task_batches if available, otherwise fall back to task_graph.batches
            let batch_count = session
                .map(|s| s.task_batches.len())
                .unwrap_or_else(|| task_graph.batches.len());
            output.push_str(&format!(
                "\n## EXECUTING\n\
                batch: {}/{}\n\
                tasks_pending: {}\n",
                current_batch, batch_count, pending_count,
            ));
        }
        GenerationState::Validating { results, .. } => {
            output.push_str(&format!("\n## VALIDATING ({} results)\n", results.len()));
        }
        GenerationState::Complete { final_graph_path } => {
            output.push_str(&format!(
                "\n## COMPLETE\n\
                graph: {}\n",
                final_graph_path.display()
            ));
        }
        GenerationState::Failed { error, recoverable } => {
            output.push_str(&format!(
                "\n## FAILED\n\
                error: {}\n\
                recoverable: {}\n",
                error, recoverable
            ));
        }
    }

    output
}

#[derive(Debug, Serialize)]
struct GenerationEfficiencyReport<'a> {
    revision: &'a str,
    phase: String,
    total_tasks: usize,
    tasks_completed: usize,
    tasks_validated: usize,
    total_iterations: usize,
    failure_type_counts: &'a std::collections::BTreeMap<String, usize>,
    total_prompt_tokens: usize,
    total_completion_tokens: usize,
    total_estimated_cost_usd: f64,
    total_runtime_ms: u64,
    backbones: &'a std::collections::BTreeMap<String, rpg_gen::BackboneStats>,
}

/// Build and persist a generation efficiency report.
pub fn generation_efficiency_report(
    project_root: &Path,
    plan: &GenerationPlan,
) -> Result<String, String> {
    let report = GenerationEfficiencyReport {
        revision: &plan.revision,
        phase: plan.phase().to_string(),
        total_tasks: plan.stats.total_tasks,
        tasks_completed: plan.stats.tasks_completed,
        tasks_validated: plan.stats.tasks_validated,
        total_iterations: plan.stats.total_iterations,
        failure_type_counts: &plan.stats.failure_type_counts,
        total_prompt_tokens: plan.stats.total_prompt_tokens,
        total_completion_tokens: plan.stats.total_completion_tokens,
        total_estimated_cost_usd: plan.stats.total_estimated_cost_usd,
        total_runtime_ms: plan.stats.total_runtime_ms,
        backbones: &plan.stats.per_backbone,
    };

    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("failed to serialize generation report: {}", e))?;
    let report_path = storage::generation_report_file(project_root);
    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    std::fs::write(&report_path, report_json)
        .map_err(|e| format!("failed to save {}: {}", report_path.display(), e))?;

    let avg_latency_ms = if plan.stats.total_iterations > 0 {
        plan.stats.total_runtime_ms as f64 / plan.stats.total_iterations as f64
    } else {
        0.0
    };
    let total_tokens = plan.stats.total_prompt_tokens + plan.stats.total_completion_tokens;
    let tokens_per_dollar = if plan.stats.total_estimated_cost_usd > 0.0 {
        total_tokens as f64 / plan.stats.total_estimated_cost_usd
    } else {
        0.0
    };

    let mut output = format!(
        "## GENERATION EFFICIENCY REPORT\n\n\
        report_file: {}\n\
        revision: {}\n\
        phase: {}\n\
        tasks: {}/{} completed, {} validated\n\
        iterations: {}\n\
        tokens: {} prompt + {} completion = {}\n\
        estimated_cost_usd: {:.6}\n\
        avg_latency_ms: {:.1}\n\
        tokens_per_dollar: {:.1}\n",
        report_path.display(),
        plan.revision,
        plan.phase(),
        plan.stats.tasks_completed,
        plan.stats.total_tasks,
        plan.stats.tasks_validated,
        plan.stats.total_iterations,
        plan.stats.total_prompt_tokens,
        plan.stats.total_completion_tokens,
        total_tokens,
        plan.stats.total_estimated_cost_usd,
        avg_latency_ms,
        tokens_per_dollar,
    );

    if !plan.stats.failure_type_counts.is_empty() {
        output.push_str("\n## FAILURE TYPES\n");
        for (kind, count) in &plan.stats.failure_type_counts {
            output.push_str(&format!("- {}: {}\n", kind, count));
        }
    }

    if !plan.stats.per_backbone.is_empty() {
        output.push_str("\n## QUALITY VS COST CURVE (by backbone)\n");
        output.push_str("backbone | iterations | pass_rate | cost_usd | latency_ms\n");
        output.push_str("---|---:|---:|---:|---:\n");
        for (name, stats) in &plan.stats.per_backbone {
            let denom = stats.tasks_passed + stats.tasks_failed;
            let pass_rate = if denom > 0 {
                stats.tasks_passed as f64 / denom as f64 * 100.0
            } else {
                0.0
            };
            output.push_str(&format!(
                "{} | {} | {:.1}% | {:.6} | {}\n",
                name, stats.iterations, pass_rate, stats.estimated_cost_usd, stats.latency_ms
            ));
        }
    }

    Ok(output)
}

/// Execute test command(s) with optional sandboxing and automatic adaptation,
/// then route the result through `report_task_outcome`.
#[allow(clippy::too_many_arguments)]
pub fn run_task_test_loop(
    project_root: &Path,
    plan: &mut GenerationPlan,
    task_id: &str,
    test_command: &str,
    file_path: Option<&str>,
    working_dir: Option<&str>,
    sandbox_mode: rpg_gen::SandboxMode,
    docker_image: Option<&str>,
    max_auto_adapt: usize,
    model: Option<&str>,
    backbone: Option<&str>,
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
    revision: Option<&str>,
) -> Result<String, String> {
    check_revision(plan, revision)?;

    let commands = adaptive_test_commands(test_command, max_auto_adapt);
    let mut last_exec: Option<CommandExecution> = None;
    let mut adapted = 0usize;

    for (idx, cmd) in commands.iter().enumerate() {
        let execution =
            match run_command(project_root, cmd, working_dir, sandbox_mode, docker_image) {
                Ok(execution) => execution,
                Err(err) => CommandExecution {
                    command: cmd.clone(),
                    stdout: String::new(),
                    stderr: err,
                    exit_code: Some(127),
                    duration_ms: 0,
                },
            };
        let outcome = classify_execution_outcome(&execution);
        let is_pass = matches!(outcome, rpg_gen::OutcomeKind::Pass);
        last_exec = Some(execution);
        adapted = idx;
        if is_pass {
            break;
        }
        // Command launch failures won't be fixed by adaptation variants.
        if matches!(outcome, rpg_gen::OutcomeKind::EnvError { .. }) {
            break;
        }
    }

    let execution = last_exec.ok_or("no command execution produced a result")?;
    let combined_output = format!("{}\n{}", execution.stdout, execution.stderr);
    let parsed_results = parse_test_results(&combined_output);
    let outcome = classify_execution_outcome(&execution);

    let prompt = prompt_tokens.unwrap_or(0);
    let completion = completion_tokens.unwrap_or(0);
    let estimated_cost = if prompt == 0 && completion == 0 {
        None
    } else {
        Some(estimate_cost_usd(model.or(backbone), prompt, completion))
    };

    let telemetry = rpg_gen::IterationTelemetry {
        sandbox_mode,
        command: Some(execution.command.clone()),
        runner: detect_runner(&execution.command),
        duration_ms: Some(execution.duration_ms),
        stdout_bytes: Some(execution.stdout.len()),
        stderr_bytes: Some(execution.stderr.len()),
        prompt_tokens,
        completion_tokens,
        model: model.map(std::string::ToString::to_string),
        backbone: backbone.map(std::string::ToString::to_string),
        estimated_cost_usd: estimated_cost,
        auto_adaptations: adapted,
    };

    let outcome_json = serde_json::to_string(&outcome)
        .map_err(|e| format!("failed to serialize outcome: {}", e))?;
    let test_results_json = parsed_results
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| format!("failed to serialize test results: {}", e))?;
    let telemetry_json = serde_json::to_string(&telemetry)
        .map_err(|e| format!("failed to serialize telemetry: {}", e))?;

    let mut routed = report_task_outcome(
        project_root,
        plan,
        task_id,
        &outcome_json,
        test_results_json.as_deref(),
        Some(&telemetry_json),
        file_path,
        revision,
    )?;

    routed.push_str("\n## EXECUTION TRACE\n\n");
    routed.push_str(&format!(
        "command: {}\nexit_code: {:?}\nduration_ms: {}\nauto_adaptations: {}\n",
        execution.command, execution.exit_code, execution.duration_ms, adapted
    ));
    if !execution.stdout.is_empty() {
        routed.push_str(&format!(
            "\nstdout (truncated):\n```\n{}\n```\n",
            truncate_source(&execution.stdout, 40)
        ));
    }
    if !execution.stderr.is_empty() {
        routed.push_str(&format!(
            "\nstderr (truncated):\n```\n{}\n```\n",
            truncate_source(&execution.stderr, 40)
        ));
    }

    Ok(routed)
}

/// Reset the generation session.
pub fn reset_generation(project_root: &Path) -> Result<String, String> {
    storage::clear_generation_plan(project_root)
        .map_err(|e| format!("Failed to clear generation plan: {}", e))?;

    Ok("Generation session reset. Call `init_generation` to start fresh.".into())
}

/// Retry failed tasks.
pub fn retry_failed_tasks(
    project_root: &Path,
    plan: &mut GenerationPlan,
) -> Result<String, String> {
    let GenerationState::Executing { task_graph, .. } = &mut plan.state else {
        return Err(format!("Cannot retry in {} phase.", plan.phase()));
    };

    let mut reset_count = 0;
    for task in task_graph.tasks.values_mut() {
        if let TaskStatus::Failed { retry_count, .. } = &task.status
            && *retry_count < 3
        {
            task.status = TaskStatus::Pending;
            reset_count += 1;
        }
    }

    plan.touch();

    // Save
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    Ok(format!(
        "Reset {} failed tasks to pending.\n\n\
        ## NEXT_ACTION\n\n\
        Call `get_tasks_for_generation` to retry the failed tasks.\n",
        reset_count
    ))
}

/// Report a TDD task outcome (pass, test_failure, code_error, test_error, env_error).
///
/// Records the outcome in the task's iteration history, routes the agent to the
/// appropriate next action, and persists the plan to disk.
#[allow(clippy::too_many_arguments)]
pub fn report_task_outcome(
    project_root: &Path,
    plan: &mut GenerationPlan,
    task_id: &str,
    outcome_json: &str,
    test_results_json: Option<&str>,
    telemetry_json: Option<&str>,
    file_path: Option<&str>,
    revision: Option<&str>,
) -> Result<String, String> {
    check_revision(plan, revision)?;

    let GenerationState::Executing { task_graph, .. } = &mut plan.state else {
        return Err(format!(
            "Cannot report outcome in {} phase. Must be in Executing phase.",
            plan.phase()
        ));
    };

    let task = task_graph
        .tasks
        .get_mut(task_id)
        .ok_or_else(|| format!("Unknown task: {}", task_id))?;

    // Parse outcome
    let outcome: rpg_gen::OutcomeKind =
        serde_json::from_str(outcome_json).map_err(|e| format!("Invalid outcome JSON: {}", e))?;

    // Parse optional test results
    let test_results: Option<rpg_gen::TestResults> = test_results_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| format!("Invalid test_results JSON: {}", e))?;

    let telemetry: Option<rpg_gen::IterationTelemetry> = telemetry_json
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| format!("Invalid telemetry JSON: {}", e))?;

    let iteration = task.iteration_history.outcomes.len();

    // Record the outcome
    task.iteration_history.record(rpg_gen::TaskOutcome {
        task_id: task_id.to_string(),
        outcome: outcome.clone(),
        test_results,
        telemetry: telemetry.clone(),
        iteration,
        reported_at: Utc::now(),
    });
    plan.stats
        .record_iteration_telemetry(telemetry.as_ref(), &outcome);
    plan.stats.record_failure_kind(&outcome);

    // Route based on outcome
    let (routing, should_complete) = match &outcome {
        rpg_gen::OutcomeKind::Pass => ("PASS", true),
        rpg_gen::OutcomeKind::TestFailure { .. } => ("FIX_CODE", false),
        rpg_gen::OutcomeKind::CodeError { .. } => ("FIX_CODE", false),
        rpg_gen::OutcomeKind::TestError { .. } => ("FIX_TEST", false),
        rpg_gen::OutcomeKind::EnvError { .. } => ("ENV_ERROR", false),
    };

    let mut output = String::new();

    if should_complete {
        let already_completed = matches!(
            task.status,
            TaskStatus::Completed { .. } | TaskStatus::Validated
        );

        // Mark task as completed
        if let Some(fp) = file_path {
            let full_path = project_root.join(fp);
            if full_path.exists() {
                let content = std::fs::read_to_string(&full_path)
                    .map_err(|e| format!("Failed to read {}: {}", fp, e))?;
                let hash = compute_hash(&content);
                task.status = TaskStatus::Completed {
                    file_path: full_path,
                    content_hash: hash,
                };
                if !already_completed {
                    plan.stats.tasks_completed += 1;
                }
            } else {
                return Err(format!("File not found: {}", fp));
            }
        } else {
            return Err(
                "Pass outcome requires file_path parameter (where the code was written).".into(),
            );
        }

        output.push_str("## OUTCOME RECORDED\n\n");
        output.push_str(&format!(
            "task: {}\nresult: PASS\niteration: {}\n",
            task_id, iteration
        ));
        if already_completed {
            output.push_str("note: task was already completed; completion count unchanged\n");
        }
        output.push_str(
            "\n## ROUTING: PASS\n\nTask completed successfully. Move to the next task.\n",
        );
    } else {
        // Check if max retries exceeded
        let exceeded = task.iteration_history.exceeded_max_retries();

        if exceeded {
            let error_msg = match &outcome {
                rpg_gen::OutcomeKind::TestFailure { summary, .. } => summary.clone(),
                rpg_gen::OutcomeKind::CodeError { error_message } => error_message.clone(),
                rpg_gen::OutcomeKind::TestError { error_message } => error_message.clone(),
                rpg_gen::OutcomeKind::EnvError { error_message } => error_message.clone(),
                rpg_gen::OutcomeKind::Pass => unreachable!(),
            };
            task.mark_failed(error_msg);
            plan.stats.tasks_failed += 1;

            output.push_str("## OUTCOME RECORDED\n\n");
            output.push_str(&format!(
                "task: {}\nresult: FAILED (max retries exceeded)\niteration: {}\nretries: {}/{}\n",
                task_id,
                iteration,
                task.iteration_history.counted_retries(),
                task.iteration_history.max_retries,
            ));
            output.push_str("\n## ROUTING: FAILED\n\nMax retries exceeded. This task is flagged for manual review. Move to the next task.\n");
        } else {
            output.push_str("## OUTCOME RECORDED\n\n");
            output.push_str(&format!(
                "task: {}\nresult: {}\niteration: {}\nretries: {}/{}\n",
                task_id,
                routing,
                iteration,
                task.iteration_history.counted_retries(),
                task.iteration_history.max_retries,
            ));
            output.push_str(&format!("\n## ROUTING: {}\n\n", routing));

            match routing {
                "FIX_CODE" => {
                    output.push_str(
                        "The tests are correct but the code has issues. Fix the implementation:\n",
                    );
                    output.push_str("1. Read the error/failure message above\n");
                    output.push_str("2. Fix the implementation code (do NOT change the tests)\n");
                    output.push_str("3. Re-run the tests\n");
                    output.push_str("4. Call `report_task_outcome` with the new result\n");
                }
                "FIX_TEST" => {
                    output.push_str("The test code itself is broken. Fix the tests:\n");
                    output.push_str("1. Read the error message above\n");
                    output.push_str("2. Fix the test code (setup, imports, assertions)\n");
                    output.push_str("3. Re-run the tests\n");
                    output.push_str("4. Call `report_task_outcome` with the new result\n");
                }
                "ENV_ERROR" => {
                    output.push_str(
                        "Environment issue detected (does NOT count toward retry limit):\n",
                    );
                    output.push_str(
                        "1. Fix the environment problem (install tool, set permissions, etc.)\n",
                    );
                    output.push_str("2. Re-run the tests\n");
                    output.push_str("3. Call `report_task_outcome` with the new result\n");
                }
                _ => {}
            }
        }
    }

    plan.touch();

    // Save to disk
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    Ok(output)
}

/// Finalize generation: build RPG and report results.
///
/// Returns `(output_message, task_snapshot)` where `task_snapshot` maps
/// planned entity IDs to their semantic features for pre-seeding the RPG.
pub fn finalize_generation(
    project_root: &Path,
    plan: &mut GenerationPlan,
) -> Result<(String, std::collections::BTreeMap<String, GenerationTask>), String> {
    // Verify all tasks are completed or validated
    let GenerationState::Executing { task_graph, .. } = &plan.state else {
        return Err(format!(
            "Cannot finalize in {} phase. Complete validation first.",
            plan.phase()
        ));
    };

    let pending = task_graph
        .tasks
        .values()
        .filter(|t| {
            !matches!(
                t.status,
                TaskStatus::Completed { .. } | TaskStatus::Validated
            )
        })
        .count();

    if pending > 0 {
        return Err(format!(
            "{} tasks still pending. Complete all tasks before finalizing.",
            pending
        ));
    }

    // Snapshot tasks BEFORE state transition for feature pre-seeding
    let task_snapshot = task_graph.tasks.clone();

    // Build RPG from generated code
    let graph_path = project_root.join(".rpg/graph.json");

    // Mark as complete
    plan.mark_complete(graph_path.clone());

    // Save
    let plan_json = serde_json::to_string_pretty(plan)
        .map_err(|e| format!("Failed to serialize plan: {}", e))?;
    storage::save_generation_plan(project_root, &plan_json)
        .map_err(|e| format!("Failed to save plan: {}", e))?;

    let output = format!(
        "## GENERATION COMPLETE\n\n\
        tasks_completed: {}\n\
        tasks_validated: {}\n\
        validation_pass_rate: {:.0}%\n\n\
        The generated code is ready. `finalize_generation` has already built and indexed the RPG.\n",
        plan.stats.tasks_completed,
        plan.stats.tasks_validated,
        plan.stats.validation_pass_rate * 100.0,
    );

    Ok((output, task_snapshot))
}

/// Pre-seed semantic features from task plans onto RPG entities.
///
/// After `build_rpg` indexes the generated code, this injects the planned
/// semantic features onto matching entities, making them pre-lifted.
pub fn preseed_features(
    graph: &mut rpg_core::graph::RPGraph,
    task_snapshot: &std::collections::BTreeMap<String, GenerationTask>,
) -> usize {
    let mut seeded = 0;

    for (planned_id, task) in task_snapshot {
        if task.semantic_features.is_empty() {
            continue;
        }

        // Try to find a matching entity in the RPG
        // The planned_id format is "file:entity_name", which should match entity IDs
        if let Some(entity) = graph.entities.get_mut(planned_id) {
            if entity.semantic_features.is_empty() {
                entity.semantic_features = task.semantic_features.clone();
                entity.feature_source = Some("planned".to_string());
                seeded += 1;
            }
        } else {
            // Try matching by entity name within the file
            let task_name = planned_id.split(':').next_back().unwrap_or(planned_id);
            let file_str = task.file_path.to_string_lossy();

            // Find entities in the same file with the same name
            let matching_id = graph
                .entities
                .iter()
                .find(|(_, e)| {
                    e.name == task_name && e.file.to_string_lossy().ends_with(file_str.as_ref())
                })
                .map(|(id, _)| id.clone());

            if let Some(id) = matching_id
                && let Some(entity) = graph.entities.get_mut(&id)
                && entity.semantic_features.is_empty()
            {
                entity.semantic_features = task.semantic_features.clone();
                entity.feature_source = Some("planned".to_string());
                seeded += 1;
            }
        }
    }

    seeded
}

/// Compare a planned signature against an actual extracted signature.
///
/// Reports mismatches as validation issues (Tier 2 validation).
fn validate_signature(
    task_id: &str,
    planned: &rpg_core::graph::Signature,
    actual: Option<&rpg_parser::entities::RawSignature>,
    issues: &mut Vec<String>,
) {
    let Some(actual_sig) = actual else {
        // No signature extracted — entity might be a struct or non-function
        return;
    };

    // Compare parameter count
    let planned_count = planned.parameters.len();
    let actual_count = actual_sig.parameters.len();
    if planned_count != actual_count {
        issues.push(format!(
            "{}: parameter count mismatch (planned {}, actual {})",
            task_id, planned_count, actual_count
        ));
    }

    // Compare parameter names (up to the shorter list)
    let min_count = planned_count.min(actual_count);
    for i in 0..min_count {
        let planned_name = &planned.parameters[i].name;
        let actual_name = &actual_sig.parameters[i].name;
        // Skip "self" parameters in comparison
        if planned_name == "self" || actual_name == "self" {
            continue;
        }
        if planned_name != actual_name {
            issues.push(format!(
                "{}: parameter {} name mismatch (planned '{}', actual '{}')",
                task_id, i, planned_name, actual_name
            ));
        }
    }

    // Compare return type presence
    if planned.return_type.is_some() && actual_sig.return_type.is_none() {
        issues.push(format!(
            "{}: planned return type '{}' but no return type found",
            task_id,
            planned.return_type.as_deref().unwrap_or("?")
        ));
    }
}

/// Compute SHA256 hash of content (first 16 hex chars).
fn compute_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_generation() {
        let tmp = TempDir::new().unwrap();
        let (plan, output) = init_generation(
            tmp.path(),
            "Create a key-value store".into(),
            "rust".into(),
            None,
        )
        .unwrap();

        assert!(matches!(plan.state, GenerationState::Initializing));
        assert!(output.contains("GENERATION INITIALIZED"));
        assert!(output.contains("NEXT_ACTION"));
    }

    #[test]
    fn test_init_generation_already_exists() {
        let tmp = TempDir::new().unwrap();

        // First init succeeds
        let _ = init_generation(tmp.path(), "spec".into(), "rust".into(), None).unwrap();

        // Second init fails
        let result = init_generation(tmp.path(), "spec".into(), "rust".into(), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_reset_generation() {
        let tmp = TempDir::new().unwrap();
        let _ = init_generation(tmp.path(), "spec".into(), "rust".into(), None).unwrap();

        let result = reset_generation(tmp.path()).unwrap();
        assert!(result.contains("reset"));

        // Should be able to init again
        let (plan, _) =
            init_generation(tmp.path(), "new spec".into(), "python".into(), None).unwrap();
        assert_eq!(plan.language, "python");
    }

    #[test]
    fn test_compute_hash() {
        let hash1 = compute_hash("hello world");
        let hash2 = compute_hash("hello world");
        let hash3 = compute_hash("different");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 16); // First 16 hex chars
    }

    #[test]
    fn test_build_interface_batches() {
        let mut feature_tree = FeatureTree::default();

        let mut area1 = rpg_gen::FeatureArea::new("Auth");
        area1.description = "Authentication".to_string();
        area1
            .features
            .push(rpg_gen::Feature::new("auth.login", "Login"));
        feature_tree
            .functional_areas
            .insert("Auth".to_string(), area1);

        let mut area2 = rpg_gen::FeatureArea::new("Storage");
        area2.description = "Data storage".to_string();
        area2
            .features
            .push(rpg_gen::Feature::new("storage.save", "Save"));
        feature_tree
            .functional_areas
            .insert("Storage".to_string(), area2);

        let batches = build_interface_batches(&feature_tree);
        assert!(!batches.is_empty());
        // With 2 areas and small token count, should be 1 batch
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0], (0, 2));
    }

    /// Helper: create a plan in Executing state with one task.
    fn setup_executing_plan(tmp: &std::path::Path) -> GenerationPlan {
        use rpg_gen::tasks::TaskIterationHistory;
        use rpg_gen::{EntitySkeleton, EntitySkeletonKind, FileSkeletonSet};

        let mut plan = GenerationPlan::new(tmp, "test spec", "rust");
        let skeleton = EntitySkeleton::new("t1", "func1", EntitySkeletonKind::Function);
        let task = GenerationTask {
            planned_id: "t1".to_string(),
            resolved_entity_id: None,
            file_path: "src/lib.rs".into(),
            kind: TaskKind::GenerateFunction,
            dependencies: Vec::new(),
            signature: None,
            semantic_features: vec!["compute result".into()],
            skeleton,
            context_entities: Vec::new(),
            status: TaskStatus::Pending,
            acceptance_criteria: Vec::new(),
            iteration_history: TaskIterationHistory::default(),
        };
        let mut task_graph = TaskGraph::new();
        task_graph.topological_order.push("t1".to_string());
        task_graph.tasks.insert("t1".to_string(), task);

        plan.start_execution(
            FeatureTree::default(),
            rpg_gen::InterfaceDesign::default(),
            FileSkeletonSet::new("rust"),
            task_graph,
        );
        plan.stats.total_tasks = 1;

        // Save to disk
        let plan_json = serde_json::to_string_pretty(&plan).unwrap();
        storage::save_generation_plan(tmp, &plan_json).unwrap();

        plan
    }

    #[test]
    fn test_report_task_outcome_pass() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        // Write a file so the pass can record it
        let code_path = tmp.path().join("src/lib.rs");
        std::fs::create_dir_all(code_path.parent().unwrap()).unwrap();
        std::fs::write(&code_path, "fn func1() {}").unwrap();

        let result = report_task_outcome(
            tmp.path(),
            &mut plan,
            "t1",
            r#"{"kind":"pass"}"#,
            None,
            None,
            Some("src/lib.rs"),
            None,
        )
        .unwrap();

        assert!(result.contains("ROUTING: PASS"));
        assert!(result.contains("Task completed"));
        assert_eq!(plan.stats.tasks_completed, 1);
    }

    #[test]
    fn test_report_task_outcome_pass_is_idempotent_for_stats() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        let code_path = tmp.path().join("src/lib.rs");
        std::fs::create_dir_all(code_path.parent().unwrap()).unwrap();
        std::fs::write(&code_path, "fn func1() {}").unwrap();

        report_task_outcome(
            tmp.path(),
            &mut plan,
            "t1",
            r#"{"kind":"pass"}"#,
            None,
            None,
            Some("src/lib.rs"),
            None,
        )
        .unwrap();

        let second = report_task_outcome(
            tmp.path(),
            &mut plan,
            "t1",
            r#"{"kind":"pass"}"#,
            None,
            None,
            Some("src/lib.rs"),
            None,
        )
        .unwrap();

        assert!(second.contains("completion count unchanged"));
        assert_eq!(plan.stats.tasks_completed, 1);
    }

    #[test]
    fn test_report_task_outcome_test_failure_then_pass() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        // First iteration: test failure
        let result = report_task_outcome(
            tmp.path(),
            &mut plan,
            "t1",
            r#"{"kind":"test_failure","failing_count":2,"summary":"assertion failed"}"#,
            Some(r#"{"total":5,"passed":3,"failed":2}"#),
            None,
            None,
            None,
        )
        .unwrap();
        assert!(result.contains("ROUTING: FIX_CODE"));
        assert!(result.contains("retries: 1/3"));

        // Second iteration: pass
        let code_path = tmp.path().join("src/lib.rs");
        std::fs::create_dir_all(code_path.parent().unwrap()).unwrap();
        std::fs::write(&code_path, "fn func1() { 42 }").unwrap();

        let result = report_task_outcome(
            tmp.path(),
            &mut plan,
            "t1",
            r#"{"kind":"pass"}"#,
            None,
            None,
            Some("src/lib.rs"),
            None,
        )
        .unwrap();
        assert!(result.contains("ROUTING: PASS"));
    }

    #[test]
    fn test_report_task_outcome_max_retries() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        // Exhaust retries (3 code errors)
        for i in 0..3 {
            let result = report_task_outcome(
                tmp.path(),
                &mut plan,
                "t1",
                r#"{"kind":"code_error","error_message":"syntax error"}"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();

            if i < 2 {
                assert!(result.contains("ROUTING: FIX_CODE"));
            } else {
                assert!(result.contains("ROUTING: FAILED"));
                assert!(result.contains("max retries exceeded"));
            }
        }
        assert_eq!(plan.stats.tasks_failed, 1);
    }

    #[test]
    fn test_report_task_outcome_env_error_not_counted() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        // Env errors don't count toward retries
        for _ in 0..5 {
            let result = report_task_outcome(
                tmp.path(),
                &mut plan,
                "t1",
                r#"{"kind":"env_error","error_message":"cargo not found"}"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            assert!(result.contains("ROUTING: ENV_ERROR"));
        }

        // Task should NOT be failed
        if let GenerationState::Executing { task_graph, .. } = &plan.state {
            let task = &task_graph.tasks["t1"];
            assert!(!matches!(task.status, TaskStatus::Failed { .. }));
        }
    }

    #[test]
    fn test_run_task_test_loop_records_telemetry() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        // Ensure pass can mark task completed
        let code_path = tmp.path().join("src/lib.rs");
        std::fs::create_dir_all(code_path.parent().unwrap()).unwrap();
        std::fs::write(&code_path, "fn func1() {}").unwrap();

        let output = run_task_test_loop(
            tmp.path(),
            &mut plan,
            "t1",
            "printf ok",
            Some("src/lib.rs"),
            None,
            rpg_gen::SandboxMode::Local,
            None,
            1,
            Some("gpt-4.1"),
            Some("gpt"),
            Some(100),
            Some(40),
            None,
        )
        .unwrap();

        assert!(output.contains("ROUTING: PASS"));
        assert_eq!(plan.stats.total_iterations, 1);
        assert_eq!(plan.stats.total_prompt_tokens, 100);
        assert_eq!(plan.stats.total_completion_tokens, 40);
    }

    #[test]
    fn test_generation_efficiency_report_writes_file() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        // record one telemetry iteration
        let code_path = tmp.path().join("src/lib.rs");
        std::fs::create_dir_all(code_path.parent().unwrap()).unwrap();
        std::fs::write(&code_path, "fn func1() {}").unwrap();
        let _ = report_task_outcome(
            tmp.path(),
            &mut plan,
            "t1",
            r#"{"kind":"pass"}"#,
            None,
            Some(r#"{"sandbox_mode":"local","prompt_tokens":10,"completion_tokens":5,"estimated_cost_usd":0.001}"#),
            Some("src/lib.rs"),
            None,
        )
        .unwrap();

        let report = generation_efficiency_report(tmp.path(), &plan).unwrap();
        assert!(report.contains("GENERATION EFFICIENCY REPORT"));
        assert!(rpg_core::storage::generation_report_file(tmp.path()).exists());
    }

    #[test]
    fn test_run_task_test_loop_missing_command_routes_env_error() {
        let tmp = TempDir::new().unwrap();
        let mut plan = setup_executing_plan(tmp.path());

        let output = run_task_test_loop(
            tmp.path(),
            &mut plan,
            "t1",
            "definitely_missing_test_command_123",
            None,
            None,
            rpg_gen::SandboxMode::Local,
            None,
            1,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert!(output.contains("ROUTING: ENV_ERROR"));
        assert_eq!(plan.stats.total_iterations, 1);
        assert_eq!(plan.stats.tasks_completed, 0);
    }
}
