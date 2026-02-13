//! DataFlow edge computation — derive data flow edges from Invokes edges + signatures.
//!
//! If entity A invokes entity B, and B has parameters, then data flows A→B (argument passing).
//! If B has a return type, then data also flows B→A (return value consumption).
//! This enables BFS traversals that follow *data*, not just calls.

use rpg_core::graph::{DependencyEdge, EdgeKind, RPGraph};
use std::collections::HashSet;

/// Compute DataFlow edges from existing Invokes edges and entity signatures.
///
/// This is idempotent: it clears all existing DataFlow edges/deps first, then
/// recomputes from scratch. Safe to call after signature restoration or incremental updates.
///
/// For each Invokes edge (A → B):
///   - If B has signature with parameters: add DataFlow edge A → B
///   - If B has signature with return_type: add DataFlow edge B → A
///
/// Edges are deduplicated. Forward/reverse dep vectors on entities are updated.
pub fn compute_data_flow_edges(graph: &mut RPGraph) {
    // Clear existing DataFlow edges for idempotent recomputation
    graph.edges.retain(|e| e.kind != EdgeKind::DataFlow);
    for entity in graph.entities.values_mut() {
        entity.deps.data_flows_to.clear();
        entity.deps.data_flows_from.clear();
    }

    // Collect existing Invokes edges
    let invokes_pairs: Vec<(String, String)> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Invokes)
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    // Track emitted edges to avoid duplicates from repeated invoke pairs
    let mut emitted: HashSet<(String, String)> = HashSet::new();
    let mut new_edges = Vec::new();
    let mut new_forward: Vec<(String, String)> = Vec::new(); // (entity_id, target)
    let mut new_reverse: Vec<(String, String)> = Vec::new(); // (entity_id, source)

    for (caller_id, callee_id) in &invokes_pairs {
        let callee_sig = graph
            .entities
            .get(callee_id)
            .and_then(|e| e.signature.as_ref());

        if let Some(sig) = callee_sig {
            // Forward data flow: caller passes data to callee's parameters
            if !sig.parameters.is_empty() && emitted.insert((caller_id.clone(), callee_id.clone()))
            {
                new_edges.push(DependencyEdge {
                    source: caller_id.clone(),
                    target: callee_id.clone(),
                    kind: EdgeKind::DataFlow,
                });
                new_forward.push((caller_id.clone(), callee_id.clone()));
                new_reverse.push((callee_id.clone(), caller_id.clone()));
            }

            // Return data flow: callee returns data consumed by caller
            if sig.return_type.is_some() && emitted.insert((callee_id.clone(), caller_id.clone())) {
                new_edges.push(DependencyEdge {
                    source: callee_id.clone(),
                    target: caller_id.clone(),
                    kind: EdgeKind::DataFlow,
                });
                new_forward.push((callee_id.clone(), caller_id.clone()));
                new_reverse.push((caller_id.clone(), callee_id.clone()));
            }
        }
    }

    // Update entity dep vectors
    for (entity_id, target) in &new_forward {
        if let Some(entity) = graph.entities.get_mut(entity_id)
            && !entity.deps.data_flows_to.contains(target)
        {
            entity.deps.data_flows_to.push(target.clone());
        }
    }
    for (entity_id, source) in &new_reverse {
        if let Some(entity) = graph.entities.get_mut(entity_id)
            && !entity.deps.data_flows_from.contains(source)
        {
            entity.deps.data_flows_from.push(source.clone());
        }
    }

    // Append new edges and refresh metadata to keep counts accurate
    graph.edges.extend(new_edges);
    graph.refresh_metadata();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rpg_core::graph::{Entity, EntityDeps, EntityKind, Param, Signature};
    use std::path::PathBuf;

    fn make_entity(name: &str, sig: Option<Signature>) -> Entity {
        Entity {
            id: String::new(),
            name: name.to_string(),
            kind: EntityKind::Function,
            file: PathBuf::from("src/lib.rs"),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            semantic_features: Vec::new(),
            feature_source: None,
            hierarchy_path: String::new(),
            deps: EntityDeps::default(),
            signature: sig,
        }
    }

    fn make_test_graph() -> RPGraph {
        RPGraph::new("rust")
    }

    #[test]
    fn test_data_flow_from_invokes_with_params() {
        let mut graph = make_test_graph();
        graph
            .entities
            .insert("src/lib.rs:caller".to_string(), make_entity("caller", None));
        graph.entities.insert(
            "src/lib.rs:callee".to_string(),
            make_entity(
                "callee",
                Some(Signature {
                    parameters: vec![Param {
                        name: "x".to_string(),
                        type_annotation: Some("i32".to_string()),
                    }],
                    return_type: Some("bool".to_string()),
                }),
            ),
        );
        // Add invokes edge
        graph.edges.push(DependencyEdge {
            source: "src/lib.rs:caller".to_string(),
            target: "src/lib.rs:callee".to_string(),
            kind: EdgeKind::Invokes,
        });

        compute_data_flow_edges(&mut graph);

        let df_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .collect();

        // Should have 2 DataFlow edges: caller→callee (params) and callee→caller (return)
        assert_eq!(df_edges.len(), 2);

        // Check forward dep on caller
        let caller = graph.entities.get("src/lib.rs:caller").unwrap();
        assert!(
            caller
                .deps
                .data_flows_to
                .contains(&"src/lib.rs:callee".to_string())
        );
        assert!(
            caller
                .deps
                .data_flows_from
                .contains(&"src/lib.rs:callee".to_string())
        );

        // Check forward dep on callee
        let callee = graph.entities.get("src/lib.rs:callee").unwrap();
        assert!(
            callee
                .deps
                .data_flows_to
                .contains(&"src/lib.rs:caller".to_string())
        );
        assert!(
            callee
                .deps
                .data_flows_from
                .contains(&"src/lib.rs:caller".to_string())
        );
    }

    #[test]
    fn test_no_data_flow_without_signature() {
        let mut graph = make_test_graph();
        graph
            .entities
            .insert("src/lib.rs:a".to_string(), make_entity("a", None));
        graph
            .entities
            .insert("src/lib.rs:b".to_string(), make_entity("b", None));
        graph.edges.push(DependencyEdge {
            source: "src/lib.rs:a".to_string(),
            target: "src/lib.rs:b".to_string(),
            kind: EdgeKind::Invokes,
        });

        compute_data_flow_edges(&mut graph);

        let df_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .collect();
        assert_eq!(df_edges.len(), 0);
    }

    #[test]
    fn test_data_flow_params_only_no_return() {
        let mut graph = make_test_graph();
        graph
            .entities
            .insert("src/lib.rs:a".to_string(), make_entity("a", None));
        graph.entities.insert(
            "src/lib.rs:b".to_string(),
            make_entity(
                "b",
                Some(Signature {
                    parameters: vec![Param {
                        name: "x".to_string(),
                        type_annotation: None,
                    }],
                    return_type: None,
                }),
            ),
        );
        graph.edges.push(DependencyEdge {
            source: "src/lib.rs:a".to_string(),
            target: "src/lib.rs:b".to_string(),
            kind: EdgeKind::Invokes,
        });

        compute_data_flow_edges(&mut graph);

        let df_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .collect();
        // Only 1 edge: a→b (params), no return
        assert_eq!(df_edges.len(), 1);
        assert_eq!(df_edges[0].source, "src/lib.rs:a");
        assert_eq!(df_edges[0].target, "src/lib.rs:b");
    }

    #[test]
    fn test_idempotent_recomputation() {
        let mut graph = make_test_graph();
        graph
            .entities
            .insert("src/lib.rs:a".to_string(), make_entity("a", None));
        graph.entities.insert(
            "src/lib.rs:b".to_string(),
            make_entity(
                "b",
                Some(Signature {
                    parameters: vec![Param {
                        name: "x".to_string(),
                        type_annotation: None,
                    }],
                    return_type: None,
                }),
            ),
        );
        graph.edges.push(DependencyEdge {
            source: "src/lib.rs:a".to_string(),
            target: "src/lib.rs:b".to_string(),
            kind: EdgeKind::Invokes,
        });

        // First computation
        compute_data_flow_edges(&mut graph);
        let count1 = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .count();

        // Second computation (idempotent — should not duplicate)
        compute_data_flow_edges(&mut graph);
        let count2 = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::DataFlow)
            .count();

        assert_eq!(count1, 1);
        assert_eq!(count2, 1);
    }

    #[test]
    fn test_stale_edges_cleared_on_recompute() {
        let mut graph = make_test_graph();
        graph
            .entities
            .insert("src/lib.rs:a".to_string(), make_entity("a", None));
        graph.entities.insert(
            "src/lib.rs:b".to_string(),
            make_entity(
                "b",
                Some(Signature {
                    parameters: vec![Param {
                        name: "x".to_string(),
                        type_annotation: None,
                    }],
                    return_type: None,
                }),
            ),
        );
        graph.edges.push(DependencyEdge {
            source: "src/lib.rs:a".to_string(),
            target: "src/lib.rs:b".to_string(),
            kind: EdgeKind::Invokes,
        });

        // First computation creates DataFlow edge
        compute_data_flow_edges(&mut graph);
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == EdgeKind::DataFlow)
                .count(),
            1
        );

        // Remove the Invokes edge (simulating an incremental update)
        graph.edges.retain(|e| e.kind != EdgeKind::Invokes);

        // Recompute — stale DataFlow edge should be gone
        compute_data_flow_edges(&mut graph);
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == EdgeKind::DataFlow)
                .count(),
            0
        );
        // Entity dep vectors should be cleared too
        let a = graph.entities.get("src/lib.rs:a").unwrap();
        assert!(a.deps.data_flows_to.is_empty());
        assert!(a.deps.data_flows_from.is_empty());
    }
}
