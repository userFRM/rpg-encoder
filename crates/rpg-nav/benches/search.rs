use criterion::{Criterion, criterion_group, criterion_main};
use rpg_core::graph::*;
use rpg_nav::search::{SearchMode, search};
use std::hint::black_box;
use std::path::PathBuf;

fn make_entity(id: &str, name: &str, file: &str, features: Vec<&str>, hierarchy: &str) -> Entity {
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
        hierarchy_path: hierarchy.to_string(),
        deps: EntityDeps::default(),
    }
}

/// Build a realistic graph with many entities and varied features.
fn build_search_graph(size: usize) -> RPGraph {
    let mut graph = RPGraph::new("rust");

    let feature_sets = [
        vec![
            "parse command arguments",
            "validate input flags",
            "read CLI options",
        ],
        vec![
            "send HTTP request",
            "handle connection errors",
            "retry on failure",
        ],
        vec!["database query", "user retrieval", "SQL execution"],
        vec![
            "JWT token validation",
            "authentication check",
            "session management",
        ],
        vec![
            "configuration parsing",
            "TOML reader",
            "environment variables",
        ],
        vec![
            "file system operations",
            "directory traversal",
            "path manipulation",
        ],
        vec![
            "serialize JSON data",
            "handle encoding errors",
            "format output",
        ],
        vec!["logging setup", "structured logging", "log level filtering"],
        vec![
            "error handling",
            "custom error types",
            "error chain propagation",
        ],
        vec![
            "unit test helpers",
            "mock data generation",
            "assertion utilities",
        ],
    ];

    let areas = [
        "CLI",
        "Networking",
        "DataAccess",
        "Security",
        "Config",
        "FileSystem",
        "Serialization",
        "Logging",
        "ErrorHandling",
        "Testing",
    ];

    for i in 0..size {
        let group = i % feature_sets.len();
        let file = format!("src/{}.rs", areas[group].to_lowercase());
        let name = format!("func_{}_{}", areas[group].to_lowercase(), i);
        let id = format!("{}:{}", file, name);
        let features = &feature_sets[group];
        let hierarchy = format!("{}/sub_{}/{}", areas[group], group, name);

        graph.insert_entity(make_entity(&id, &name, &file, features.clone(), &hierarchy));
        graph.insert_into_hierarchy(&hierarchy, &id);
    }

    graph.rebuild_edge_index();
    graph.rebuild_hierarchy_index();
    graph
}

fn bench_search_small(c: &mut Criterion) {
    let graph = build_search_graph(50);

    c.bench_function("search_50_entities", |b| {
        b.iter(|| {
            search(
                black_box(&graph),
                black_box("parse command arguments"),
                SearchMode::Features,
                None,
                10,
            )
        })
    });
}

fn bench_search_medium(c: &mut Criterion) {
    let graph = build_search_graph(500);

    c.bench_function("search_500_entities", |b| {
        b.iter(|| {
            search(
                black_box(&graph),
                black_box("database query user retrieval"),
                SearchMode::Features,
                None,
                10,
            )
        })
    });
}

fn bench_search_large(c: &mut Criterion) {
    let graph = build_search_graph(2000);

    c.bench_function("search_2000_entities", |b| {
        b.iter(|| {
            search(
                black_box(&graph),
                black_box("authentication JWT token"),
                SearchMode::Features,
                None,
                10,
            )
        })
    });
}

fn bench_search_snippets(c: &mut Criterion) {
    let graph = build_search_graph(500);

    c.bench_function("search_snippets_mode_500", |b| {
        b.iter(|| {
            search(
                black_box(&graph),
                black_box("func_cli_0"),
                SearchMode::Snippets,
                None,
                10,
            )
        })
    });
}

fn bench_search_with_scope(c: &mut Criterion) {
    let graph = build_search_graph(500);

    c.bench_function("search_scoped_500", |b| {
        b.iter(|| {
            search(
                black_box(&graph),
                black_box("parse arguments"),
                SearchMode::Features,
                Some("CLI"),
                10,
            )
        })
    });
}

criterion_group!(
    benches,
    bench_search_small,
    bench_search_medium,
    bench_search_large,
    bench_search_snippets,
    bench_search_with_scope,
);
criterion_main!(benches);
