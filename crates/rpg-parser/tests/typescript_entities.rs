use rpg_core::graph::EntityKind;
use rpg_parser::entities::{RawEntity, extract_entities};
use rpg_parser::languages::Language;
use std::path::Path;

/// Extract entities with the full TOML paradigm pipeline (classify + entity queries + builtins).
/// Uses all built-in paradigm defs (React, NextJs, Redux) — suitable for unit tests.
fn extract_entities_with_paradigms(
    file: &Path,
    source: &str,
    language: Language,
) -> Vec<RawEntity> {
    let mut entities = extract_entities(file, source, language);
    let defs = rpg_parser::paradigms::defs::load_builtin_defs().unwrap();
    let qcache = rpg_parser::paradigms::query_engine::QueryCache::compile_all(&defs).unwrap();
    let active: Vec<&_> = defs.iter().collect();
    rpg_parser::paradigms::classify::classify_entities(&active, file, &mut entities);
    let extra = rpg_parser::paradigms::query_engine::execute_entity_queries(
        &qcache, &active, file, source, language, &entities,
    );
    entities.extend(extra);
    rpg_parser::paradigms::features::apply_builtin_entity_features(
        &active,
        file,
        source,
        language,
        &mut entities,
    );
    entities
}

#[test]
fn test_function_declaration() {
    let source = r#"function greet(name: string): string { return "hi"; }"#;
    let entities = extract_entities(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "greet");
    assert_eq!(entities[0].kind, EntityKind::Function);
    assert_eq!(entities[0].line_start, 1);
    assert!(entities[0].parent_class.is_none());
}

#[test]
fn test_class_with_method() {
    let source = "\
class Foo {
    bar(): void {}
}
";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert!(
        entities.len() >= 2,
        "expected at least 2 entities, got {}",
        entities.len()
    );
    let class_entity = entities
        .iter()
        .find(|e| e.name == "Foo")
        .expect("missing Foo class");
    assert_eq!(class_entity.kind, EntityKind::Class);
    assert!(class_entity.parent_class.is_none());

    let method_entity = entities
        .iter()
        .find(|e| e.name == "bar")
        .expect("missing bar method");
    assert_eq!(method_entity.kind, EntityKind::Method);
    assert_eq!(method_entity.parent_class.as_deref(), Some("Foo"));
}

#[test]
fn test_interface_declaration() {
    let source = "\
interface Animal {
    name: string;
    speak(): void;
}
";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TYPESCRIPT);
    let iface = entities
        .iter()
        .find(|e| e.name == "Animal")
        .expect("missing Animal interface");
    assert_eq!(iface.kind, EntityKind::Class);
    assert!(iface.parent_class.is_none());
}

#[test]
fn test_named_arrow_function() {
    let source = "const add = (a: number, b: number): number => a + b;";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "add");
    assert_eq!(entities[0].kind, EntityKind::Function);
}

#[test]
fn test_exported_function() {
    let source = "export function doStuff() {}";
    let entities = extract_entities(Path::new("test.ts"), source, Language::TYPESCRIPT);
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "doStuff");
    assert_eq!(entities[0].kind, EntityKind::Function);
}

#[test]
fn test_tsx_entity_extraction() {
    let source = r#"
import React from 'react';

interface ButtonProps {
    label: string;
    onClick: () => void;
}

const Button: React.FC<ButtonProps> = ({ label, onClick }) => {
    return <button onClick={onClick}>{label}</button>;
};

export function App() {
    return (
        <div>
            <Button label="Click me" onClick={() => {}} />
        </div>
    );
}
"#;
    let entities =
        extract_entities_with_paradigms(Path::new("test.tsx"), source, Language::TYPESCRIPT);
    let iface = entities.iter().find(|e| e.name == "ButtonProps");
    assert!(
        iface.is_some(),
        "expected ButtonProps interface, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(iface.unwrap().kind, EntityKind::Class);

    let button = entities.iter().find(|e| e.name == "Button");
    assert!(
        button.is_some(),
        "expected Button arrow function, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(button.unwrap().kind, EntityKind::Component);

    let app = entities.iter().find(|e| e.name == "App");
    assert!(
        app.is_some(),
        "expected App function, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(app.unwrap().kind, EntityKind::Component);
}

#[test]
fn test_next_app_router_page_and_layout_kinds() {
    let page_source = r"
export default function Page() {
    return <LoginForm />;
}
";
    let page_entities = extract_entities_with_paradigms(
        Path::new("app/auth/login/page.tsx"),
        page_source,
        Language::TYPESCRIPT,
    );
    let page = page_entities
        .iter()
        .find(|e| e.name == "Page")
        .expect("missing Page entity");
    assert_eq!(page.kind, EntityKind::Page);

    let layout_source = r"
export default function RootLayout({ children }: { children: React.ReactNode }) {
    return <html><body>{children}</body></html>;
}
";
    let layout_entities = extract_entities_with_paradigms(
        Path::new("app/layout.tsx"),
        layout_source,
        Language::TYPESCRIPT,
    );
    let layout = layout_entities
        .iter()
        .find(|e| e.name == "RootLayout")
        .expect("missing RootLayout entity");
    assert_eq!(layout.kind, EntityKind::Layout);
}

#[test]
fn test_component_hook_and_store_kinds() {
    let source = r#"
import { configureStore } from "@reduxjs/toolkit";

const authStore = configureStore({ reducer: {} });

const useAuth = () => {
    return { ok: true };
};

function LoginForm() {
    return <form />;
}
"#;

    let entities =
        extract_entities_with_paradigms(Path::new("src/auth.tsx"), source, Language::TYPESCRIPT);

    let store = entities
        .iter()
        .find(|e| e.name == "authStore")
        .expect("missing authStore entity");
    assert_eq!(store.kind, EntityKind::Store);

    let hook = entities
        .iter()
        .find(|e| e.name == "useAuth")
        .expect("missing useAuth entity");
    assert_eq!(hook.kind, EntityKind::Hook);

    let component = entities
        .iter()
        .find(|e| e.name == "LoginForm")
        .expect("missing LoginForm entity");
    assert_eq!(component.kind, EntityKind::Component);
}

#[test]
fn test_create_slice_extracts_reducers() {
    let source = r#"
const authSlice = createSlice({
    name: "auth",
    initialState: { user: null },
    reducers: {
        loginStarted(state) { state.loading = true; },
        loginSucceeded(state, action) { state.user = action.payload; },
        logout(state) { state.user = null; },
    },
});
"#;
    let entities = extract_entities_with_paradigms(
        Path::new("src/state/authSlice.ts"),
        source,
        Language::TYPESCRIPT,
    );

    // The slice itself should be a Store entity
    let slice = entities.iter().find(|e| e.name == "authSlice");
    assert!(
        slice.is_some(),
        "expected authSlice entity, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(slice.unwrap().kind, EntityKind::Store);

    // Each reducer key should be a Function entity with parent_class = "authSlice"
    let reducer_names: Vec<&str> = entities
        .iter()
        .filter(|e| e.parent_class.as_deref() == Some("authSlice"))
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        reducer_names.contains(&"loginStarted"),
        "missing loginStarted reducer, got: {:?}",
        reducer_names
    );
    assert!(
        reducer_names.contains(&"loginSucceeded"),
        "missing loginSucceeded reducer, got: {:?}",
        reducer_names
    );
    assert!(
        reducer_names.contains(&"logout"),
        "missing logout reducer, got: {:?}",
        reducer_names
    );

    // Verify reducer entities are Functions
    let login_started = entities.iter().find(|e| e.name == "loginStarted").unwrap();
    assert_eq!(login_started.kind, EntityKind::Function);
    assert_eq!(login_started.parent_class.as_deref(), Some("authSlice"));
}

#[test]
fn test_create_async_thunk_entity() {
    let source = r#"
const loginUser = createAsyncThunk(
    "auth/loginUser",
    async (credentials: { email: string }) => {
        return await fetch("/api/login");
    }
);
"#;
    let entities =
        extract_entities_with_paradigms(Path::new("src/thunks.ts"), source, Language::TYPESCRIPT);
    let thunk = entities.iter().find(|e| e.name == "loginUser");
    assert!(
        thunk.is_some(),
        "expected loginUser entity, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    // createAsyncThunk should be classified as Function, not Store
    assert_eq!(thunk.unwrap().kind, EntityKind::Function);
}

#[test]
fn test_create_api_store_entity() {
    let source = r#"
const postsApi = createApi({
    baseQuery: fetchBaseQuery({ baseUrl: "/api" }),
    endpoints: (builder) => ({
        getPosts: builder.query({ query: () => "/posts" }),
    }),
});
"#;
    let entities =
        extract_entities_with_paradigms(Path::new("src/api.ts"), source, Language::TYPESCRIPT);
    let api = entities.iter().find(|e| e.name == "postsApi");
    assert!(
        api.is_some(),
        "expected postsApi entity, got: {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert_eq!(api.unwrap().kind, EntityKind::Store);
}

#[test]
fn test_rtk_query_destructured_hooks() {
    let source = r"
export const { useGetPostsQuery, useGetUserQuery } = postsApi;
";
    let entities =
        extract_entities_with_paradigms(Path::new("src/api.ts"), source, Language::TYPESCRIPT);
    let hook_names: Vec<&str> = entities
        .iter()
        .filter(|e| e.kind == EntityKind::Hook)
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        hook_names.contains(&"useGetPostsQuery"),
        "missing useGetPostsQuery, got: {:?}",
        hook_names
    );
    assert!(
        hook_names.contains(&"useGetUserQuery"),
        "missing useGetUserQuery, got: {:?}",
        hook_names
    );
}
