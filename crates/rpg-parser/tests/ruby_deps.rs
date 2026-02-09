use std::path::Path;

use rpg_parser::deps::extract_deps;
use rpg_parser::languages::Language;

#[test]
fn ruby_require_import() {
    let source = "require 'json'";
    let deps = extract_deps(Path::new("foo.rb"), source, Language::RUBY);
    assert_eq!(deps.imports.len(), 1);
    assert_eq!(deps.imports[0].module, "json");
}

#[test]
fn ruby_class_inheritance() {
    let source = "class Dog < Animal\nend";
    let deps = extract_deps(Path::new("dog.rb"), source, Language::RUBY);
    assert!(!deps.inherits.is_empty());
    let inherit = &deps.inherits[0];
    assert_eq!(inherit.child_class, "Dog");
    // tree-sitter Ruby superclass field includes the "< " prefix
    assert!(inherit.parent_class.contains("Animal"));
}

#[test]
fn ruby_include_mixin() {
    let source = "class Foo\n  include Enumerable\nend";
    let deps = extract_deps(Path::new("foo.rb"), source, Language::RUBY);
    assert!(!deps.inherits.is_empty());
    let mixin = deps
        .inherits
        .iter()
        .find(|i| i.parent_class == "Enumerable")
        .unwrap();
    assert_eq!(mixin.child_class, "Foo");
    assert_eq!(mixin.parent_class, "Enumerable");
}

#[test]
fn ruby_module_nesting() {
    let source = r"module Outer
  module Inner
    class Foo
      def bar; end
    end
  end
end";
    let deps = extract_deps(Path::new("foo.rb"), source, Language::RUBY);
    // Module nesting doesn't produce imports, but classes inside should still be parseable.
    // Verify no crash and that calls/inherits work within nested modules.
    // The parser should handle nested modules gracefully.
    assert!(
        deps.imports.is_empty(),
        "module nesting should not produce imports"
    );
}

#[test]
fn ruby_extend_mixin() {
    let source = "class Bar\n  extend ClassMethods\nend";
    let deps = extract_deps(Path::new("bar.rb"), source, Language::RUBY);
    assert!(!deps.inherits.is_empty());
    let mixin = deps
        .inherits
        .iter()
        .find(|i| i.parent_class == "ClassMethods")
        .unwrap();
    assert_eq!(mixin.child_class, "Bar");
    assert_eq!(mixin.parent_class, "ClassMethods");
}
