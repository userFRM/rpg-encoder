//! Interface design data structures.
//!
//! This module defines types for designing module interfaces and data types
//! before implementation. Interfaces define contracts between components.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Interface contracts between components, designed before implementation.
///
/// This captures the public API of each module, data type definitions,
/// and dependency relationships between modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceDesign {
    /// Schema version for forward compatibility
    pub version: String,

    /// Module interfaces keyed by file path
    pub modules: BTreeMap<String, ModuleInterface>,

    /// Data type specifications keyed by type name
    pub data_types: BTreeMap<String, DataTypeSpec>,

    /// Dependency graph between modules
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_graph: Vec<InterfaceDep>,
}

impl Default for InterfaceDesign {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
            modules: BTreeMap::new(),
            data_types: BTreeMap::new(),
            dependency_graph: Vec::new(),
        }
    }
}

/// Interface for a single module (file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInterface {
    /// Module name (usually derived from file path)
    pub name: String,

    /// File path for this module
    pub file_path: String,

    /// Public API functions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_api: Vec<FunctionSignature>,

    /// Internal helper functions (not part of public API)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub internal_api: Vec<FunctionSignature>,

    /// Required imports
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,

    /// Exported symbols
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
}

impl ModuleInterface {
    /// Create a new module interface.
    #[must_use]
    pub fn new(name: impl Into<String>, file_path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            file_path: file_path.into(),
            public_api: Vec::new(),
            internal_api: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
        }
    }
}

/// A function signature (planned, not yet implemented).
///
/// This is similar to `rpg_core::graph::Signature` but includes additional
/// metadata for generation (doc comments, semantic features).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSignature {
    /// Function name
    pub name: String,

    /// Parameters
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,

    /// Return type (None for void/unit)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,

    /// Documentation comment
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc_comment: String,

    /// Semantic features (verb-object phrases) describing behavior
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semantic_features: Vec<String>,
}

impl FunctionSignature {
    /// Create a new function signature.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: Vec::new(),
            return_type: None,
            doc_comment: String::new(),
            semantic_features: Vec::new(),
        }
    }

    /// Convert to rpg-core Signature type.
    #[must_use]
    pub fn to_core_signature(&self) -> rpg_core::graph::Signature {
        rpg_core::graph::Signature {
            parameters: self
                .parameters
                .iter()
                .map(|p| rpg_core::graph::Param {
                    name: p.name.clone(),
                    type_annotation: p.type_annotation.clone(),
                })
                .collect(),
            return_type: self.return_type.clone(),
        }
    }
}

/// A function parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    /// Parameter name
    pub name: String,

    /// Type annotation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_annotation: Option<String>,

    /// Whether the parameter is optional
    #[serde(default)]
    pub optional: bool,

    /// Default value (if optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

impl Parameter {
    /// Create a new required parameter.
    #[must_use]
    pub fn new(name: impl Into<String>, type_annotation: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_annotation: Some(type_annotation.into()),
            optional: false,
            default_value: None,
        }
    }

    /// Create an optional parameter with a default value.
    #[must_use]
    pub fn optional(
        name: impl Into<String>,
        type_annotation: impl Into<String>,
        default: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            type_annotation: Some(type_annotation.into()),
            optional: true,
            default_value: Some(default.into()),
        }
    }
}

/// Specification for a data type (struct, enum, trait, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataTypeSpec {
    /// Type name
    pub name: String,

    /// Kind of data type
    pub kind: DataTypeKind,

    /// Fields (for structs/enums with data)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldSpec>,

    /// Derives (e.g., Debug, Clone, Serialize)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derives: Vec<String>,

    /// Documentation comment
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc_comment: String,
}

impl DataTypeSpec {
    /// Create a new data type specification.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: DataTypeKind) -> Self {
        Self {
            name: name.into(),
            kind,
            fields: Vec::new(),
            derives: Vec::new(),
            doc_comment: String::new(),
        }
    }
}

/// Kind of data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataTypeKind {
    /// Struct (record type)
    Struct,
    /// Enum (sum type)
    Enum,
    /// Trait (interface/protocol)
    Trait,
    /// Interface (TypeScript/Java style)
    Interface,
    /// Type alias
    TypeAlias,
}

/// Specification for a field within a data type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Field name
    pub name: String,

    /// Type annotation
    pub type_annotation: String,

    /// Visibility
    #[serde(default)]
    pub visibility: Visibility,

    /// Documentation comment
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_comment: Option<String>,
}

impl FieldSpec {
    /// Create a new field specification.
    #[must_use]
    pub fn new(name: impl Into<String>, type_annotation: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_annotation: type_annotation.into(),
            visibility: Visibility::default(),
            doc_comment: None,
        }
    }
}

/// Visibility of a field or method.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Public visibility
    #[default]
    Public,
    /// Private visibility
    Private,
    /// Crate-level visibility (Rust)
    Crate,
}

/// Dependency between modules in the interface design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceDep {
    /// Source module path
    pub source: String,

    /// Target module path
    pub target: String,

    /// Kind of dependency
    pub kind: InterfaceDepKind,
}

impl InterfaceDep {
    /// Create a new interface dependency.
    #[must_use]
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        kind: InterfaceDepKind,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            kind,
        }
    }
}

/// Kind of dependency between modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceDepKind {
    /// Source imports from target
    Imports,
    /// Source implements trait from target
    Implements,
    /// Source extends type from target
    Extends,
    /// Source uses types/functions from target
    Uses,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interface_design_serialization() {
        let mut design = InterfaceDesign::default();

        let mut module = ModuleInterface::new("auth", "src/auth.rs");
        let mut func = FunctionSignature::new("login");
        func.parameters.push(Parameter::new("username", "String"));
        func.parameters.push(Parameter::new("password", "String"));
        func.return_type = Some("Result<User, Error>".to_string());
        func.semantic_features
            .push("authenticate user credentials".to_string());
        module.public_api.push(func);

        design.modules.insert("src/auth.rs".to_string(), module);

        let json = serde_json::to_string(&design).unwrap();
        let parsed: InterfaceDesign = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, "1.0.0");
        assert!(parsed.modules.contains_key("src/auth.rs"));
    }

    #[test]
    fn test_to_core_signature() {
        let mut func = FunctionSignature::new("test");
        func.parameters.push(Parameter::new("x", "i32"));
        func.return_type = Some("bool".to_string());

        let core_sig = func.to_core_signature();
        assert_eq!(core_sig.parameters.len(), 1);
        assert_eq!(core_sig.parameters[0].name, "x");
        assert_eq!(core_sig.return_type, Some("bool".to_string()));
    }
}
