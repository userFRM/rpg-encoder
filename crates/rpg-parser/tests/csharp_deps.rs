use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn csharp_using_directive() {
    let source = r"using System.Collections.Generic;

public class Foo { }
";
    let deps = extract_deps(Path::new("Foo.cs"), source, Language::CSHARP);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "System.Collections.Generic");
}

#[test]
fn csharp_class_inheritance() {
    let source = "public class Dog : Animal { }";
    let deps = extract_deps(Path::new("Dog.cs"), source, Language::CSHARP);
    assert!(!deps.inherits.is_empty());
    let inherit = &deps.inherits[0];
    assert_eq!(inherit.child_class, "Dog");
    assert_eq!(inherit.parent_class, "Animal");
}

#[test]
fn csharp_method_invocation() {
    let source = r#"public class Foo {
    public void Bar() {
        Console.WriteLine("hi");
    }
}
"#;
    let deps = extract_deps(Path::new("Foo.cs"), source, Language::CSHARP);
    assert!(!deps.calls.is_empty());
    let call = deps.calls.iter().find(|c| c.callee == "WriteLine").unwrap();
    assert_eq!(call.callee, "WriteLine");
}

#[test]
fn csharp_global_using() {
    let source = "global using System.Text;\n\npublic class Foo { }";
    let deps = extract_deps(Path::new("Foo.cs"), source, Language::CSHARP);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "System.Text");
}

#[test]
fn csharp_static_using() {
    let source = "using static System.Math;\n\npublic class Foo { }";
    let deps = extract_deps(Path::new("Foo.cs"), source, Language::CSHARP);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "System.Math");
}
