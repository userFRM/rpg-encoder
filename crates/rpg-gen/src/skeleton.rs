//! File skeleton data structures.
//!
//! This module defines types for representing the structural template
//! of code files before actual implementation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A set of file skeletons for the entire project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSkeletonSet {
    /// Schema version for forward compatibility
    pub version: String,

    /// Target programming language
    pub target_language: String,

    /// File skeletons keyed by path
    pub files: BTreeMap<PathBuf, FileSkeleton>,
}

impl Default for FileSkeletonSet {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            target_language: String::new(),
            files: BTreeMap::new(),
        }
    }
}

impl FileSkeletonSet {
    /// Create a new file skeleton set for a language.
    #[must_use]
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            target_language: language.into(),
            ..Default::default()
        }
    }
}

/// Skeleton for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSkeleton {
    /// File path
    pub path: PathBuf,

    /// Kind of file
    pub kind: FileKind,

    /// Imports/includes for this file
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,

    /// Entity skeletons (functions, classes, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntitySkeleton>,

    /// Module-level documentation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_doc: Option<String>,
}

impl FileSkeleton {
    /// Create a new file skeleton.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, kind: FileKind) -> Self {
        Self {
            path: path.into(),
            kind,
            imports: Vec::new(),
            entities: Vec::new(),
            module_doc: None,
        }
    }
}

/// Kind of file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// Regular module file
    Module,
    /// Test file
    Test,
    /// Configuration file
    Config,
    /// Main entry point
    Main,
    /// Library entry point
    Library,
}

/// Skeleton for a single entity within a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySkeleton {
    /// Planned entity ID (format: "file:name")
    pub id: String,

    /// Entity name
    pub name: String,

    /// Kind of entity
    pub kind: EntitySkeletonKind,

    /// Signature (for functions/methods)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// Documentation comment
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc_comment: String,

    /// Hint for body generation
    #[serde(default)]
    pub body_hint: BodyHint,

    /// Dependencies on other entities (by ID)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

impl EntitySkeleton {
    /// Create a new entity skeleton.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: EntitySkeletonKind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            signature: None,
            doc_comment: String::new(),
            body_hint: BodyHint::default(),
            dependencies: Vec::new(),
        }
    }
}

/// Kind of entity skeleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitySkeletonKind {
    /// Function
    Function,
    /// Struct/class
    Struct,
    /// Implementation block (Rust)
    Impl,
    /// Method within a class/impl
    Method,
    /// Enum
    Enum,
    /// Trait/interface
    Trait,
    /// Constant
    Const,
}

/// Hints for body generation (similar to auto-lift confidence tiers).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyHint {
    /// Trivial: can be auto-generated (getter/setter/delegation)
    AutoGenerate {
        /// Template for auto-generation
        template: String,
    },
    /// Simple: single responsibility, provide semantic hint
    #[default]
    SemanticHint,
    /// Complex: needs full LLM generation with context
    FullGeneration {
        /// Additional context for generation
        context: String,
    },
}

impl BodyHint {
    /// Create an auto-generate hint with a template.
    #[must_use]
    pub fn auto_generate(template: impl Into<String>) -> Self {
        Self::AutoGenerate {
            template: template.into(),
        }
    }

    /// Create a full-generation hint with context.
    #[must_use]
    pub fn full_generation(context: impl Into<String>) -> Self {
        Self::FullGeneration {
            context: context.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_skeleton_serialization() {
        let mut skeleton_set = FileSkeletonSet::new("rust");

        let mut file = FileSkeleton::new("src/lib.rs", FileKind::Library);
        file.module_doc = Some("Main library entry point".to_string());

        let mut entity = EntitySkeleton::new(
            "src/lib.rs:process",
            "process",
            EntitySkeletonKind::Function,
        );
        entity.signature = Some("fn process(data: &[u8]) -> Result<Output, Error>".to_string());
        entity.doc_comment = "Process input data".to_string();
        entity.body_hint = BodyHint::full_generation("Complex parsing logic");
        file.entities.push(entity);

        skeleton_set.files.insert(PathBuf::from("src/lib.rs"), file);

        let json = serde_json::to_string(&skeleton_set).unwrap();
        let parsed: FileSkeletonSet = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.target_language, "rust");
        assert!(parsed.files.contains_key(&PathBuf::from("src/lib.rs")));
    }

    #[test]
    fn test_body_hint_variants() {
        let auto = BodyHint::auto_generate("return self.field");
        assert!(matches!(auto, BodyHint::AutoGenerate { .. }));

        let full = BodyHint::full_generation("complex logic");
        assert!(matches!(full, BodyHint::FullGeneration { .. }));

        let semantic = BodyHint::SemanticHint;
        assert!(matches!(semantic, BodyHint::SemanticHint));
    }
}
