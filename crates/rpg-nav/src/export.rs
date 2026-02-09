//! Export RPG graph as DOT (Graphviz) or Mermaid flowchart.

use rpg_core::graph::{EdgeKind, RPGraph};
use std::fmt::Write;

/// Export format for graph visualization.
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    Dot,
    Mermaid,
}

/// Export the graph as a DOT (Graphviz) string.
pub fn export_dot(graph: &RPGraph) -> String {
    let mut out = String::new();
    writeln!(out, "digraph RPG {{").unwrap();
    writeln!(out, "  rankdir=LR;").unwrap();
    writeln!(out, "  node [shape=box, fontsize=10];").unwrap();
    writeln!(out).unwrap();

    // Hierarchy nodes
    for (area_name, area) in &graph.hierarchy {
        writeln!(
            out,
            "  \"{}\" [shape=folder, style=filled, fillcolor=\"#e0e0ff\", label=\"{}\"];",
            area.id, area_name
        )
        .unwrap();
        write_dot_hierarchy_children(&area.id, area, &mut out);
    }

    writeln!(out).unwrap();

    // Entity nodes
    for (id, entity) in &graph.entities {
        let shape = match entity.kind {
            rpg_core::graph::EntityKind::Function | rpg_core::graph::EntityKind::Method => {
                "ellipse"
            }
            rpg_core::graph::EntityKind::Class => "box",
            rpg_core::graph::EntityKind::Page | rpg_core::graph::EntityKind::Layout => "tab",
            rpg_core::graph::EntityKind::Component => "box3d",
            rpg_core::graph::EntityKind::Hook => "ellipse",
            rpg_core::graph::EntityKind::Store => "cylinder",
            rpg_core::graph::EntityKind::Module => "component",
            rpg_core::graph::EntityKind::Controller | rpg_core::graph::EntityKind::Route => {
                "hexagon"
            }
            rpg_core::graph::EntityKind::Model => "box",
            rpg_core::graph::EntityKind::Service => "ellipse",
            rpg_core::graph::EntityKind::Middleware => "trapezium",
            rpg_core::graph::EntityKind::Test => "diamond",
        };
        let color = if entity.semantic_features.is_empty() {
            "#ffffff"
        } else {
            "#e0ffe0"
        };
        writeln!(
            out,
            "  \"{}\" [shape={}, style=filled, fillcolor=\"{}\", label=\"{}\"];",
            id, shape, color, entity.name
        )
        .unwrap();
    }

    writeln!(out).unwrap();

    // Edges
    for edge in &graph.edges {
        let style = match edge.kind {
            EdgeKind::Invokes => "solid",
            EdgeKind::Imports => "dashed",
            EdgeKind::Inherits => "bold",
            EdgeKind::Composes => "solid",
            EdgeKind::Renders => "solid",
            EdgeKind::ReadsState => "dashed",
            EdgeKind::WritesState => "bold",
            EdgeKind::Dispatches => "solid",
            EdgeKind::Contains => "dotted",
        };
        let label = match edge.kind {
            EdgeKind::Invokes => "invokes",
            EdgeKind::Imports => "imports",
            EdgeKind::Inherits => "inherits",
            EdgeKind::Composes => "composes",
            EdgeKind::Renders => "renders",
            EdgeKind::ReadsState => "reads_state",
            EdgeKind::WritesState => "writes_state",
            EdgeKind::Dispatches => "dispatches",
            EdgeKind::Contains => "contains",
        };
        writeln!(
            out,
            "  \"{}\" -> \"{}\" [style={}, label=\"{}\"];",
            edge.source, edge.target, style, label
        )
        .unwrap();
    }

    writeln!(out, "}}").unwrap();
    out
}

fn write_dot_hierarchy_children(
    parent_id: &str,
    node: &rpg_core::graph::HierarchyNode,
    out: &mut String,
) {
    for (child_name, child) in &node.children {
        writeln!(
            out,
            "  \"{}\" [shape=folder, style=filled, fillcolor=\"#e0e0ff\", label=\"{}\"];",
            child.id, child_name
        )
        .unwrap();
        writeln!(
            out,
            "  \"{}\" -> \"{}\" [style=dotted];",
            parent_id, child.id
        )
        .unwrap();
        write_dot_hierarchy_children(&child.id, child, out);
    }
}

/// Export the graph as a Mermaid flowchart string.
pub fn export_mermaid(graph: &RPGraph) -> String {
    let mut out = String::new();
    writeln!(out, "flowchart LR").unwrap();
    writeln!(out).unwrap();

    // Hierarchy nodes as subgraphs
    for (area_name, area) in &graph.hierarchy {
        let safe_id = mermaid_safe_id(&area.id);
        writeln!(out, "  subgraph {}[\"{}\"]", safe_id, area_name).unwrap();

        // List entities directly in this area
        for eid in &area.entities {
            if let Some(entity) = graph.entities.get(eid) {
                let safe_eid = mermaid_safe_id(eid);
                writeln!(
                    out,
                    "    {}[\"{}\\n({})\"]",
                    safe_eid,
                    entity.name,
                    format!("{:?}", entity.kind).to_lowercase()
                )
                .unwrap();
            }
        }

        // Recurse into children
        write_mermaid_hierarchy_children(area, graph, &mut out, 2);

        writeln!(out, "  end").unwrap();
        writeln!(out).unwrap();
    }

    // Dependency edges (skip Contains — already shown via subgraphs)
    for edge in &graph.edges {
        if edge.kind == EdgeKind::Contains {
            continue;
        }
        let src = mermaid_safe_id(&edge.source);
        let tgt = mermaid_safe_id(&edge.target);
        let arrow = match edge.kind {
            EdgeKind::Invokes
            | EdgeKind::Contains
            | EdgeKind::Composes
            | EdgeKind::Renders
            | EdgeKind::Dispatches => "-->",
            EdgeKind::Imports => "-.->",
            EdgeKind::Inherits | EdgeKind::WritesState => "==>",
            EdgeKind::ReadsState => "-.->",
        };
        let label = match edge.kind {
            EdgeKind::Invokes => "invokes",
            EdgeKind::Imports => "imports",
            EdgeKind::Inherits => "inherits",
            EdgeKind::Composes => "composes",
            EdgeKind::Renders => "renders",
            EdgeKind::ReadsState => "reads_state",
            EdgeKind::WritesState => "writes_state",
            EdgeKind::Dispatches => "dispatches",
            EdgeKind::Contains => "contains",
        };
        writeln!(out, "  {} {}|{}| {}", src, arrow, label, tgt).unwrap();
    }

    out
}

fn write_mermaid_hierarchy_children(
    node: &rpg_core::graph::HierarchyNode,
    graph: &RPGraph,
    out: &mut String,
    indent: usize,
) {
    let pad = " ".repeat(indent * 2);
    for (child_name, child) in &node.children {
        let safe_id = mermaid_safe_id(&child.id);
        writeln!(out, "{}subgraph {}[\"{}\"]", pad, safe_id, child_name).unwrap();

        for eid in &child.entities {
            if let Some(entity) = graph.entities.get(eid) {
                let safe_eid = mermaid_safe_id(eid);
                writeln!(
                    out,
                    "{}  {}[\"{}\\n({})\"]",
                    pad,
                    safe_eid,
                    entity.name,
                    format!("{:?}", entity.kind).to_lowercase()
                )
                .unwrap();
            }
        }

        write_mermaid_hierarchy_children(child, graph, out, indent + 1);
        writeln!(out, "{}end", pad).unwrap();
    }
}

/// Make an ID safe for Mermaid (replace special characters).
fn mermaid_safe_id(id: &str) -> String {
    id.replace([':', '/', '.', ' ', '-'], "_")
}

/// Export the graph in the specified format.
pub fn export(graph: &RPGraph, format: ExportFormat) -> String {
    match format {
        ExportFormat::Dot => export_dot(graph),
        ExportFormat::Mermaid => export_mermaid(graph),
    }
}
