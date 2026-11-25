//! `usagi format`'s output: see `docs/FORMATTING.md`. Each rule's tests are
//! named after it.

use usagi_core::format::{FormatError, FormatOptions, Indent, format_text};

fn format(input: &str) -> String {
    format_text(input, &FormatOptions::default())
        .unwrap_or_else(|e| panic!("{input:?} should format: {e}"))
}

/// Formats `input`, and checks that the result is `expected` and that
/// formatting it again changes nothing.
#[track_caller]
fn assert_format(input: &str, expected: &str) {
    let output = format(input);
    assert_eq!(output, expected, "formatting {input:?}");
    assert_eq!(format(expected), expected, "formatting the output again");
}

/// As [`assert_format`], for a class body after `class A extends Object;`.
#[track_caller]
fn assert_body(input: &str, expected: &str) {
    let class = "class A extends Object;\n";
    assert_format(&format!("{class}{input}"), &format!("{class}\n{expected}"));
}

/// As [`assert_format`], for the statements of a function. `expected` is
/// indented one level, as in the function.
#[track_caller]
fn assert_code(input: &str, expected: &str) {
    let indented: Vec<String> = expected
        .lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("    {l}")
            }
        })
        .collect();
    assert_body(
        &format!("function F()\n{{\n{input}\n}}\n"),
        &format!("function F()\n{{\n{}\n}}\n", indented.join("\n")),
    );
}

#[track_caller]
fn assert_unformattable(input: &str, offset: usize) {
    match format_text(input, &FormatOptions::default()) {
        Err(FormatError::Unformattable { offset: o, .. }) => assert_eq!(o, offset),
        other => panic!("{input:?} shouldn't format: {other:?}"),
    }
}

#[test]
fn r1_indentation() {
    assert_code("\tif (A)\n\t{\n\t\tB();\n\t}", "if (A)\n{\n    B();\n}");
    let options = FormatOptions {
        indent: Indent::Tabs,
        ..Default::default()
    };
    assert_eq!(
        format_text("class A extends Object;\nfunction F() { G(); }", &options).unwrap(),
        "class A extends Object;\n\nfunction F()\n{\n\tG();\n}\n"
    );
    let options = FormatOptions {
        indent: Indent::Spaces(2),
        ..Default::default()
    };
    assert_eq!(
        format_text("class A extends Object;\nfunction F() { G(); }", &options).unwrap(),
        "class A extends Object;\n\nfunction F()\n{\n  G();\n}\n"
    );
}

#[test]
fn r2_trailing_whitespace() {
    assert_body("var int X; // a   \n", "var int X; // a\n");
    assert_body(
        "/* a   \n   b */\nvar int X;\n",
        "/* a\n   b */\nvar int X;\n",
    );
}

#[test]
fn r3_end_of_file() {
    assert_format("class A extends Object;", "class A extends Object;\n");
    assert_format(
        "class A extends Object;\n\n\n\n",
        "class A extends Object;\n",
    );
}

#[test]
fn line_endings_are_kept() {
    assert_eq!(
        format("class A extends Object;\r\nvar int X;\r\n"),
        "class A extends Object;\r\n\r\nvar int X;\r\n"
    );
}

#[test]
fn r4_consecutive_blank_lines() {
    assert_body(
        "var int X;\n\n\n\nvar int Y;\n",
        "var int X;\n\nvar int Y;\n",
    );
    assert_code("A();\n\n\n\nB();", "A();\n\nB();");
}

#[test]
fn r5_blank_lines_after_open_and_before_close() {
    assert_code("\n\nA();\n\n", "A();");
    assert_code(
        "if (A)\n{\n\n    // a\n    B();\n\n    // b\n\n}",
        "if (A)\n{\n    // a\n    B();\n\n    // b\n}",
    );
}

#[test]
fn r6_blank_lines_between_declarations() {
    assert_body(
        "function F();\nvar int X;\n\n\n// G\nfunction G() { G(); }",
        "function F();\n\nvar int X;\n\n// G\nfunction G()\n{\n    G();\n}\n",
    );
    // Functions without bodies are stacked, with no blank line between them
    // or the comments above them, even across a stray `;`, and in states.
    // Anything else around them is spaced.
    assert_body(
        "function F();\n\nfunction G();\n\n\n// H\n\nfunction H();",
        "function F();\nfunction G();\n// H\nfunction H();\n",
    );
    assert_body(
        "function F();;\nnative function G();\nfunction H() {}\nfunction I();\nstate S {\nfunction J();\nfunction K();\n}\nfunction L();",
        "function F();\nnative function G();\nfunction H();\nfunction I();\n\nstate S\n{\n    function J();\n    function K();\n}\n\nfunction L();\n",
    );
    assert_body(
        "// About F.\n\nfunction F();",
        "// About F.\n\nfunction F();\n",
    );
}

#[test]
fn r7_blank_line_after_locals() {
    assert_code(
        "local int I;\nlocal int J;\nI = J;",
        "local int I;\nlocal int J;\n\nI = J;",
    );
    assert_code("local int I;", "local int I;");
}

#[test]
fn r8_r9_blank_lines_kept() {
    assert_body(
        "var int X;\nvar int Y;\n\nvar int Z;\nconst C = 1;",
        "var int X;\nvar int Y;\n\nvar int Z;\nconst C = 1;\n",
    );
}

#[test]
fn r56_blank_line_above_control_flow() {
    assert_code(
        "A = B;\nif (A)\n{\n    C();\n    if (D) { E(); }\n}",
        "A = B;\n\nif (A)\n{\n    C();\n\n    if (D)\n    {\n        E();\n    }\n}",
    );
    assert_code(
        "A();\n// Loop.\nfor (;;) {}\nB();\nwhile (A) {}\nC();\nforeach F(X) {}",
        "A();\n\n// Loop.\nfor (;;)\n{\n}\nB();\n\nwhile (A)\n{\n}\nC();\n\nforeach F(X)\n{\n}",
    );
    assert_code(
        "A();\ndo {} until (A);\nB();\nswitch (A) {\ncase 1:\n    if (B) {}\n    C();\n    if (B) {}\n}",
        "A();\n\ndo\n{\n}\nuntil (A);\nB();\n\nswitch (A)\n{\n    case 1:\n        if (B)\n        {\n        }\n        C();\n\n        if (B)\n        {\n        }\n}",
    );
    assert_body(
        "state S\n{\nBegin:\n    if (A) {}\n}",
        "state S\n{\nBegin:\n    if (A)\n    {\n    }\n}\n",
    );
}

#[test]
fn r57_empty_end_of_line_comments_removed() {
    assert_body(
        "var() sound S; // \nvar int X; //\n//\n// A.\nvar int Y; // B",
        "var() sound S;\nvar int X;\n//\n// A.\nvar int Y; // B\n",
    );
    // A block copied as written keeps them.
    assert_body(
        "defaultproperties\n{\n    A=1 //\n}",
        "defaultproperties\n{\n    A=1 //\n}\n",
    );
}

#[test]
fn r58_space_after_line_comment() {
    assert_body(
        "//Note.\n//=====\n///Doc.\n//\nvar int X; //Inline\nvar int Y; //  Two\n/*Block*/\ndefaultproperties\n{\n    A=1 //B\n}",
        "// Note.\n//=====\n///Doc.\n//\nvar int X; // Inline\nvar int Y; //  Two\n\n/*Block*/\ndefaultproperties\n{\n    A=1 //B\n}\n",
    );
    // Lined-up end-of-line comments stay lined up.
    assert_code("A(); //One\n     //Two", "A(); // One\n     // Two");
}

#[test]
fn r10_allman_braces() {
    assert_code("if (A) {\n    B();\n}", "if (A)\n{\n    B();\n}");
    assert_body(
        "function F() {\n    F();\n}",
        "function F()\n{\n    F();\n}\n",
    );
}

#[test]
fn r15b_empty_function_bodies_are_removed() {
    assert_body("function F() {}", "function F();\n");
    assert_body(
        "function int F(int A)\n{\n    ;\n}",
        "function int F(int A);\n",
    );
    assert_body(
        "state S\n{\n    function F() {}\n}",
        "state S\n{\n    function F();\n}\n",
    );
    // Stacked like other functions without a body.
    assert_body(
        "function F();\nfunction G() {}",
        "function F();\nfunction G();\n",
    );
    // A body with a comment in it is kept.
    assert_body(
        "function F() { /* stub */ }",
        "function F()\n{ /* stub */\n}\n",
    );
    // So is one with only locals.
    assert_body(
        "function F() { local int A; }",
        "function F()\n{\n    local int A;\n}\n",
    );
}

#[test]
fn r11_r12_else() {
    assert_code(
        "if (A) { B(); } else if (C) { D(); } else { E(); }",
        "if (A)\n{\n    B();\n}\nelse if (C)\n{\n    D();\n}\nelse\n{\n    E();\n}",
    );
}

#[test]
fn r13_one_statement_per_line() {
    assert_code("A(); B();", "A();\nB();");
}

#[test]
fn r14_blocks_expanded() {
    assert_body(
        "function bool F() {return false;}",
        "function bool F()\n{\n    return false;\n}\n",
    );
}

#[test]
fn r14b_braces_added() {
    assert_code(
        "if (A) B();\nelse C();\nfor (I = 0; I < 1; ++I) D();\nwhile (A) E();\nforeach AllActors(Class'Actor', X) F();\ndo G(); until (A);",
        "if (A)\n{\n    B();\n}\nelse\n{\n    C();\n}\n\nfor (I = 0; I < 1; ++I)\n{\n    D();\n}\n\nwhile (A)\n{\n    E();\n}\n\nforeach AllActors(Class'Actor', X)\n{\n    F();\n}\n\ndo\n{\n    G();\n}\nuntil (A);",
    );
    // An `if` with no body.
    assert_code("if (A)\n// B();\n", "if (A)\n{\n}\n// B();");
}

#[test]
fn r15_empty_blocks() {
    assert_code("if (A) {}", "if (A)\n{\n}");
}

#[test]
fn r16_do_until() {
    assert_code("do { A(); } until (B)", "do\n{\n    A();\n}\nuntil (B);");
    assert_code("do { A(); }", "do\n{\n    A();\n}");
}

#[test]
fn r17_semicolons() {
    assert_body(
        "struct S { var int X; };\nenum E { A, B, };",
        "struct S\n{\n    var int X;\n};\n\nenum E\n{\n    A,\n    B,\n};\n",
    );
    assert_body(
        "var int X;;\nfunction F() { ; A();; };",
        "var int X;\n\nfunction F()\n{\n    A();\n}\n",
    );
}

#[test]
fn r18_to_r28_spacing() {
    assert_code(
        "if(A){return(B);}\nwhile( A ){}\nswitch(A){}\nX=F( A ,B )[ 1 ];\nX+=-Y*!Z++;\nfor(I=0;I<1;I++){}\nX = A.default.B + default.C + A.static.D() + static.E();\nX = super(Actor).F() + Super.G() + Self.H + global.I();",
        "if (A)\n{\n    return (B);\n}\n\nwhile (A)\n{\n}\n\nswitch (A)\n{\n}\nX = F(A, B)[1];\nX += -Y * !Z++;\n\nfor (I = 0; I < 1; I++)\n{\n}\nX = A.default.B + default.C + A.static.D() + static.E();\nX = super(Actor).F() + super.G() + self.H + global.I();",
    );
    // `- -1` mustn't become `--1`.
    assert_code("X = - -1;\nX = -(-Y);", "X = - -1;\nX = -(-Y);");
    assert_code(
        "for(;;){}\nfor(;A;){}",
        "for (;;)\n{\n}\n\nfor (; A;)\n{\n}",
    );
    assert_code("F(A,,B);\nF(,);\nF();", "F(A,, B);\nF(,);\nF();");
    assert_code("X = A Dot B;", "X = A Dot B;");
}

#[test]
fn r26_generic_types() {
    assert_body(
        "var Array< Class<Actor> > A;\nvar array<int> B;",
        "var array<class<Actor> > A;\nvar array<int> B;\n",
    );
}

#[test]
fn r27_var_category() {
    assert_body(
        "var ( Display ) int X;\nvar() int Y;",
        "var(Display) int X;\nvar() int Y;\n",
    );
}

#[test]
fn r29_end_of_line_comments() {
    // One space before each, however they were aligned.
    assert_code(
        "A();       // one\nB();\t/* two */   /* three */\nif (A)// four\n{ // five\n}  // six",
        "A(); // one\nB(); /* two */ /* three */\n\nif (A) // four\n{ // five\n} // six",
    );
    assert_body(
        "struct S\n{\n    var() int    X;     // one\n    var() string Name;  // two\n};",
        "struct S\n{\n    var() int X; // one\n    var() string Name; // two\n};\n",
    );
    // A comment after code that's moved to a line of its own stays with it.
    assert_code("if (A) B(); // one", "if (A)\n{\n    B(); // one\n}");
}

#[test]
fn r29b_continued_end_of_line_comments() {
    // Comments lined up below an end-of-line comment stay lined up with it.
    assert_body(
        "var int A;   // one\n             // two\n             // three\nvar int B; // four\n// five\n",
        "var int A; // one\n           // two\n           // three\nvar int B; // four\n// five\n",
    );
    // A tab reaches the next multiple of 4 columns.
    assert_body(
        "var int AB;\t// one\n\t\t\t// two\n",
        "var int AB; // one\n            // two\n",
    );
    // With tabs, the indentation stays tabs.
    let options = FormatOptions {
        indent: Indent::Tabs,
        ..FormatOptions::default()
    };
    assert_eq!(
        format_text(
            "class A extends Object;\nfunction F()\n{\n    X = 1;  // one\n            // two\n}",
            &options
        )
        .unwrap(),
        "class A extends Object;\n\nfunction F()\n{\n\tX = 1; // one\n\t       // two\n}\n"
    );
    // Not in another column, after a blank line, or after a block comment.
    assert_body(
        "var int A; // one\n            // two\nvar int B; // three\n\n           // four\nvar int C; /* five */\n           // six\n",
        "var int A; // one\n// two\nvar int B; // three\n\n// four\nvar int C; /* five */\n// six\n",
    );
    // When something is printed between them, they're separated.
    assert_code(
        "if (A) B();  // one\n             // two\nC();",
        "if (A)\n{\n    B(); // one\n}\n// two\nC();",
    );
    // The blank line before a function goes after the continuation.
    assert_body(
        "var int A; // one\n           // two\nfunction F() { F(); }",
        "var int A; // one\n           // two\n\nfunction F()\n{\n    F();\n}\n",
    );
}

#[test]
fn comments_are_reindented() {
    // The lines inside a block comment are kept as written.
    assert_body(
        "function F() {\n        // one\n  /* two\n     three */\nA();\n}",
        "function F()\n{\n    // one\n    /* two\n     three */\n    A();\n}\n",
    );
    assert_format(
        "// Header\n\nclass A extends Object;\n// Trailer",
        "// Header\n\nclass A extends Object;\n\n// Trailer\n",
    );
}

#[test]
fn comments_that_cant_be_placed() {
    let class = "class A extends Object;\n";
    for code in [
        // Inside a member access, between modifiers, and after `(`.
        "function F() { X = A./* B */C; }",
        "var config /* X */ int Y;",
        "function F() { if (// A\nB) {} }",
        "function F() { X = (A + B // C\n); }",
    ] {
        let input = format!("{class}{code}");
        let offset = input.find("/").unwrap();
        assert_unformattable(&input, offset);
    }
    // The offset is into the original text, with its CRLFs.
    let input = "class A extends Object;\r\n\r\nvar config /* X */ int Y;\r\n";
    assert_unformattable(input, input.find("/*").unwrap());
}

#[test]
fn files_that_dont_parse() {
    assert!(matches!(
        format_text("class A extends", &FormatOptions::default()),
        Err(FormatError::Parse(_))
    ));
}

#[test]
fn r30_r31_class_declaration() {
    assert_format(
        "Class A Extends B Within C;",
        "class A extends B within C;\n",
    );
    assert_format(
        "class A extends B notplaceable Config(User) abstract HideCategories(X,Y);",
        "class A extends B\n    abstract\n    config(User)\n    hidecategories(X, Y)\n    notplaceable;\n",
    );
    assert_format(
        "class A extends B\n    // abstract\n    ;",
        "class A extends B\n    // abstract\n    ;\n",
    );
    assert_format(
        "class A extends B // abstract\n;",
        "class A extends B // abstract\n    ;\n",
    );
    assert_format(
        "class A extends B\n    abstract // why\n    // native\n    ;",
        "class A extends B\n    abstract // why\n    // native\n    ;\n",
    );
}

#[test]
fn r32_modifier_order() {
    assert_body(
        "var transient private config int X;\nfinal simulated private native(12) function F(out optional int X);",
        "var private config transient int X;\n\nprivate final native(12) simulated function F(optional out int X);\n",
    );
}

#[test]
fn r33_function_headers() {
    assert_body(
        "static final operator(16) float * ( float A, float B );\nevent bool G();\nfunction H(int A[2], class<Actor> B);",
        "final static operator(16) float *(float A, float B);\nevent bool G();\nfunction H(int A[2], class<Actor> B);\n",
    );
}

#[test]
fn r33_parameters_one_per_line() {
    let options = FormatOptions {
        line_width: 40,
        ..Default::default()
    };
    let check = |input: &str, expected: &str| {
        let class = "class A extends Object;\n";
        let (input, expected) = (format!("{class}{input}"), format!("{class}\n{expected}"));
        assert_eq!(
            format_text(&input, &options).unwrap(),
            expected,
            "formatting {input:?}"
        );
        assert_eq!(format_text(&expected, &options).unwrap(), expected);
    };
    // Parameters that fit stay on the header's line, block comments too.
    check(
        "function F(int A,\n    int B /* b */) { F(); }",
        "function F(int A, int B /* b */)\n{\n    F();\n}\n",
    );
    // Otherwise each goes on its own line, and so does the `)`.
    check(
        "static function string GetString(optional int Switch, optional Object O);",
        "static function string GetString(\n    optional int Switch,\n    optional Object O\n    );\n",
    );
    // An end-of-line comment puts them on their own lines, and comments stay
    // at the ends of lines or on their own lines, as written.
    check(
        "function F( // one\nint A, // two\n// three\nint B, /* four */ int C // five\n) {}",
        "function F( // one\n    int A, // two\n    // three\n    int B, /* four */\n    int C // five\n    );\n",
    );
    // A comment inside a parameter can't be placed.
    let input = "class A extends Object;\nfunction F(int A, int /* x */ B, // y\n int C);";
    assert_unformattable(input, input.find("/*").unwrap());
}

#[test]
fn r34_r35_declarations() {
    assert_code(
        "local   int   I,J ;\nlocal string S[ 4 ];\nlocal array<int> A;",
        "local int I, J;\nlocal string S[4];\nlocal array<int> A;",
    );
    assert_body("var byte B[MAX];", "var byte B[MAX];\n");
}

#[test]
fn r36_constants() {
    assert_body("const   X=1;", "const X = 1;\n");
    assert_code("const X=1;\nA();", "const X = 1;\nA();");
}

#[test]
fn r37_structs_and_enums() {
    assert_body(
        "struct native export S extends T { var() const int X; var float Y; cppstruct { int Z; } };",
        "struct export native S extends T\n{\n    var() const int X;\n    var float Y;\n    cppstruct\n    { int Z; }\n};\n",
    );
    assert_body(
        "enum E { A, // one\n B // two\n};",
        "enum E\n{\n    A, // one\n    B, // two\n};\n",
    );
    assert_body(
        "var enum E { A, B } X, Y;\nvar() struct S { var int Z; } V;",
        "var enum E\n{\n    A,\n    B,\n} X, Y;\nvar() struct S\n{\n    var int Z;\n} V;\n",
    );
    assert_body(
        "struct S { var struct T { var int X; } Y; };",
        "struct S\n{\n    var struct T\n    {\n        var int X;\n    } Y;\n};\n",
    );
}

#[test]
fn types_declared_where_they_cant_be_printed() {
    let class = "class A extends Object;\n";
    assert_unformattable(
        &format!("{class}var array<struct S {{ var int X; }}> Y;"),
        class.len(),
    );
    assert_unformattable(
        &format!("{class}function struct S {{ var int X; }} F();"),
        class.len(),
    );
}

#[test]
fn r38_replication() {
    assert_body(
        "replication {\n    reliable if (Role == ROLE_Authority)\n        A, B;\n  UNRELIABLE IF (bNetDirty) C;\n}",
        "replication\n{\n    reliable if (Role == ROLE_Authority)\n        A, B;\n    unreliable if (bNetDirty)\n        C;\n}\n",
    );
}

#[test]
fn r39_switch() {
    assert_code(
        "switch (A) {\ncase 0: // zero\nB(); break;\n\ndefault:\nC();\n// end\n}",
        "switch (A)\n{\n    case 0: // zero\n        B();\n        break;\n\n    default:\n        C();\n    // end\n}",
    );
}

#[test]
fn r41_labels() {
    assert_body(
        "auto simulated state() S extends T {\n    ignores A, B;\n    const C = 1;\n    function F();\n    Begin:\n    G();\n    goto 'Begin';\nEnd:\n    goto End;\n}",
        "auto simulated state() S extends T\n{\n    ignores A, B;\n    const C = 1;\n\n    function F();\n\nBegin:\n    G();\n    goto 'Begin';\nEnd:\n    goto End;\n}\n",
    );
    // Labels in functions too.
    assert_body(
        "function F() {\n    Loop:\n    A();\n}",
        "function F()\n{\nLoop:\n    A();\n}\n",
    );
}

#[test]
fn r42_jumps() {
    assert_code(
        "while (A) { break ; continue ; }\nreturn ;\nreturn\"\";",
        "while (A)\n{\n    break;\n    continue;\n}\nreturn;\nreturn \"\";",
    );
}

#[test]
fn r43_to_r48_keyword_case() {
    assert_code(
        "IF (X == NONE || True || FALSE) RETURN SELF;\nX = CLASS'Actor' == class'Actor' && enum'E'.A && Texture'T';\nX = New(Self) CLASS<Actor>(Y);",
        "if (X == none || true || false)\n{\n    return self;\n}\nX = Class'Actor' == Class'Actor' && Enum'E'.A && Texture'T';\nX = new(self) class<Actor>(Y);",
    );
    assert_body(
        "VAR INT X;\nvar Float Y;\nvar BYTE Z;\nvar String S;\nvar Name N;\nvar Bool B;\nvar Vector V;",
        "var int X;\nvar float Y;\nvar byte Z;\nvar string S;\nvar name N;\nvar bool B;\nvar Vector V;\n",
    );
    assert_code("X = New Class'A';", "X = new Class'A';");
}

#[test]
fn r49_r50_literals() {
    assert_code(
        "X = 1. + .5 - -.25 + 1.f + 1.0f + 1e5 + 0X1f + -0x2 + +3;",
        "X = 1.0 + 0.5 - -0.25 + 1.0f + 1.0f + 1e5 + 0x1f + -0x2 + +3;",
    );
    // A malformed float is repaired: it's read as `0.0`.
    assert_code("X = 0.0.5;", "X = 0.0;");
}

#[test]
fn r51_r52_strings_names_vectors_and_rotators() {
    assert_code(
        "X = \"A  b\" $ 'Some Name' $ VECT( 1,2 ,3 ) $ Rot(1, 2, 3);",
        "X = \"A  b\" $ 'Some Name' $ vect(1, 2, 3) $ rot(1, 2, 3);",
    );
}

#[test]
fn r53_directives() {
    assert_format(
        "#EXEC OBJ  LOAD FILE=A.utx  \nclass A extends Object;\n  #exec obj load file=B.utx // B\nfunction F() {\n#exec C\n}",
        "#EXEC OBJ  LOAD FILE=A.utx\nclass A extends Object;\n\n#exec obj load file=B.utx // B\n\nfunction F()\n{\n#exec C\n}\n",
    );
}

#[test]
fn r53_comments_in_directives() {
    // A comment inside a directive is part of it.
    assert_body(
        "#exec OBJ /* x */ LOAD FILE=A.utx // A\n",
        "#exec OBJ /* x */ LOAD FILE=A.utx // A\n",
    );
}

#[test]
fn r54_defaultproperties() {
    assert_body(
        "defaultproperties {\n    X=1   \n  // a } comment\n    Y=\"}\"\n}",
        "defaultproperties\n{\n    X=1\n  // a } comment\n    Y=\"}\"\n}\n",
    );
    assert_body(
        "defaultproperties // why\n// before\n{X=1}",
        "defaultproperties // why\n// before\n{X=1}\n",
    );
    // An unclosed block is closed.
    assert_body(
        "defaultproperties\n{\n    X=1\n\n",
        "defaultproperties\n{\n    X=1\n}\n",
    );
}

#[test]
fn r55_cpptext() {
    assert_body(
        "cpptext // C++\n{\n    void F();   \n}",
        "cpptext // C++\n{\n    void F();   \n}\n",
    );
}

#[test]
fn only_the_first_problem_is_reported() {
    let class = "class A extends Object;\n";
    let input = format!("{class}var config /* X */ int Y;\nvar config /* Z */ int W;\n");
    assert_unformattable(&input, input.find("/*").unwrap());
}

#[test]
fn comments_after_stray_semicolons_and_copied_blocks() {
    // A stray `;` is removed, and the comment after it goes on its own line.
    assert_body("var int X;; // X", "var int X;\n// X\n");
    assert_body(
        "defaultproperties { X=1 } // X",
        "defaultproperties\n{ X=1 } // X\n",
    );
}

#[test]
fn classes_without_parents_and_states_without_anything() {
    assert_format(
        "class Object native;\nstate S {}",
        "class Object\n    native;\n\nstate S\n{\n}\n",
    );
}

#[test]
fn constants_in_blocks() {
    assert_code("if (A) { const X = 1; }", "if (A)\n{\n    const X = 1;\n}");
}

/// As [`assert_code`], with lines at most `width` columns.
#[track_caller]
fn assert_code_width(width: usize, input: &str, expected: &str) {
    let options = FormatOptions {
        line_width: width,
        ..Default::default()
    };
    let indented: Vec<String> = expected
        .lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("    {l}")
            }
        })
        .collect();
    let input = format!("class A extends Object;\nfunction F()\n{{\n{input}\n}}\n");
    let expected = format!(
        "class A extends Object;\n\nfunction F()\n{{\n{}\n}}\n",
        indented.join("\n")
    );
    let output = format_text(&input, &options).unwrap_or_else(|e| panic!("{input:?}: {e}"));
    assert_eq!(output, expected, "formatting {input:?}");
    assert_eq!(
        format_text(&expected, &options).unwrap(),
        expected,
        "formatting again"
    );
}

#[test]
fn l_lines_that_fit_are_joined() {
    assert_code_width(40, "X = A +\n    B;\nF(A,\n  B);", "X = A + B;\nF(A, B);");
}

#[test]
fn l1_unbreakable_lines_are_left_long() {
    assert_code_width(
        20,
        "X = \"a very long string literal\";\nY = Aaaaaa.Bbbbbbb.Cccccccc.Ddddddd;",
        "X = \"a very long string literal\";\nY = Aaaaaa.Bbbbbbb.Cccccccc.Ddddddd;",
    );
}

#[test]
fn l2_arguments_one_per_line() {
    assert_code_width(
        30,
        "Function(Argument1, Argument2, Argument3, Argument4);",
        "Function(\n    Argument1,\n    Argument2,\n    Argument3,\n    Argument4\n    );",
    );
    // Each argument is laid out on its own line: a call that fits there
    // stays on it, and one that doesn't breaks a level further in.
    assert_code(
        "return Repl(Repl(Repl(default.KickedFromSquadMessage,\n    \"{0}\", RelatedPRI_1.PlayerName),\n    \"{1}\", RelatedPRI_2.PlayerName),\n    \"{2}\", static.GetSquadName(RelatedPRI_2.Team.TeamIndex, ExtraValue, DHSquadReplicationInfo(OptionalObject)));",
        "return Repl(\n    Repl(Repl(default.KickedFromSquadMessage, \"{0}\", RelatedPRI_1.PlayerName), \"{1}\", RelatedPRI_2.PlayerName),\n    \"{2}\",\n    static.GetSquadName(RelatedPRI_2.Team.TeamIndex, ExtraValue, DHSquadReplicationInfo(OptionalObject))\n    );",
    );
    assert_code_width(
        30,
        "F(A, G(Argument1, Argument2, Argument3));",
        "F(\n    A,\n    G(\n        Argument1,\n        Argument2,\n        Argument3\n        )\n    );",
    );
    // Empty slots, in a call that fits, and one that doesn't, where an empty
    // slot's comma stays on the line before.
    assert_code("F(A,,B /* b */);", "F(A,, B /* b */);");
    assert_code_width(
        20,
        "Function(Aaaaaa,, Bbbbbb, Cccccc);\nFunction(, Bbbbbbbbb, Cccccc);\nF(Aaaaaaaaaaa, Bbbbbbbbbb,);",
        "Function(\n    Aaaaaa,,\n    Bbbbbb,\n    Cccccc\n    );\nFunction(,\n    Bbbbbbbbb,\n    Cccccc\n    );\nF(\n    Aaaaaaaaaaa,\n    Bbbbbbbbbb,\n    );",
    );
    assert_code_width(
        20,
        "X = global.Function(Aaaaaa, Bbbbbb);",
        "X = global.Function(\n    Aaaaaa,\n    Bbbbbb\n    );",
    );
}

#[test]
fn l2_declarations_are_filled() {
    let options = FormatOptions {
        line_width: 40,
        ..Default::default()
    };
    let input = "class A extends Object;\nfunction F(int Argument1, int Argument2, int Argument3);\nfunction G(int Argument1, int Argument2, int Argument3) {\nlocal int Aaaaaaaaaa, Bbbbbbbbbb, Cccccccccc, Ddddddddd;\n}\nvar int Aaaaaaaaaa, Bbbbbbbbbb, Cccccccccc, Ddddddddd;\nreplication {\nreliable if (bNetDirty)\nAaaaaaaaaa, Bbbbbbbbbb, Cccccccccc, Dddddddddd;\n}\nstate S {\nignores Aaaaaaaaaa, Bbbbbbbbbb, Cccccccccc, Dddddddddd;\n}";
    let expected = "class A extends Object;\n\nfunction F(\n    int Argument1,\n    int Argument2,\n    int Argument3\n    );\n\nfunction G(\n    int Argument1,\n    int Argument2,\n    int Argument3\n    )\n{\n    local int Aaaaaaaaaa, Bbbbbbbbbb,\n        Cccccccccc, Ddddddddd;\n}\n\nvar int Aaaaaaaaaa, Bbbbbbbbbb,\n    Cccccccccc, Ddddddddd;\n\nreplication\n{\n    reliable if (bNetDirty)\n        Aaaaaaaaaa, Bbbbbbbbbb,\n        Cccccccccc, Dddddddddd;\n}\n\nstate S\n{\n    ignores Aaaaaaaaaa, Bbbbbbbbbb,\n        Cccccccccc, Dddddddddd;\n}\n";
    assert_eq!(format_text(input, &options).unwrap(), expected);
    assert_eq!(format_text(expected, &options).unwrap(), expected);
}

#[test]
fn l3_operators_one_per_line() {
    assert_code_width(
        30,
        "if (Aaaaaaaa && Bbbbbbbbbb && Cccccccccc) {}",
        "if (Aaaaaaaa\n    && Bbbbbbbbbb\n    && Cccccccccc)\n{\n}",
    );
    assert_code_width(
        30,
        "return Aaaaaaaaaa == Bbbbbbbbbbb + Cccccccccc;",
        "return Aaaaaaaaaa\n    == Bbbbbbbbbbb\n        + Cccccccccc;",
    );
    // Assignments don't break at their operator.
    assert_code_width(
        20,
        "Xxxxxx = Aaaaaa || Bbbbbbb;",
        "Xxxxxx = Aaaaaa\n    || Bbbbbbb;",
    );
}

#[test]
fn l4_loosest_operators_first() {
    assert_code_width(
        40,
        "while (Aaaaaaa && Bbbbbbb || Ccccccc && Ddddddd || (Eeeeeee && Fffffff && Ggggggg && Hhhhhhh)) {}",
        "while (Aaaaaaa && Bbbbbbb\n    || Ccccccc && Ddddddd\n    || (Eeeeeee\n        && Fffffff\n        && Ggggggg\n        && Hhhhhhh))\n{\n}",
    );
}

#[test]
fn l5_concatenation_is_filled() {
    assert_code_width(
        30,
        "Log(\"Aaaaaa\" @ Bbbbbbb @ \"Cccccc\" $ Dddddd $ Eeeeee);",
        "Log(\n    \"Aaaaaa\" @ Bbbbbbb\n        @ \"Cccccc\"\n        $ Dddddd $ Eeeeee\n    );",
    );
}

#[test]
fn chains_break_before_each_call() {
    assert_code_width(
        50,
        "return (new Class'JSONObject').PutString(\"id\", ID).Put(\"x\", (new Class'JSONObject').PutInteger(\"team\", Team).Put(\"pawn\", Pawn));",
        "return (new Class'JSONObject')\n    .PutString(\"id\", ID)\n    .Put(\n        \"x\",\n        (new Class'JSONObject')\n            .PutInteger(\"team\", Team)\n            .Put(\"pawn\", Pawn)\n        );",
    );
    // A chain that fits is joined, and one call isn't a chain.
    assert_code_width(40, "X = A.B()\n    .C();", "X = A.B().C();");
    assert_code_width(
        20,
        "Level.Game.Broadcast(Self, Msg);",
        "Level.Game.Broadcast(\n    self,\n    Msg\n    );",
    );
}

#[test]
fn breaking_inside_other_expressions() {
    assert_code_width(
        20,
        "X = !Aaaaaa(Bbbbbb, Cccccc);\nX = Aaaaaa(Bbbbbb, Cccccc)++;\nX = Aaaaaa(Bbbbbb, Cccccc)[0];\nX = Aaaaaa(Bbbbbb, Cccccc).Dd;\nX = Aaaaaa(Bbbbbb, Cccccc).default.Dd;\nX = Aaaaaa(Bbbbbb, Cccccc).static.Dd();\nX = class<A>(Aaaaaa(Bbbbbb, Cccccc));\nX = -Aaaaaa(Bbbbbb, Cccccc);\nX = Dddddddddddddddddd[Aaaaaa(Bbbbbb, Cccccc)];",
        "X = !Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    );\nX = Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    )++;\nX = Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    )[0];\nX = Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    ).Dd;\nX = Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    ).default.Dd;\nX = Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    ).static.Dd();\nX = class<A>(Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    ));\nX = -Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    );\nX = Dddddddddddddddddd[Aaaaaa(\n    Bbbbbb,\n    Cccccc\n    )];",
    );
}

#[test]
fn comments_inside_lines() {
    // Block comments stay inline, and the line breaks only if it's too long.
    assert_code(
        "X = A /* B */ + C;\nF(/* none */);\nF(/* first */ A, B /* last */);\nif (/* A */ B /* C */) {}\nX = (A /* B */);\nY = A /* B */;\nreturn A /* B */;",
        "X = A /* B */ + C;\nF(/* none */);\nF(/* first */ A, B /* last */);\n\nif (/* A */ B /* C */)\n{\n}\nX = (A /* B */);\nY = A /* B */;\nreturn A /* B */;",
    );
    assert_code(
        "local string /* A, */ B, C;\nX = A /* B */ = C;",
        "local string /* A, */ B, C;\n\nX = A = /* B */ C;",
    );
    // End-of-line comments break the line after them, after one space.
    assert_code(
        "if (A &&   // one\n    B)\n{}\nF(A,  // two\n  B);\nX = A.B()  // three\n    .C();",
        "if (A // one\n    && B)\n{\n}\nF(\n    A, // two\n    B\n    );\nX = A\n    .B() // three\n    .C();",
    );
    // One before the `)` of a header goes after it.
    assert_code(
        "while (A &&\n    B  // three\n) {}",
        "while (A && B) // three\n{\n}",
    );
    assert_code("do {} until (A // four\n)", "do\n{\n}\nuntil (A); // four");
}

#[test]
fn comments_between_enum_values_and_commas() {
    assert_body(
        "enum E { A /* one */, B };",
        "enum E\n{\n    A /* one */,\n    B,\n};\n",
    );
}

#[test]
fn comments_ending_in_whitespace() {
    // A `//` comment includes the spaces at the end of its line.
    assert_body(
        "function F()\n{\n\t// one   \n\tA();\n}",
        "function F()\n{\n    // one\n    A();\n}\n",
    );
}

#[test]
fn more_comments_inside_lines() {
    // Inline in a chain that fits, and between arguments, on a line of its
    // own or at the end of one, as written.
    assert_code(
        "X = A.B() /* one */ .C();\nF(A,\n// two\nB);\nF(A,// three\nB);\nF( // four\n A /* five */, B /* six */ // seven\n);",
        "X = A.B() /* one */.C();\nF(\n    A,\n    // two\n    B\n    );\nF(\n    A, // three\n    B\n    );\nF( // four\n    A, /* five */\n    B /* six */ // seven\n    );",
    );
    // A comment in a line that's printed as it is.
    let class = "class A extends Object;\n";
    let input = format!("{class}const X = /* one */ 1;");
    assert_unformattable(&input, input.find("/*").unwrap());
}
