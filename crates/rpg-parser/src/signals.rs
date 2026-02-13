//! Structural signal extraction from source text.
//!
//! Language-agnostic regex heuristics for counting branches, loops, early returns,
//! and function calls in entity source code. Used for confidence-gated auto-lift.

use regex::Regex;
use std::sync::OnceLock;

/// Structural complexity signals extracted from source text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StructuralSignals {
    /// Total line count.
    pub line_count: usize,
    /// Branch points: if, else, match, switch, case, when, elif, elsif.
    pub branch_count: usize,
    /// Loop constructs: for, while, loop, each, do { }.
    pub loop_count: usize,
    /// Return statements not on the last line.
    pub early_return_count: usize,
    /// Function/method calls: identifier followed by `(`.
    pub call_count: usize,
}

/// Compute structural signals from raw source text.
///
/// Uses regex heuristics — not a formal parser. Good enough for triage/confidence
/// scoring, not for exact analysis. Comments and strings may trigger false counts.
pub fn analyze(source: &str) -> StructuralSignals {
    static BRANCH_RE: OnceLock<Regex> = OnceLock::new();
    static LOOP_RE: OnceLock<Regex> = OnceLock::new();
    static RETURN_RE: OnceLock<Regex> = OnceLock::new();
    static CALL_RE: OnceLock<Regex> = OnceLock::new();

    let branch_re = BRANCH_RE
        .get_or_init(|| Regex::new(r"\b(if|else|match|switch|case|when|elif|elsif)\b").unwrap());
    let loop_re = LOOP_RE.get_or_init(|| Regex::new(r"\b(for|while|loop|each)\b").unwrap());
    let return_re = RETURN_RE.get_or_init(|| Regex::new(r"\breturn\b").unwrap());
    let call_re = CALL_RE.get_or_init(|| Regex::new(r"\b[a-zA-Z_]\w*\s*\(").unwrap());

    let lines: Vec<&str> = source.lines().collect();
    let line_count = lines.len();

    let branch_count = branch_re.find_iter(source).count();
    let loop_count = loop_re.find_iter(source).count();
    let call_count = call_re.find_iter(source).count();

    // Early returns: return statements not on the last non-empty line
    let last_non_empty = lines.iter().rposition(|l| !l.trim().is_empty());
    let early_return_count = lines
        .iter()
        .enumerate()
        .filter(|(i, line)| return_re.is_match(line) && Some(*i) != last_non_empty)
        .count();

    StructuralSignals {
        line_count,
        branch_count,
        loop_count,
        early_return_count,
        call_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signals_simple_getter() {
        let source = "fn get_name(&self) -> &str { &self.name }";
        let signals = analyze(source);
        assert_eq!(signals.line_count, 1);
        assert_eq!(signals.branch_count, 0);
        assert_eq!(signals.loop_count, 0);
        assert_eq!(signals.early_return_count, 0);
    }

    #[test]
    fn test_signals_branching_function() {
        let source = r"
fn validate(x: i32) -> bool {
    if x > 0 {
        if x < 100 {
            return true;
        } else {
            return false;
        }
    }
    match x {
        -1 => true,
        _ => false,
    }
}";
        let signals = analyze(source);
        // if, if, else, match = 4 branches
        assert_eq!(signals.branch_count, 4);
        assert!(signals.early_return_count >= 1);
    }

    #[test]
    fn test_signals_loops() {
        let source = r"
fn process(items: &[Item]) {
    for item in items {
        while item.needs_retry() {
            item.process();
        }
    }
}";
        let signals = analyze(source);
        assert_eq!(signals.loop_count, 2); // for + while
        assert_eq!(signals.branch_count, 0);
    }

    #[test]
    fn test_signals_complex() {
        let source = r#"
fn handle(req: Request) -> Response {
    if req.is_valid() {
        for header in req.headers() {
            if header.is_auth() {
                match header.scheme() {
                    "bearer" => return handle_bearer(header),
                    "basic" => return handle_basic(header),
                    _ => {}
                }
            }
        }
        while retry_count < 3 {
            retry_count += 1;
        }
    } else {
        return Response::bad_request();
    }
    Response::ok()
}"#;
        let signals = analyze(source);
        assert!(signals.branch_count >= 4); // if, if, match, else
        assert!(signals.loop_count >= 2); // for, while
        assert!(signals.call_count >= 3);
    }

    #[test]
    fn test_signals_empty() {
        let signals = analyze("");
        assert_eq!(
            signals,
            StructuralSignals {
                line_count: 0,
                branch_count: 0,
                loop_count: 0,
                early_return_count: 0,
                call_count: 0,
            }
        );
    }

    #[test]
    fn test_signals_calls() {
        let source = "fn setup() { init(); configure(); start(); }";
        let signals = analyze(source);
        // setup(), init(), configure(), start() = 4 calls
        assert!(signals.call_count >= 3);
    }

    #[test]
    fn test_signals_early_return() {
        let source = r"fn check(x: i32) -> bool {
    if x < 0 {
        return false;
    }
    true
}";
        let signals = analyze(source);
        assert_eq!(signals.early_return_count, 1);
    }

    #[test]
    fn test_signals_return_before_closing_brace_is_early() {
        // "return 42;" is on line index 1, closing "}" is line index 2 (last non-empty).
        // Since the return is not on the last non-empty line, it counts as early.
        let source = r"fn get_value() -> i32 {
    return 42;
}";
        let signals = analyze(source);
        assert_eq!(signals.early_return_count, 1);
    }

    #[test]
    fn test_signals_return_on_last_line_not_early() {
        // When the return IS on the last non-empty line, it doesn't count as early.
        let source = "fn get_value() -> i32 { return 42; }";
        let signals = analyze(source);
        assert_eq!(signals.early_return_count, 0);
    }
}
