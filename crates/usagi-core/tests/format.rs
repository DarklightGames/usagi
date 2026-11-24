use usagi_core::format::equivalent;
use usagi_core::parse_program;

fn program(source: &str) -> usagi_core::ast::Program {
    parse_program(source)
        .unwrap_or_else(|e| panic!("{source:?} should parse: {e:?}"))
        .program
}

/// Whether two class bodies are the same program, once formatted.
fn same(a: &str, b: &str) -> bool {
    let class = "class A extends Object;\n";
    equivalent(
        program(&format!("{class}{a}")),
        program(&format!("{class}{b}")),
    )
}

/// Whether two function bodies are the same program, once formatted.
fn same_code(a: &str, b: &str) -> bool {
    same(
        &format!("function F()\n{{\n{a}\n}}\n"),
        &format!("function F()\n{{\n{b}\n}}\n"),
    )
}

#[test]
fn layout_is_ignored() {
    assert!(same(
        "function F(){local int I;I=1;if(I>0){I++;}}",
        "function F()\n{\n    local int I;\n\n    I = 1;\n\n    if (I > 0)\n    {\n        I++;\n    }\n}\n",
    ));
    assert!(same("var   int  X ;", "var int X;"));
    assert!(same("var ( Display ) int X;", "var(Display) int X;"));
}

#[test]
fn case_of_keywords_and_identifiers_is_ignored() {
    assert!(same(
        "Function F() { Local Int I; If (I == None) Return; I = TRUE; Super.F(); }",
        "function F() { local int I; if (I == none) return; I = true; super.F(); }",
    ));
    assert!(same("var Class<Actor> C;", "var class<Actor> C;"));
    assert!(same_code("C = class'Actor';", "C = Class'Actor';"));
}

#[test]
fn comments_are_ignored() {
    // They're compared separately.
    assert!(same_code("A = 1; // one", "A = 1;"));
}

#[test]
fn code_is_compared() {
    assert!(!same_code("A = 1;", "A = 2;"));
    assert!(!same_code("A = 1;", "B = 1;"));
    assert!(!same_code("A = B + C;", "A = B - C;"));
    assert!(!same_code("A = 1; B = 2;", "B = 2; A = 1;"));
    assert!(!same_code("A = \"x\";", "A = \"X\";"));
    assert!(!same_code("A = 'x';", "A = 'y';"));
    assert!(!same("var int X;", "var float X;"));
    assert!(!same("var int X;", "var config int X;"));
    assert!(!same("function F();", "function G();"));
}

#[test]
fn parentheses_are_kept() {
    assert!(!same_code("A = (B + C) * D;", "A = B + C * D;"));
    assert!(!same_code("A = (B);", "A = B;"));
    assert!(!same_code("A = B + C * D;", "A = B + (C * D);"));
}

#[test]
fn modifiers_may_be_sorted() {
    assert!(same(
        "simulated static final function F();",
        "final simulated static function F();"
    ));
    assert!(same(
        "var transient config int X;",
        "var config transient int X;"
    ));
    assert!(!same("var config int X;", "var transient int X;"));
    assert!(same(
        "function F(out optional int X);",
        "function F(optional out int X);"
    ));
    assert!(same(
        "simulated auto state S {}",
        "auto simulated state S {}"
    ));
}

#[test]
fn class_modifiers_may_be_sorted() {
    let a = "class A extends Object\n    notplaceable\n    abstract;\n";
    let b = "class A extends Object\n    abstract\n    notplaceable;\n";
    assert!(equivalent(program(a), program(b)));
    let c = "class A extends Object\n    abstract;\n";
    assert!(!equivalent(program(a), program(c)));
}

#[test]
fn struct_modifiers_may_be_sorted() {
    assert!(same(
        "struct native export S { var transient config int X; };",
        "struct export native S { var config transient int X; };",
    ));
    assert!(same(
        "var struct native export S { var() transient config int X; } V;",
        "var struct export native S { var() config transient int X; } V;",
    ));
    assert!(same(
        "var array<struct native export S { var int X; }> V;",
        "var array<struct export native S { var int X; }> V;",
    ));
}

#[test]
fn braces_may_be_added() {
    assert!(same_code("if (A) B();", "if (A) { B(); }"));
    assert!(same_code(
        "if (A) B(); else if (C) D(); else E();",
        "if (A) { B(); } else if (C) { D(); } else { E(); }",
    ));
    assert!(same_code(
        "for (I = 0; I < 3; I++) B();",
        "for (I = 0; I < 3; I++) { B(); }"
    ));
    assert!(same_code(
        "foreach AllActors(class'Actor', A) B();",
        "foreach AllActors(class'Actor', A) { B(); }"
    ));
    assert!(same_code("while (A) B();", "while (A) { B(); }"));
    assert!(same_code("do B(); until (A);", "do { B(); } until (A);"));
    assert!(same_code(
        "if (A) if (B) C();",
        "if (A) { if (B) { C(); } }"
    ));
    assert!(!same_code("if (A) { B(); C(); }", "if (A) B(); C();"));
}

#[test]
fn an_if_without_a_body_may_get_an_empty_one() {
    assert!(same_code("if (A)\n// B();\n", "if (A) {}"));
    assert!(!same_code("if (A)\n", "if (B) {}"));
}

#[test]
fn stray_semicolons_may_be_removed() {
    assert!(same("var int X;;", "var int X;"));
    assert!(same("function F() {};", "function F() {}"));
    assert!(same_code("A = 1;;", "A = 1;"));
    assert!(same_code("if (A) ;", "if (A) {}"));
    assert!(same_code("if (A) { ; }", "if (A) {}"));
    assert!(same_code(
        "switch (A) { case 1: ; B(); }",
        "switch (A) { case 1: B(); }"
    ));
    assert!(same("state S { Begin: ; A(); }", "state S { Begin: A(); }"));
    assert!(same(
        "state S { const C = 1; Begin: ; A(); }",
        "state S { const C = 1; Begin: A(); }"
    ));
    assert!(same(
        "state S { function F() { ; A(); } }",
        "state S { function F() { A(); } }"
    ));
}

#[test]
fn literals_are_compared_by_value() {
    assert!(same_code("A = 1.;", "A = 1.0;"));
    assert!(same_code("A = .5;", "A = 0.5;"));
    assert!(same_code("A = 0x10;", "A = 0X10;"));
    // Recovered: read as `0.0`.
    assert!(same_code("A = 0.0.5;", "A = 0.0;"));
    assert!(!same_code("A = 1.5;", "A = 1.25;"));
}

#[test]
fn switch_cases_are_compared_without_their_positions() {
    assert!(same_code(
        "switch (A) { case 1: B(); break; default: C(); }",
        "switch (A)\n{\n    case 1:\n        B();\n        break;\n    default:\n        C();\n}",
    ));
    assert!(!same_code(
        "switch (A) { case 1: B(); }",
        "switch (A) { case 2: B(); }"
    ));
}

#[test]
fn directives_are_compared_as_written() {
    assert!(same(
        "#exec OBJ LOAD FILE=A.utx  \n",
        "#exec OBJ LOAD FILE=A.utx\n"
    ));
    assert!(!same(
        "#exec OBJ LOAD FILE=A.utx\n",
        "#exec obj load file=A.utx\n"
    ));
}

#[test]
fn defaultproperties_are_compared_as_written() {
    assert!(same(
        "defaultproperties {\n    X=1\n}\n",
        "\n\ndefaultproperties\n{\n    X=1\n}\n"
    ));
    assert!(!same(
        "defaultproperties\n{\n    X=1\n}\n",
        "defaultproperties\n{\n    X=2\n}\n"
    ));
    assert!(!same(
        "defaultproperties\n{\n    X=1\n}\n",
        "defaultproperties\n{\n  X=1\n}\n"
    ));
    // UE2 reads an unclosed block to the end of the file, so the formatter
    // closes it.
    assert!(same(
        "defaultproperties\n{\n    X=1\n}",
        "defaultproperties\n{\n    X=1   \n\n"
    ));
}

#[test]
fn goto_labels_are_compared_with_their_quotes() {
    // In state code, `goto` takes a name: unquoted, the label is a variable.
    assert!(!same(
        "state S { Begin: goto 'A'; }",
        "state S { Begin: goto A; }"
    ));
    assert!(same(
        "state S { Begin: goto 'A'; }",
        "state S { Begin: GOTO 'a'; }"
    ));
}

#[test]
fn keyword_literals_are_literals_however_they_are_spaced() {
    assert!(same_code("A = None ;", "A = none;"));
    assert!(same_code("if (True )\n{}", "if (true) {}"));
}
