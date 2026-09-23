use super::*;

#[test]
fn call_search_finds_syntactic_calls() {
    let rust = concat!(
        "fn foo() {}\n",
        "fn run(x: X) {\n",
        "    foo();\n",
        "    x.foo();\n",
        "    a::foo();\n",
        "    foo::<u8>();\n",
        "    foo!();\n",
        "    // foo()\n",
        "    let s = \"foo()\";\n",
        "    foobar();\n",
        "}\n",
    );
    assert_eq!(sites(rust, Grammar::Rust, "foo", &mut || Duration::ZERO), Some(vec![3, 4, 5, 6, 7]));
    let javascript = "new Foo();\nFoo.bar();\n";
    assert_eq!(sites(javascript, Grammar::JavaScript, "Foo", &mut || Duration::ZERO), Some(vec![1]));
}

#[test]
fn python_and_c_calls_match_by_last_identifier() {
    let python = "obj.foo()\nfoo()\nfoo_x()\n";
    assert_eq!(sites(python, Grammar::Python, "foo", &mut || Duration::ZERO), Some(vec![1, 2]));
    let c = "void run(struct s *p) {\n  foo();\n  p->foo();\n}\n";
    assert_eq!(sites(c, Grammar::C, "foo", &mut || Duration::ZERO), Some(vec![2, 3]));
}

#[test]
fn only_rust_javascript_python_and_c_files_have_a_call_grammar() {
    for name in ["a.rs", "a.js", "a.py", "a.c"] {
        assert!(call_grammar(Path::new(name)).is_some(), "{name}");
    }
    for name in ["a.md", "a.json", "a.txt"] {
        assert!(call_grammar(Path::new(name)).is_none(), "{name}");
    }
}
