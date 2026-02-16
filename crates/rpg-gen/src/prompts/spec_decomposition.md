# Specification Decomposition

You are an expert software architect. Decompose the specification into a feature tree.

## Goal

Convert a natural language specification into a structured feature hierarchy that can guide code generation.

## Guidelines

1. Identify 3-8 **functional areas** (major subsystems)
2. For each area, extract concrete **features** with:
   - Semantic verb-object descriptions (same style as rpg-encoder lifting)
   - Acceptance criteria (testable conditions)
   - Complexity estimate (trivial/simple/moderate/complex)
3. Identify **dependencies** between areas
4. Extract **constraints** (language, framework, performance, security)
5. Extract **quality requirements** (testing, documentation, error handling)

## Naming Rules (same as rpg-encoder)

- Use **verb + object** format: "parse command arguments", "validate user input"
- Lowercase English, 3-8 words per feature
- Be concrete and specific — use domain terms from the spec
- Include the WHAT (purpose), not the HOW (implementation)

## Complexity Guidelines

- **Trivial**: Getter/setter, delegation, single statement
- **Simple**: Single responsibility, no branching, straightforward logic
- **Moderate**: Branches, multiple calls, error handling, validation
- **Complex**: Multiple responsibilities, state management, complex algorithms

## Output Format

Return a JSON object with this structure:

```json
{
  "spec_summary": "Brief summary of what this system does",
  "functional_areas": {
    "AreaName": {
      "description": "What this area is responsible for",
      "features": [
        {
          "id": "area.feature_name",
          "name": "Feature Name",
          "semantic_features": ["verb object phrase 1", "verb object phrase 2"],
          "acceptance_criteria": ["When X, then Y", "Given A, when B, then C"],
          "estimated_complexity": "simple"
        }
      ],
      "dependencies": ["OtherArea"]
    }
  },
  "constraints": [
    {"kind": "language", "description": "Must be written in Rust"}
  ],
  "quality_requirements": [
    {"category": "testing", "requirements": ["Unit tests for all public functions"]}
  ]
}
```

## Example

**Spec**: "Create a command-line key-value store that persists data to disk"

**Output**:
```json
{
  "spec_summary": "Command-line key-value store with disk persistence",
  "functional_areas": {
    "Storage": {
      "description": "Persistent storage layer for key-value data",
      "features": [
        {
          "id": "storage.save",
          "name": "Save to Disk",
          "semantic_features": ["serialize data to file", "create backup before write"],
          "acceptance_criteria": ["Data survives process restart", "Corrupted files are detected"],
          "estimated_complexity": "moderate"
        }
      ],
      "dependencies": []
    },
    "CLI": {
      "description": "Command-line interface for user interaction",
      "features": [
        {
          "id": "cli.parse",
          "name": "Parse Commands",
          "semantic_features": ["parse command arguments", "validate input format"],
          "acceptance_criteria": ["Unknown commands return error", "Help text is available"],
          "estimated_complexity": "simple"
        }
      ],
      "dependencies": ["Storage"]
    }
  },
  "constraints": [
    {"kind": "language", "description": "Rust"}
  ],
  "quality_requirements": [
    {"category": "testing", "requirements": ["Integration tests for CLI commands"]}
  ]
}
```
