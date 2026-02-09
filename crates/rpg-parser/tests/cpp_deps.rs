use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;
use std::path::Path;

#[test]
fn test_cpp_include() {
    let source = "#include <iostream>\n";
    let deps = extract_deps(Path::new("test.cpp"), source, Language::CPP);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "iostream");
    assert!(deps.imports[0].symbols.is_empty());
}

#[test]
fn test_cpp_class_inheritance() {
    let source = "\
class Animal {};
class Dog : public Animal {};
";
    let deps = extract_deps(Path::new("test.cpp"), source, Language::CPP);
    assert_eq!(deps.inherits.len(), 1);
    assert_eq!(deps.inherits[0].child_class, "Dog");
    assert_eq!(deps.inherits[0].parent_class, "Animal");
}

#[test]
fn test_cpp_multiple_inheritance() {
    let source = "\
class Bar {};
class Baz {};
class Foo : public Bar, protected Baz {};
";
    let deps = extract_deps(Path::new("test.cpp"), source, Language::CPP);
    assert_eq!(deps.inherits.len(), 2);
    assert!(
        deps.inherits
            .iter()
            .any(|i| i.child_class == "Foo" && i.parent_class == "Bar")
    );
    assert!(
        deps.inherits
            .iter()
            .any(|i| i.child_class == "Foo" && i.parent_class == "Baz")
    );
}
