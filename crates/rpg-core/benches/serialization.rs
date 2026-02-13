use criterion::{Criterion, criterion_group, criterion_main};
use rpg_core::graph::*;
use std::hint::black_box;
use std::path::PathBuf;

fn make_entity(id: &str, name: &str, file: &str, features: Vec<&str>) -> Entity {
    Entity {
        id: id.to_string(),
        kind: EntityKind::Function,
        name: name.to_string(),
        file: PathBuf::from(file),
        line_start: 1,
        line_end: 50,
        parent_class: None,
        semantic_features: features.into_iter().map(String::from).collect(),
        feature_source: None,
        hierarchy_path: format!("Area/category/{}", name),
        deps: EntityDeps::default(),
    }
}

fn build_graph(entity_count: usize) -> RPGraph {
    let mut graph = RPGraph::new("rust");

    for i in 0..entity_count {
        let file = format!("src/file_{}.rs", i / 5);
        let name = format!("func_{}", i);
        let id = format!("{}:{}", file, name);
        let features = vec!["parse data", "validate input", "handle errors"];
        graph.insert_entity(make_entity(&id, &name, &file, features));
        graph.insert_into_hierarchy(&format!("Area/cat_{}/{}", i % 3, name), &id);
    }

    // Add some edges
    let entity_ids: Vec<String> = graph.entities.keys().cloned().collect();
    for i in 0..entity_ids.len().saturating_sub(1) {
        graph.edges.push(DependencyEdge {
            source: entity_ids[i].clone(),
            target: entity_ids[i + 1].clone(),
            kind: EdgeKind::Invokes,
        });
    }

    graph.rebuild_edge_index();
    graph
}

fn bench_serialization_100(c: &mut Criterion) {
    let graph = build_graph(100);

    c.bench_function("serialize_json_100_entities", |b| {
        b.iter(|| serde_json::to_string(black_box(&graph)).unwrap())
    });
}

fn bench_deserialization_100(c: &mut Criterion) {
    let graph = build_graph(100);
    let json = serde_json::to_string(&graph).unwrap();

    c.bench_function("deserialize_json_100_entities", |b| {
        b.iter(|| serde_json::from_str::<RPGraph>(black_box(&json)).unwrap())
    });
}

fn bench_serialization_500(c: &mut Criterion) {
    let graph = build_graph(500);

    c.bench_function("serialize_json_500_entities", |b| {
        b.iter(|| serde_json::to_string(black_box(&graph)).unwrap())
    });
}

fn bench_edge_lookup(c: &mut Criterion) {
    let graph = build_graph(500);
    let entity_ids: Vec<&String> = graph.entities.keys().collect();
    let middle_id = entity_ids[entity_ids.len() / 2];

    c.bench_function("edge_lookup_indexed_500", |b| {
        b.iter(|| graph.edges_for(black_box(middle_id)))
    });
}

fn bench_rebuild_edge_index(c: &mut Criterion) {
    let mut graph = build_graph(500);

    c.bench_function("rebuild_edge_index_500", |b| {
        b.iter(|| {
            black_box(&mut graph).rebuild_edge_index();
        })
    });
}

fn bench_rebuild_hierarchy_index(c: &mut Criterion) {
    let mut graph = build_graph(500);
    graph.assign_hierarchy_ids();

    c.bench_function("rebuild_hierarchy_index_500", |b| {
        b.iter(|| {
            black_box(&mut graph).rebuild_hierarchy_index();
        })
    });
}

criterion_group!(
    benches,
    bench_serialization_100,
    bench_deserialization_100,
    bench_serialization_500,
    bench_edge_lookup,
    bench_rebuild_edge_index,
    bench_rebuild_hierarchy_index,
);
criterion_main!(benches);
