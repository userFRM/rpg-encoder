# TDD Task Instructions

You are implementing code using a test-driven development (TDD) workflow.

## Task Context

The following information is provided for each task:

- **Entity**: The planned entity ID (e.g., `src/auth.rs:login`)
- **File**: The target file path
- **Kind**: Type of entity (function, struct, impl, etc.)
- **Skeleton**: The planned signature and structure
- **Semantic Features**: Verb-object phrases describing behavior
- **Dependencies**: Already-generated code you can reference (with source)

## TDD Workflow

For each task, follow this loop:

### 1. Write Tests First

Create tests that validate the expected behavior described by the semantic features.

**Rust:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_email_accepts_valid() {
        assert!(validate_email("user@example.com").is_ok());
    }

    #[test]
    fn test_validate_email_rejects_missing_at() {
        assert!(validate_email("invalid").is_err());
    }
}
```

**Python:**
```python
def test_validate_email_accepts_valid():
    assert validate_email("user@example.com") is True

def test_validate_email_rejects_missing_at():
    with pytest.raises(ValidationError):
        validate_email("invalid")
```

**TypeScript:**
```typescript
describe("validateEmail", () => {
  it("accepts valid email", () => {
    expect(validateEmail("user@example.com")).toBe(true);
  });

  it("rejects missing @", () => {
    expect(() => validateEmail("invalid")).toThrow();
  });
});
```

### 2. Implement the Code

Write the minimal implementation that should make all tests pass. Follow the skeleton signature exactly.

### 3. Run Tests

Execute the test suite using the appropriate runner:
- **Rust**: `cargo test -p <crate> -- <test_name>`
- **Python**: `pytest <test_file> -v`
- **TypeScript**: `npx jest <test_file>` or `npx vitest <test_file>`
- **Go**: `go test ./... -run <TestName>`

### 4. Report Outcome

Call `report_task_outcome` with the result:

**If tests pass:**
```json
{"task_id": "src/auth.rs:login", "outcome": "{\"kind\":\"pass\"}", "file_path": "src/auth.rs"}
```

**If tests fail (code is buggy):**
```json
{
  "task_id": "src/auth.rs:login",
  "outcome": "{\"kind\":\"test_failure\",\"failing_count\":2,\"summary\":\"expected Ok but got Err\"}",
  "test_results": "{\"total\":5,\"passed\":3,\"failed\":2,\"test_file\":\"src/auth_test.rs\"}"
}
```

**If code won't compile:**
```json
{
  "task_id": "src/auth.rs:login",
  "outcome": "{\"kind\":\"code_error\",\"error_message\":\"cannot find type User in scope\"}"
}
```

**If test code is broken:**
```json
{
  "task_id": "src/auth.rs:login",
  "outcome": "{\"kind\":\"test_error\",\"error_message\":\"test setup failed: missing fixture\"}"
}
```

### 5. Follow Routing

The tool response tells you what to do next:

| Routing | Meaning | Action |
|---------|---------|--------|
| `PASS` | All good | Move to the next task |
| `FIX_CODE` | Tests correct, code buggy | Fix the implementation, re-run tests, report again |
| `FIX_TEST` | Test code is broken | Fix the test, re-run, report again |
| `ENV_ERROR` | Environment issue | Fix environment (install tool, set permission), retry |
| `FAILED` | Max retries exceeded | Move to the next task (this one is flagged for review) |

## Implementation Guidelines

1. **Follow the skeleton exactly**: Match the planned signature, parameter names, and return type
2. **Implement all semantic features**: Each feature should be reflected in the code
3. **Use dependency context**: Reference already-generated code for imports and calls
4. **Follow language conventions**: Use idiomatic patterns for the target language
5. **Include error handling**: Handle errors appropriate to the complexity level
6. **Write meaningful tests**: Each semantic feature should have at least one test

## Test Placement

- **Rust**: Inline `#[cfg(test)] mod tests` in the same file, or `tests/` directory for integration tests
- **Python**: `test_<module>.py` in the same directory or `tests/` directory
- **TypeScript/JavaScript**: `<module>.test.ts` co-located or in `__tests__/` directory
- **Go**: `<module>_test.go` in the same package

## Complexity Guidelines

- **Trivial**: Single expression, delegation, getter/setter — 1 test sufficient
- **Simple**: Linear logic, no branches — 2-3 tests (happy path + edge case)
- **Moderate**: Branches, loops, validation — 3-5 tests covering each branch
- **Complex**: State management, algorithms — 5+ tests including error cases
