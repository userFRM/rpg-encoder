use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn php_use_declaration() {
    let source = r"<?php
use App\Models\User;

class Foo { }
";
    let deps = extract_deps(Path::new("Foo.php"), source, Language::PHP);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, r"App\Models\User");
}

#[test]
fn php_class_extends() {
    let source = "<?php class Dog extends Animal { }";
    let deps = extract_deps(Path::new("Dog.php"), source, Language::PHP);
    assert!(!deps.inherits.is_empty());
    let inherit = &deps.inherits[0];
    assert_eq!(inherit.child_class, "Dog");
    assert_eq!(inherit.parent_class, "Animal");
}

#[test]
fn php_class_implements() {
    let source = "<?php class Foo implements Bar { }";
    let deps = extract_deps(Path::new("Foo.php"), source, Language::PHP);
    assert!(!deps.inherits.is_empty());
    let inherit = deps
        .inherits
        .iter()
        .find(|i| i.parent_class == "Bar")
        .unwrap();
    assert_eq!(inherit.child_class, "Foo");
    assert_eq!(inherit.parent_class, "Bar");
}

#[test]
fn php_grouped_use_statement() {
    let source = r"<?php
use Foo\{Bar, Baz};

class MyClass { }
";
    let deps = extract_deps(Path::new("MyClass.php"), source, Language::PHP);
    assert!(!deps.imports.is_empty());
    // The parser captures the grouped use as a single import with the full text
    let import = &deps.imports[0];
    assert!(
        import.module.contains("Foo"),
        "expected module to contain 'Foo', got: {:?}",
        import.module
    );
}
