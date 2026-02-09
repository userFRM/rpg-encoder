//! Generic entity classification from TOML rules.
//!
//! Applies classify rules from paradigm definitions in priority order.
//! Each rule can reclassify, skip (freeze), or tag an entity.

use super::defs::{ClassifyAction, EntityMatch, ParadigmDef, parse_entity_kind};
use crate::entities::RawEntity;
use regex::Regex;
use rpg_core::graph::EntityKind;
use std::path::Path;

/// Apply TOML-driven classification rules to extracted entities.
///
/// Paradigm defs are iterated in priority order (lowest priority number first = highest priority).
/// Once an entity is frozen (by `Reclassify` or `Skip`), no lower-priority paradigm can touch it.
pub fn classify_entities(active_defs: &[&ParadigmDef], file: &Path, entities: &mut [RawEntity]) {
    for entity in entities.iter_mut() {
        // Only reclassify base kinds — Method entities are never reclassified
        if !matches!(entity.kind, EntityKind::Function | EntityKind::Class) {
            continue;
        }
        'paradigms: for def in active_defs {
            for rule in &def.classify {
                if matches_entity(&rule.match_rule, entity, file) {
                    match &rule.action {
                        ClassifyAction::Skip => {
                            // Terminal: keep kind, freeze entity
                            break 'paradigms;
                        }
                        ClassifyAction::Reclassify(kind_str) => {
                            if let Some(kind) = parse_entity_kind(kind_str) {
                                entity.kind = kind;
                            }
                            // Terminal: reclassified + frozen
                            break 'paradigms;
                        }
                        ClassifyAction::Tag(_label) => {
                            // Non-terminal — continue checking rules
                        }
                    }
                }
            }
        }
    }
}

/// Check if an entity matches all specified fields of an `EntityMatch` (AND logic).
fn matches_entity(m: &EntityMatch, entity: &RawEntity, file: &Path) -> bool {
    // kind filter
    if let Some(ref kind_str) = m.kind {
        let expected = parse_entity_kind(kind_str);
        if expected.is_some_and(|k| k != entity.kind) {
            return false;
        }
        // If kind_str doesn't parse, skip this filter (validation catches it at load time)
    }

    // name_regex filter
    if let Some(ref regex_str) = m.name_regex {
        // Regex was validated at load time, so this should always succeed
        if let Ok(re) = Regex::new(regex_str)
            && !re.is_match(&entity.name)
        {
            return false;
        }
    }

    // name_starts_uppercase filter
    if let Some(true) = m.name_starts_uppercase
        && !entity.name.starts_with(|c: char| c.is_ascii_uppercase())
    {
        return false;
    }

    // name_min_length filter
    if let Some(min_len) = m.name_min_length
        && entity.name.len() < min_len
    {
        return false;
    }

    // source_contains_any filter (OR within the list)
    if let Some(ref patterns) = m.source_contains_any
        && !patterns.iter().any(|p| entity.source_text.contains(p))
    {
        return false;
    }

    // file_name_stem filter
    if let Some(ref stem) = m.file_name_stem {
        let file_stem = file
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('.').next())
            .unwrap_or("");
        if file_stem != stem {
            return false;
        }
    }

    // file_path_contains filter
    if let Some(ref substr) = m.file_path_contains {
        let path_str = file.to_string_lossy();
        if !path_str.contains(substr) {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradigms::defs::load_builtin_defs;
    use std::path::PathBuf;

    fn make_entity(name: &str, kind: EntityKind, source: &str, file: &str) -> RawEntity {
        RawEntity {
            name: name.to_string(),
            kind,
            file: PathBuf::from(file),
            line_start: 1,
            line_end: 10,
            parent_class: None,
            source_text: source.to_string(),
        }
    }

    #[test]
    fn test_react_hook_classification() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "useAuth",
            EntityKind::Function,
            "function useAuth() { return useState(false); }",
            "src/hooks/useAuth.ts",
        )];
        classify_entities(&active, Path::new("src/hooks/useAuth.ts"), &mut entities);
        assert_eq!(entities[0].kind, EntityKind::Hook);
    }

    #[test]
    fn test_react_component_classification() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "LoginForm",
            EntityKind::Function,
            "function LoginForm() { return <div>login</div>; }",
            "src/components/LoginForm.tsx",
        )];
        classify_entities(
            &active,
            Path::new("src/components/LoginForm.tsx"),
            &mut entities,
        );
        assert_eq!(entities[0].kind, EntityKind::Component);
    }

    #[test]
    fn test_nextjs_page_overrides_react() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "HomePage",
            EntityKind::Function,
            "function HomePage() { return <div>home</div>; }",
            "app/page.tsx",
        )];
        classify_entities(&active, Path::new("app/page.tsx"), &mut entities);
        // NextJs has priority 10 (runs first), so it becomes Page, not Component
        assert_eq!(entities[0].kind, EntityKind::Page);
    }

    #[test]
    fn test_nextjs_layout_classification() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "RootLayout",
            EntityKind::Function,
            "function RootLayout({ children }) { return <html>{children}</html>; }",
            "app/layout.tsx",
        )];
        classify_entities(&active, Path::new("app/layout.tsx"), &mut entities);
        assert_eq!(entities[0].kind, EntityKind::Layout);
    }

    #[test]
    fn test_redux_skip_thunk_freezes() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "fetchUser",
            EntityKind::Function,
            "const fetchUser = createAsyncThunk('user/fetch', async () => {})",
            "src/store/userSlice.ts",
        )];
        classify_entities(&active, Path::new("src/store/userSlice.ts"), &mut entities);
        // Redux skip_thunk freezes it as Function — React hook/component rules don't apply
        assert_eq!(entities[0].kind, EntityKind::Function);
    }

    #[test]
    fn test_redux_store_classification() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "userSlice",
            EntityKind::Function,
            "const userSlice = createSlice({ name: 'user', reducers: {} })",
            "src/store/userSlice.ts",
        )];
        classify_entities(&active, Path::new("src/store/userSlice.ts"), &mut entities);
        assert_eq!(entities[0].kind, EntityKind::Store);
    }

    #[test]
    fn test_method_not_reclassified() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "useAuth",
            EntityKind::Method,
            "useAuth() { return true; }",
            "src/test.ts",
        )];
        classify_entities(&active, Path::new("src/test.ts"), &mut entities);
        assert_eq!(entities[0].kind, EntityKind::Method);
    }

    #[test]
    fn test_lowercase_function_not_component() {
        let defs = load_builtin_defs().unwrap();
        let active: Vec<&_> = defs.iter().collect();
        let mut entities = vec![make_entity(
            "helper",
            EntityKind::Function,
            "function helper() { return <div/>; }",
            "src/utils.tsx",
        )];
        classify_entities(&active, Path::new("src/utils.tsx"), &mut entities);
        // Not uppercase name, so not a component. But name doesn't match hook either.
        assert_eq!(entities[0].kind, EntityKind::Function);
    }
}
