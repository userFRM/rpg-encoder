//! Feature quality critique — soft feedback on submitted semantic features.
//!
//! Non-blocking: features are always applied, but warnings help the LLM self-correct
//! on subsequent submissions. Checks for vague verbs, implementation details,
//! too-short/too-long features, and duplicates.

use std::collections::HashSet;
use std::fmt;

/// A quality warning for a specific feature on an entity.
#[derive(Debug)]
pub struct QualityWarning {
    pub entity_id: String,
    pub feature: String,
    pub issue: QualityIssue,
    pub suggestion: Option<String>,
}

/// Categories of feature quality issues.
#[derive(Debug, PartialEq, Eq)]
pub enum QualityIssue {
    /// Feature has fewer than 2 words.
    TooShort,
    /// Feature has more than 10 words.
    TooLong,
    /// Feature uses a vague verb (e.g., "handle", "process", "manage").
    VagueVerb(String),
    /// Feature contains implementation-level language (e.g., "loop", "iterate", "array").
    ImplementationDetail,
    /// Same feature appears twice on the same entity.
    Duplicate,
}

impl fmt::Display for QualityIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QualityIssue::TooShort => write!(f, "too short (< 2 words)"),
            QualityIssue::TooLong => write!(f, "too long (> 10 words)"),
            QualityIssue::VagueVerb(verb) => {
                write!(f, "vague verb \"{verb}\" — use a more specific action")
            }
            QualityIssue::ImplementationDetail => {
                write!(
                    f,
                    "contains implementation detail — describe intent, not mechanism"
                )
            }
            QualityIssue::Duplicate => write!(f, "duplicate feature on same entity"),
        }
    }
}

const VAGUE_VERBS: &[&str] = &[
    "handle", "process", "manage", "deal", "do", "run", "execute", "perform", "work", "utilize",
];

const IMPL_DETAIL_WORDS: &[&str] = &[
    "loop",
    "iterate",
    "array",
    "index",
    "variable",
    "pointer",
    "mutex",
    "allocate",
    "deallocate",
    "malloc",
    "free",
    "increment",
    "decrement",
];

/// Critique a set of features for a single entity.
///
/// Returns warnings for each quality issue found. Returns empty vec if no issues.
pub fn critique(entity_id: &str, features: &[String]) -> Vec<QualityWarning> {
    let mut warnings = Vec::new();

    // Check for duplicates
    let mut seen = HashSet::new();
    for feat in features {
        if !seen.insert(feat.to_lowercase()) {
            warnings.push(QualityWarning {
                entity_id: entity_id.to_string(),
                feature: feat.clone(),
                issue: QualityIssue::Duplicate,
                suggestion: Some("remove the duplicate".to_string()),
            });
        }
    }

    for feat in features {
        let words: Vec<&str> = feat.split_whitespace().collect();

        // Too short
        if words.len() < 2 {
            warnings.push(QualityWarning {
                entity_id: entity_id.to_string(),
                feature: feat.clone(),
                issue: QualityIssue::TooShort,
                suggestion: Some("use verb-object form, e.g. \"validate input\"".to_string()),
            });
            continue;
        }

        // Too long
        if words.len() > 10 {
            warnings.push(QualityWarning {
                entity_id: entity_id.to_string(),
                feature: feat.clone(),
                issue: QualityIssue::TooLong,
                suggestion: Some("split into multiple atomic features".to_string()),
            });
        }

        // Vague verb (check first word)
        let first_word = words[0].to_lowercase();
        if let Some(verb) = VAGUE_VERBS.iter().find(|&&v| v == first_word) {
            warnings.push(QualityWarning {
                entity_id: entity_id.to_string(),
                feature: feat.clone(),
                issue: QualityIssue::VagueVerb(verb.to_string()),
                suggestion: Some(format!(
                    "replace \"{}\" with a specific verb (validate, parse, compute, etc.)",
                    verb
                )),
            });
        }

        // Implementation detail
        let lower = feat.to_lowercase();
        if IMPL_DETAIL_WORDS
            .iter()
            .any(|w| lower.split_whitespace().any(|word| word == *w))
        {
            warnings.push(QualityWarning {
                entity_id: entity_id.to_string(),
                feature: feat.clone(),
                issue: QualityIssue::ImplementationDetail,
                suggestion: Some("describe intent, not mechanism".to_string()),
            });
        }
    }

    warnings
}

/// Format quality warnings as a markdown section for inclusion in tool output.
pub fn format_warnings(warnings: &[QualityWarning]) -> String {
    if warnings.is_empty() {
        return String::new();
    }

    // Group by entity
    let mut by_entity: std::collections::BTreeMap<&str, Vec<&QualityWarning>> =
        std::collections::BTreeMap::new();
    for w in warnings {
        by_entity.entry(&w.entity_id).or_default().push(w);
    }

    let entity_count = by_entity.len();
    let mut out = format!(
        "\n## QUALITY\n\n{} entit{} with feature quality warnings:\n",
        entity_count,
        if entity_count == 1 { "y" } else { "ies" },
    );

    for (eid, ws) in &by_entity {
        for w in ws {
            if let Some(ref suggestion) = w.suggestion {
                out.push_str(&format!(
                    "- `{}` → \"{}\" — {}. {}\n",
                    eid, w.feature, w.issue, suggestion
                ));
            } else {
                out.push_str(&format!("- `{}` → \"{}\" — {}\n", eid, w.feature, w.issue));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_critique_vague_verb() {
        let warnings = critique("src/lib.rs:foo", &["handle data".to_string()]);
        assert!(!warnings.is_empty());
        assert!(
            warnings
                .iter()
                .any(|w| matches!(&w.issue, QualityIssue::VagueVerb(v) if v == "handle"))
        );
    }

    #[test]
    fn test_critique_too_short() {
        let warnings = critique("src/lib.rs:foo", &["auth".to_string()]);
        assert!(warnings.iter().any(|w| w.issue == QualityIssue::TooShort));
    }

    #[test]
    fn test_critique_too_long() {
        let feature = "this is a very long feature description that has way too many words in it";
        let warnings = critique("src/lib.rs:foo", &[feature.to_string()]);
        assert!(warnings.iter().any(|w| w.issue == QualityIssue::TooLong));
    }

    #[test]
    fn test_critique_implementation_detail() {
        let warnings = critique("src/lib.rs:foo", &["loop through results".to_string()]);
        assert!(
            warnings
                .iter()
                .any(|w| w.issue == QualityIssue::ImplementationDetail)
        );
    }

    #[test]
    fn test_critique_duplicate() {
        let warnings = critique(
            "src/lib.rs:foo",
            &[
                "validate user credentials".to_string(),
                "validate user credentials".to_string(),
            ],
        );
        assert!(warnings.iter().any(|w| w.issue == QualityIssue::Duplicate));
    }

    #[test]
    fn test_critique_good_features() {
        let warnings = critique(
            "src/lib.rs:foo",
            &[
                "validate user credentials".to_string(),
                "return authentication token".to_string(),
            ],
        );
        assert!(
            warnings.is_empty(),
            "good features should produce no warnings"
        );
    }

    #[test]
    fn test_format_warnings_empty() {
        let result = format_warnings(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_warnings_nonempty() {
        let warnings = vec![QualityWarning {
            entity_id: "src/lib.rs:foo".to_string(),
            feature: "handle data".to_string(),
            issue: QualityIssue::VagueVerb("handle".to_string()),
            suggestion: Some("replace with a specific verb".to_string()),
        }];
        let result = format_warnings(&warnings);
        assert!(result.contains("## QUALITY"));
        assert!(result.contains("handle data"));
        assert!(result.contains("vague verb"));
    }
}
