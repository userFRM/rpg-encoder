# Task Instructions

You are implementing code for a code generation system.

## Task Context

The following information is provided for each task:

- **Entity**: The planned entity ID (e.g., `src/auth.rs:login`)
- **File**: The target file path
- **Kind**: Type of entity (function, struct, impl, etc.)
- **Skeleton**: The planned signature and structure
- **Semantic Features**: Verb-object phrases describing behavior
- **Dependencies**: Already-generated code you can reference

## Implementation Guidelines

1. **Follow the skeleton exactly**: Match the planned signature, parameter names, and return type
2. **Implement all semantic features**: Each feature should be reflected in the code
3. **Use dependency context**: Reference already-generated code for imports and calls
4. **Follow language conventions**: Use idiomatic patterns for the target language
5. **Include error handling**: Handle errors appropriate to the complexity level
6. **Add documentation**: Doc comments should match the semantic features

## Output Format

Return **ONLY** the code for this entity, properly formatted.

- Do **not** include file-level imports unless this is a `CreateFile` task
- Do **not** include unrelated code
- Do **not** explain the code — just provide the implementation

## Examples

### Function Task

**Task**: Generate `validate_email` function
**Skeleton**: `fn validate_email(email: &str) -> Result<(), ValidationError>`
**Features**: ["validate email format", "check for common typos"]

**Output**:
```rust
/// Validate email format and check for common typos.
pub fn validate_email(email: &str) -> Result<(), ValidationError> {
    // Check basic format
    if !email.contains('@') {
        return Err(ValidationError::MissingAt);
    }

    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return Err(ValidationError::InvalidFormat);
    }

    let domain = parts[1];

    // Check for common typos
    let typo_domains = ["gmial.com", "gmal.com", "hotmal.com"];
    if typo_domains.contains(&domain) {
        return Err(ValidationError::PossibleTypo(domain.to_string()));
    }

    Ok(())
}
```

### Struct Task

**Task**: Generate `User` struct
**Skeleton**: `struct User { id, name, email, created_at }`
**Features**: ["store user identity", "track creation time"]

**Output**:
```rust
/// A user in the system.
///
/// Stores user identity and tracks when the user was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user identifier.
    pub id: UserId,
    /// User's display name.
    pub name: String,
    /// User's email address.
    pub email: String,
    /// When the user was created.
    pub created_at: DateTime<Utc>,
}
```

## Complexity Guidelines

- **Trivial**: Single expression, delegation, getter/setter
- **Simple**: Linear logic, no branches, straightforward
- **Moderate**: Branches, loops, multiple steps, validation
- **Complex**: State management, recursion, complex algorithms

Match your implementation complexity to the task's estimated complexity.
