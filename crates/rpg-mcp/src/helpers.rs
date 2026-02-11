//! Utility functions shared across tool handlers.

use std::collections::BTreeMap;

/// Truncate source code to `max_lines`, preserving the signature and start of the body.
/// Appends a `(truncated)` note if the source exceeds the limit.
pub(crate) fn truncate_source(source: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if lines.len() <= max_lines {
        return source.to_string();
    }
    let mut out: String = lines[..max_lines].join("\n");
    out.push_str(&format!(
        "\n    // ... ({} more lines, truncated for context)",
        lines.len() - max_lines
    ));
    out
}

/// Parse a comma-separated entity type filter string into EntityKind values.
///
/// Accepts entity names: function, class, method, page, layout, component,
/// hook, store, file, module, directory.
/// "file" is an alias for Module (file-level entity nodes, V_L).
/// "directory" is mapped to Module for paper-schema compatibility.
pub(crate) fn parse_entity_type_filter(filter: &str) -> Vec<rpg_core::graph::EntityKind> {
    filter
        .split(',')
        .filter_map(|s| match s.trim().to_lowercase().as_str() {
            "function" => Some(rpg_core::graph::EntityKind::Function),
            "class" => Some(rpg_core::graph::EntityKind::Class),
            "method" => Some(rpg_core::graph::EntityKind::Method),
            "page" => Some(rpg_core::graph::EntityKind::Page),
            "layout" => Some(rpg_core::graph::EntityKind::Layout),
            "component" => Some(rpg_core::graph::EntityKind::Component),
            "hook" => Some(rpg_core::graph::EntityKind::Hook),
            "store" => Some(rpg_core::graph::EntityKind::Store),
            "module" | "file" | "directory" => Some(rpg_core::graph::EntityKind::Module),
            "controller" => Some(rpg_core::graph::EntityKind::Controller),
            "model" => Some(rpg_core::graph::EntityKind::Model),
            "service" => Some(rpg_core::graph::EntityKind::Service),
            "middleware" => Some(rpg_core::graph::EntityKind::Middleware),
            "route" => Some(rpg_core::graph::EntityKind::Route),
            "test" => Some(rpg_core::graph::EntityKind::Test),
            _ => None,
        })
        .collect()
}

/// Validate strict paper-style hierarchy path format: `Area/category/subcategory`.
///
/// Rules:
/// - Exactly three slash-delimited segments
/// - No empty or whitespace-padded segments
/// - No leading/trailing slash
pub(crate) fn is_three_level_hierarchy_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.ends_with('/') {
        return false;
    }

    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() != 3 {
        return false;
    }

    // Reject segments with leading/trailing whitespace to prevent malformed node names
    parts.iter().all(|p| !p.is_empty() && *p == p.trim())
}

/// Check whether a slash-delimited hierarchy path exists in the current hierarchy tree.
///
/// Accepts full paths like `Area/category/subcategory`. Returns false for empty paths
/// or when any segment is missing.
pub(crate) fn hierarchy_path_exists(
    hierarchy: &BTreeMap<String, rpg_core::graph::HierarchyNode>,
    path: &str,
) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut parts = trimmed.split('/');
    let Some(first) = parts.next() else {
        return false;
    };

    let Some(mut current) = hierarchy.get(first) else {
        return false;
    };

    for part in parts {
        let Some(next) = current.children.get(part) else {
            return false;
        };
        current = next;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{hierarchy_path_exists, is_three_level_hierarchy_path, parse_entity_type_filter};
    use rpg_core::graph::{EntityKind, HierarchyNode};
    use std::collections::BTreeMap;

    #[test]
    fn test_three_level_hierarchy_path_valid() {
        assert!(is_three_level_hierarchy_path(
            "Authentication/manage session/validate token"
        ));
    }

    #[test]
    fn test_three_level_hierarchy_path_invalid_depth() {
        assert!(!is_three_level_hierarchy_path(
            "Authentication/manage session"
        ));
        assert!(!is_three_level_hierarchy_path("A/B/C/D"));
    }

    #[test]
    fn test_three_level_hierarchy_path_invalid_empty_segments() {
        assert!(!is_three_level_hierarchy_path("A//C"));
        assert!(!is_three_level_hierarchy_path("/A/B/C"));
        assert!(!is_three_level_hierarchy_path("A/B/C/"));
        assert!(!is_three_level_hierarchy_path("A / B / C"));
        assert!(!is_three_level_hierarchy_path("A/ B/C"));
    }

    #[test]
    fn test_hierarchy_path_exists_true() {
        let mut hierarchy = BTreeMap::new();
        let mut area = HierarchyNode::new("Authentication");
        let mut category = HierarchyNode::new("sessions");
        category
            .children
            .insert("validate".to_string(), HierarchyNode::new("validate"));
        area.children.insert("sessions".to_string(), category);
        hierarchy.insert("Authentication".to_string(), area);

        assert!(hierarchy_path_exists(
            &hierarchy,
            "Authentication/sessions/validate"
        ));
    }

    #[test]
    fn test_hierarchy_path_exists_false() {
        let mut hierarchy = BTreeMap::new();
        hierarchy.insert(
            "Authentication".to_string(),
            HierarchyNode::new("Authentication"),
        );

        assert!(!hierarchy_path_exists(
            &hierarchy,
            "Authentication/sessions/validate"
        ));
        assert!(!hierarchy_path_exists(&hierarchy, ""));
    }

    #[test]
    fn test_parse_entity_type_filter_directory_alias() {
        let parsed = parse_entity_type_filter("directory,file,function");
        assert!(parsed.contains(&EntityKind::Module));
        assert!(parsed.contains(&EntityKind::Function));
    }
}
