use rpg_core::graph::{DependencyEdge, EdgeKind, Entity, EntityDeps, EntityKind, RPGraph};
use std::path::PathBuf;

fn entity(id: &str, name: &str, hierarchy_path: &str, kind: EntityKind) -> Entity {
    Entity {
        id: id.to_string(),
        kind,
        name: name.to_string(),
        file: PathBuf::from("src/lib.rs"),
        line_start: 1,
        line_end: 2,
        parent_class: None,
        semantic_features: vec![],
        hierarchy_path: hierarchy_path.to_string(),
        deps: EntityDeps::default(),
    }
}

#[test]
fn test_topological_execution_order_dependency_first() {
    let mut g = RPGraph::new("rust");
    g.insert_entity(entity("a", "a", "Api/handlers/list", EntityKind::Function));
    g.insert_entity(entity(
        "b",
        "b",
        "Core/services/process",
        EntityKind::Function,
    ));
    g.insert_entity(entity(
        "c",
        "c",
        "Core/utils/validate",
        EntityKind::Function,
    ));

    // a depends on b, b depends on c
    g.edges.push(DependencyEdge {
        source: "a".into(),
        target: "b".into(),
        kind: EdgeKind::Invokes,
    });
    g.edges.push(DependencyEdge {
        source: "b".into(),
        target: "c".into(),
        kind: EdgeKind::Invokes,
    });

    let order = rpg_encoder::reconstruction::build_topological_execution_order(&g, false);
    assert_eq!(
        order,
        vec!["c".to_string(), "b".to_string(), "a".to_string()]
    );
}

#[test]
fn test_topological_execution_order_cycle_fallback_is_deterministic() {
    let mut g = RPGraph::new("rust");
    g.insert_entity(entity("a", "a", "Core/services/a", EntityKind::Function));
    g.insert_entity(entity("b", "b", "Core/services/b", EntityKind::Function));

    // Cycle: a <-> b
    g.edges.push(DependencyEdge {
        source: "a".into(),
        target: "b".into(),
        kind: EdgeKind::Invokes,
    });
    g.edges.push(DependencyEdge {
        source: "b".into(),
        target: "a".into(),
        kind: EdgeKind::Invokes,
    });

    let order = rpg_encoder::reconstruction::build_topological_execution_order(&g, false);
    assert_eq!(order, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn test_schedule_reconstruction_batches_by_area_and_size() {
    let mut g = RPGraph::new("rust");
    g.insert_entity(entity(
        "c",
        "c",
        "Core/utils/validate",
        EntityKind::Function,
    ));
    g.insert_entity(entity(
        "b",
        "b",
        "Core/services/process",
        EntityKind::Function,
    ));
    g.insert_entity(entity("a", "a", "Api/handlers/list", EntityKind::Function));
    g.insert_entity(entity(
        "d",
        "d",
        "Api/handlers/create",
        EntityKind::Function,
    ));

    // a depends on b, b depends on c, d depends on c
    g.edges.push(DependencyEdge {
        source: "a".into(),
        target: "b".into(),
        kind: EdgeKind::Invokes,
    });
    g.edges.push(DependencyEdge {
        source: "b".into(),
        target: "c".into(),
        kind: EdgeKind::Invokes,
    });
    g.edges.push(DependencyEdge {
        source: "d".into(),
        target: "c".into(),
        kind: EdgeKind::Invokes,
    });

    let plan = rpg_encoder::reconstruction::schedule_reconstruction(
        &g,
        rpg_encoder::reconstruction::ReconstructionOptions {
            max_batch_size: 2,
            include_modules: false,
        },
    );

    assert_eq!(plan.topological_order.len(), 4);
    assert!(!plan.batches.is_empty());
    assert_eq!(plan.batches[0].area, "Core");
    assert!(plan.batches.iter().all(|b| b.entity_ids.len() <= 2));
}

#[test]
fn test_topological_execution_order_excludes_modules_by_default() {
    let mut g = RPGraph::new("rust");
    g.insert_entity(entity(
        "src/lib.rs:lib",
        "lib",
        "Core/modules/root",
        EntityKind::Module,
    ));
    g.insert_entity(entity("a", "a", "Core/services/a", EntityKind::Function));

    let order_default = rpg_encoder::reconstruction::build_topological_execution_order(&g, false);
    assert_eq!(order_default, vec!["a".to_string()]);

    let order_with_modules =
        rpg_encoder::reconstruction::build_topological_execution_order(&g, true);
    assert_eq!(order_with_modules.len(), 2);
}
