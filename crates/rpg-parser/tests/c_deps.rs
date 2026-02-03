use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_c_include_angle_bracket() {
    let source = "#include <stdio.h>\n";
    let deps = extract_deps(Path::new("test.c"), source, Language::C);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "stdio.h");
    assert!(deps.imports[0].symbols.is_empty());
}

#[test]
fn test_c_include_quoted() {
    let source = "#include \"myheader.h\"\n";
    let deps = extract_deps(Path::new("test.c"), source, Language::C);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "myheader.h");
    assert!(deps.imports[0].symbols.is_empty());
}

#[test]
fn test_c_function_call() {
    let source = "\
#include <stdio.h>
int main() {
    printf(\"hi\");
    return 0;
}
";
    let deps = extract_deps(Path::new("test.c"), source, Language::C);
    assert!(
        deps.calls
            .iter()
            .any(|c| c.callee == "printf" && c.caller_entity == "main")
    );
}
