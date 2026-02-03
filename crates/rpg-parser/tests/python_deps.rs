use rpg_parser::deps::extract_python_deps;
use std::path::Path;

#[test]
fn test_import_statement() {
    let source = "import os\n";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "os");
    assert!(deps.imports[0].symbols.is_empty());
}

#[test]
fn test_from_import() {
    let source = "from os.path import join, exists\n";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "os.path");
    assert_eq!(deps.imports[0].symbols.len(), 2);
    assert!(deps.imports[0].symbols.contains(&"join".to_string()));
    assert!(deps.imports[0].symbols.contains(&"exists".to_string()));
}

#[test]
fn test_import_with_alias() {
    let source = "import numpy as np\n";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "numpy");
}

#[test]
fn test_inheritance() {
    let source = "\
class Animal:
    pass

class Dog(Animal):
    pass
";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert_eq!(deps.inherits.len(), 1);
    assert_eq!(deps.inherits[0].child_class, "Dog");
    assert_eq!(deps.inherits[0].parent_class, "Animal");
}

#[test]
fn test_multiple_inheritance() {
    let source = "\
class A:
    pass

class B:
    pass

class C(A, B):
    pass
";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert_eq!(deps.inherits.len(), 2);
    assert!(
        deps.inherits
            .iter()
            .any(|i| i.child_class == "C" && i.parent_class == "A")
    );
    assert!(
        deps.inherits
            .iter()
            .any(|i| i.child_class == "C" && i.parent_class == "B")
    );
}

#[test]
fn test_function_calls() {
    let source = "\
def helper():
    pass

def main():
    helper()
";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert!(!deps.calls.is_empty());
    assert!(
        deps.calls
            .iter()
            .any(|c| c.callee == "helper" && c.caller_entity == "main")
    );
}

#[test]
fn test_method_calls() {
    let source = "\
class Foo:
    def bar(self):
        self.baz()

    def baz(self):
        pass
";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert!(
        deps.calls
            .iter()
            .any(|c| c.callee == "baz" && c.caller_entity == "Foo.bar")
    );
}

#[test]
fn test_module_level_call() {
    let source = "print('hello')\n";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert!(
        deps.calls
            .iter()
            .any(|c| c.callee == "print" && c.caller_entity == "<module>")
    );
}

#[test]
fn test_empty_file() {
    let source = "";
    let deps = extract_python_deps(Path::new("test.py"), source);
    assert!(deps.imports.is_empty());
    assert!(deps.calls.is_empty());
    assert!(deps.inherits.is_empty());
}
