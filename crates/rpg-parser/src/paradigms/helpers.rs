//! Shared detection utilities for paradigm adapters.

use std::path::Path;

/// Read the primary package manifests as a combined string for detection heuristics.
///
/// Reads package.json, requirements.txt, pyproject.toml, Cargo.toml, go.mod, etc.
/// once and shares the content across all paradigm detectors.
pub fn read_manifest(root: &Path) -> String {
    let mut content = String::new();

    // JS/TS ecosystem
    if let Ok(pkg) = std::fs::read_to_string(root.join("package.json")) {
        content.push_str(&pkg);
        content.push('\n');
    }

    // Python ecosystem
    for name in &[
        "requirements.txt",
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
    ] {
        if let Ok(req) = std::fs::read_to_string(root.join(name)) {
            content.push_str(&req);
            content.push('\n');
        }
    }

    // Rust ecosystem
    if let Ok(cargo) = std::fs::read_to_string(root.join("Cargo.toml")) {
        content.push_str(&cargo);
        content.push('\n');
    }

    // Go ecosystem
    if let Ok(gomod) = std::fs::read_to_string(root.join("go.mod")) {
        content.push_str(&gomod);
        content.push('\n');
    }

    // Java/Kotlin ecosystem
    for name in &["pom.xml", "build.gradle", "build.gradle.kts"] {
        if let Ok(java) = std::fs::read_to_string(root.join(name)) {
            content.push_str(&java);
            content.push('\n');
        }
    }

    // PHP ecosystem
    if let Ok(composer) = std::fs::read_to_string(root.join("composer.json")) {
        content.push_str(&composer);
        content.push('\n');
    }

    // Ruby ecosystem
    if let Ok(gemfile) = std::fs::read_to_string(root.join("Gemfile")) {
        content.push_str(&gemfile);
        content.push('\n');
    }

    // C#/.NET ecosystem
    for name in &["Directory.Build.props"] {
        if let Ok(dotnet) = std::fs::read_to_string(root.join(name)) {
            content.push_str(&dotnet);
            content.push('\n');
        }
    }
    // Also check for *.csproj files in root
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "csproj") {
                if let Ok(csproj) = std::fs::read_to_string(entry.path()) {
                    content.push_str(&csproj);
                    content.push('\n');
                }
                break; // Only read first .csproj
            }
        }
    }

    // Swift ecosystem
    if let Ok(pkg) = std::fs::read_to_string(root.join("Package.swift")) {
        content.push_str(&pkg);
        content.push('\n');
    }

    // Scala ecosystem
    if let Ok(sbt) = std::fs::read_to_string(root.join("build.sbt")) {
        content.push_str(&sbt);
        content.push('\n');
    }

    content
}

/// Check if the manifest content lists a dependency by name.
///
/// Multi-strategy matching to handle different manifest formats:
/// - JSON/TOML/Gradle: `"dep_name"` (double-quoted)
/// - Gemfile/Ruby: `'dep_name'` (single-quoted)
/// - go.mod/requirements.txt: whitespace-bounded tokens (unquoted)
/// - .csproj/pom.xml: `Include="dep_name"` (XML attribute)
pub fn has_dep(manifest: &str, dep_name: &str) -> bool {
    if manifest.is_empty() {
        return false;
    }
    // JSON / TOML / Gradle / Swift Package.swift (double-quoted)
    manifest.contains(&format!("\"{}\"", dep_name))
    // Gemfile / Ruby (single-quoted)
    || manifest.contains(&format!("'{}'", dep_name))
    // go.mod / requirements.txt (unquoted, whitespace-bounded token)
    // Also handles version specifiers: flask==3.0.0, django~=4.2, etc.
    || manifest.lines().any(|line| {
        line.split_whitespace().any(|token| {
            token == dep_name
                || token
                    .split(&['=', '>', '<', '~', '!', ';', '@'][..])
                    .next()
                    .is_some_and(|prefix| prefix == dep_name)
        })
    })
    // XML attribute (e.g., <PackageReference Include="Dep.Name">)
    || manifest.contains(&format!("Include=\"{}\"", dep_name))
}

/// Check if any .tsx or .jsx files exist in the project.
pub fn has_tsx_jsx_files(root: &Path) -> bool {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .add_custom_ignore_filename(".rpgignore")
        .max_depth(Some(5))
        .build();

    for entry in walker.flatten() {
        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str())
            && (ext == "tsx" || ext == "jsx")
        {
            return true;
        }
    }
    false
}

/// Check if a next.config.* file exists.
pub fn has_next_config(root: &Path) -> bool {
    root.join("next.config.js").exists()
        || root.join("next.config.mjs").exists()
        || root.join("next.config.ts").exists()
}

/// Check if app/ directory contains page.* files (Next.js App Router).
pub fn has_app_router_pages(root: &Path) -> bool {
    let app_dir = root.join("app");
    if !app_dir.is_dir() {
        return false;
    }
    let walker = ignore::WalkBuilder::new(&app_dir)
        .hidden(true)
        .git_ignore(true)
        .max_depth(Some(4))
        .build();
    for entry in walker.flatten() {
        if let Some(name) = entry.path().file_name().and_then(|n| n.to_str())
            && name.starts_with("page.")
        {
            return true;
        }
    }
    false
}

/// Check if a function/component name looks like a React component.
///
/// PascalCase name + JSX return indicators in the source snippet.
/// Shared between React and NextJs adapters.
pub fn looks_like_react_component(name: &str, source_snippet: &str) -> bool {
    let starts_upper = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    if !starts_upper {
        return false;
    }
    source_snippet.contains("return <")
        || (source_snippet.contains("return (") && source_snippet.contains('<'))
        || source_snippet.contains("=> <")
        || source_snippet.contains("React.FC")
        || source_snippet.contains("<>")
}

/// Check if a name looks like a custom React hook (use* prefix + 4th char uppercase).
pub fn looks_like_custom_hook(name: &str) -> bool {
    if !name.starts_with("use") || name.len() <= 3 {
        return false;
    }
    name.chars().nth(3).is_some_and(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_dep_json() {
        let pkg = r#"{"dependencies": {"react": "^18.0.0", "@reduxjs/toolkit": "^2.0"}}"#;
        assert!(has_dep(pkg, "react"));
        assert!(has_dep(pkg, "@reduxjs/toolkit"));
        assert!(!has_dep(pkg, "vue"));
        assert!(!has_dep("", "react"));
    }

    #[test]
    fn test_has_dep_single_quoted() {
        let gemfile = "source 'https://rubygems.org'\ngem 'rails', '~> 7.0'\ngem 'devise'";
        assert!(has_dep(gemfile, "rails"));
        assert!(has_dep(gemfile, "devise"));
        assert!(!has_dep(gemfile, "sinatra"));
    }

    #[test]
    fn test_has_dep_unquoted_gomod() {
        let gomod = "module example.com/myapp\n\ngo 1.21\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.9.1\n\tgithub.com/lib/pq v1.10.9\n)";
        assert!(has_dep(gomod, "github.com/gin-gonic/gin"));
        assert!(has_dep(gomod, "github.com/lib/pq"));
        assert!(!has_dep(gomod, "github.com/gorilla/mux"));
    }

    #[test]
    fn test_has_dep_unquoted_requirements() {
        let req = "flask==3.0.0\nrequests>=2.31.0\ndjango~=4.2";
        assert!(has_dep(req, "flask"));
        assert!(has_dep(req, "requests"));
        assert!(has_dep(req, "django"));
        assert!(!has_dep(req, "fastapi"));
    }

    #[test]
    fn test_has_dep_xml_csproj() {
        let csproj = r#"<Project Sdk="Microsoft.NET.Sdk.Web">
  <ItemGroup>
    <PackageReference Include="Microsoft.AspNetCore" Version="2.2.0" />
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
  </ItemGroup>
</Project>"#;
        assert!(has_dep(csproj, "Microsoft.AspNetCore"));
        assert!(has_dep(csproj, "Newtonsoft.Json"));
        assert!(!has_dep(csproj, "System.Text.Json"));
    }

    #[test]
    fn test_looks_like_react_component() {
        assert!(looks_like_react_component("App", "return <div>hello</div>"));
        assert!(looks_like_react_component(
            "Button",
            "React.FC<Props> = () => <button />"
        ));
        assert!(!looks_like_react_component("app", "return <div />"));
        assert!(!looks_like_react_component("App", "return 42"));
    }

    #[test]
    fn test_looks_like_custom_hook() {
        assert!(looks_like_custom_hook("useAuth"));
        assert!(looks_like_custom_hook("useState"));
        assert!(!looks_like_custom_hook("use"));
        assert!(!looks_like_custom_hook("useless"));
        assert!(!looks_like_custom_hook("notAHook"));
    }
}
