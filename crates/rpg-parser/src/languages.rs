//! Language detection and tree-sitter grammar loading.

use std::path::Path;

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Python,
    Rust,
    TypeScript,
    JavaScript,
    Go,
    Java,
    C,
    Cpp,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Self::Python),
            "rs" => Some(Self::Rust),
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some(Self::Cpp),
            _ => None,
        }
    }

    /// Detect the primary language of a project by counting file extensions.
    pub fn detect_primary(root: &Path) -> Option<Self> {
        let mut counts = [0usize; 8]; // indexed by variant order

        let walker = ignore::WalkBuilder::new(root)
            .hidden(true)
            .git_ignore(true)
            .build();

        for entry in walker.flatten() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str())
                && let Some(lang) = Self::from_extension(ext)
            {
                counts[lang.index()] += 1;
            }
        }

        let max_idx = counts
            .iter()
            .enumerate()
            .max_by_key(|(_, count)| **count)?
            .0;

        if counts[max_idx] == 0 {
            return None;
        }

        Some(Self::from_index(max_idx))
    }

    /// Get source file glob pattern for this language.
    pub fn glob_pattern(&self) -> &'static str {
        match self {
            Self::Python => "**/*.py",
            Self::Rust => "**/*.rs",
            Self::TypeScript => "**/*.{ts,tsx}",
            Self::JavaScript => "**/*.{js,jsx,mjs,cjs}",
            Self::Go => "**/*.go",
            Self::Java => "**/*.java",
            Self::C => "**/*.{c,h}",
            Self::Cpp => "**/*.{cpp,cc,cxx,hpp,hxx,hh}",
        }
    }

    /// Parse language from name string (as returned by `name()`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "python" => Some(Self::Python),
            "rust" => Some(Self::Rust),
            "typescript" => Some(Self::TypeScript),
            "javascript" => Some(Self::JavaScript),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "c" => Some(Self::C),
            "cpp" => Some(Self::Cpp),
            _ => None,
        }
    }

    /// Display name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
        }
    }

    /// Get the tree-sitter Language for parsing.
    pub fn ts_language(&self) -> tree_sitter::Language {
        match self {
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        }
    }

    fn index(&self) -> usize {
        match self {
            Self::Python => 0,
            Self::Rust => 1,
            Self::TypeScript => 2,
            Self::JavaScript => 3,
            Self::Go => 4,
            Self::Java => 5,
            Self::C => 6,
            Self::Cpp => 7,
        }
    }

    fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Python,
            1 => Self::Rust,
            2 => Self::TypeScript,
            3 => Self::JavaScript,
            4 => Self::Go,
            5 => Self::Java,
            6 => Self::C,
            _ => Self::Cpp,
        }
    }
}
