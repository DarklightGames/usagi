// Tests for the lints that look at other classes.
use std::path::PathBuf;
use usagi_core::project_lint;
use usagi_core::{ClassId, Lint, Origin, Project, parse_program};

fn project(sources: &[&str]) -> Project {
    let files = sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let origin = Origin {
                root: 0,
                package: None,
                path: PathBuf::from(format!("{:03}.uc", i)),
            };
            (origin, parse_program(source).unwrap().symbols)
        })
        .collect();
    Project::build(files)
}

const OBJECT: &str = "class Object;\nvar int Flags;\nconst LIMIT = 3;\nenum EColor { C_Red };\n";
const ACTOR: &str = "class Actor extends Object;\nvar float Health;\nvar int Slots[4];\n";

/// The `(line, name)` of each undeclared property in the last source's
/// `defaultproperties`, with the others in the project.
fn undeclared(sources: &[&str]) -> Vec<(usize, String)> {
    let p = project(sources);
    let last = sources.last().unwrap();
    let class = p.classes().last().unwrap();
    project_lint::lint(&p, class, last)
        .into_iter()
        .map(|e| {
            assert_eq!(e.lint, Some(Lint::UndeclaredDefaultProperty));
            assert!(e.fix.is_none());
            (
                e.span.line.unwrap(),
                last[e.span.start..e.span.end].to_string(),
            )
        })
        .collect()
}

#[test]
fn a_property_nobody_declares() {
    let pawn = "class Pawn extends Actor;\n\
        var int Own;\n\
        defaultproperties\n\
        {\n\
        \x20   Own=1\n\
        \x20   health=2\n\
        \x20   Flags=3\n\
        \x20   Slots(1)=4\n\
        \x20   Typo=5\n\
        \x20   Typo2(0)=6\n\
        }\n";
    assert_eq!(
        undeclared(&[OBJECT, ACTOR, pawn]),
        [(9, "Typo".to_string()), (10, "Typo2".to_string())]
    );
}

#[test]
fn the_message_names_the_property_and_the_class() {
    let p = project(&[
        OBJECT,
        "class Pawn extends Object;\ndefaultproperties\n{\n    Nope=1\n}\n",
    ]);
    let class = p.class("pawn").unwrap();
    let errors = project_lint::check(&p, class);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "`Nope` isn't declared in `Pawn` or any class it extends"
    );
    assert_eq!(
        errors[0].severity,
        Lint::UndeclaredDefaultProperty.severity()
    );
    // `check` leaves the line, as the parser does until the lines are known.
    assert_eq!(errors[0].span.line, None);
}

#[test]
fn only_what_is_known_not_to_exist_is_reported() {
    // Actor's parent isn't in the project, so what Pawn lacks may be Object's.
    let pawn = "class Pawn extends Actor;\ndefaultproperties\n{\n    Unknown=1\n}\n";
    assert_eq!(undeclared(&[ACTOR, pawn]), []);
    // A class that extends nothing in the project at all.
    let alone = "class Alone extends Gone;\ndefaultproperties\n{\n    Unknown=1\n}\n";
    assert_eq!(undeclared(&[alone]), []);
    // And the same class is reported once its parents are.
    assert_eq!(undeclared(&[OBJECT, ACTOR, pawn]).len(), 1);
}

#[test]
fn names_that_are_not_variables_are_not_this_lints_business() {
    // A const or an enum value isn't a property, but it is declared.
    let pawn = "class Pawn extends Object;\ndefaultproperties\n{\n    LIMIT=1\n    C_Red=2\n}\n";
    assert_eq!(undeclared(&[OBJECT, pawn]), []);
}

#[test]
fn delegates_are_assigned_there_too() {
    let parent = "class Parent extends Object;\ndelegate bool OnKey(int Key);\n";
    let pawn = "class Pawn extends Parent;\n\
        defaultproperties\n\
        {\n\
        \x20   OnKey=Foo.Handler\n\
        \x20   OnNothing=Foo.Handler\n\
        }\n";
    assert_eq!(
        undeclared(&[OBJECT, parent, pawn]),
        [(5, "OnNothing".to_string())]
    );
}

#[test]
fn objects_and_other_statements_are_skipped() {
    let pawn = "class Pawn extends Object;\n\
        defaultproperties\n\
        {\n\
        \x20   Begin Object Class=Foo Name=Sub\n\
        \x20       NotAProperty=1\n\
        \x20   End Object\n\
        \x20   Nope=1\n\
        }\n";
    assert_eq!(undeclared(&[OBJECT, pawn]), [(7, "Nope".to_string())]);
}

#[test]
fn suppression_comments() {
    let pawn = "class Pawn extends Object;\n\
        // usagi: ignore-file[correctness]\n\
        defaultproperties\n\
        {\n\
        \x20   A=1 // usagi: ignore[C017]\n\
        \x20   // usagi: ignore[undeclared-default-property] a reason\n\
        \x20   B=1\n\
        \x20   // usagi: ignore[ALL]\n\
        \x20   C=1\n\
        \x20   // usagi: ignore[S]\n\
        \x20   D=1\n\
        }\n";
    // Naming the lint suppresses it; a category or `ALL` doesn't, as they
    // don't select it either.
    assert_eq!(
        undeclared(&[OBJECT, pawn]),
        [(9, "C".to_string()), (11, "D".to_string())]
    );
    let file = "class Pawn extends Object;\n// usagi: ignore-file[C017]\ndefaultproperties\n{\n    A=1\n}\n";
    assert_eq!(undeclared(&[OBJECT, file]), []);
}

#[test]
fn parsing_does_not_call_a_suppression_of_a_project_lint_unused() {
    let source = "class Pawn extends Object;\n\
        defaultproperties\n\
        {\n\
        \x20   // usagi: ignore[C017]\n\
        \x20   A=1\n\
        \x20   // usagi: ignore[S001]\n\
        \x20   B=1\n\
        }\n";
    let errors = parse_program(source).unwrap().errors;
    let lints: Vec<_> = errors.iter().filter_map(|e| e.lint).collect();
    // Only the ordinary lint's comment is judged, here.
    assert_eq!(lints, [Lint::UnusedSuppression]);
    assert!(errors[0].message.contains("S001"));
}

#[test]
fn a_class_with_nothing_to_report_costs_nothing() {
    let p = project(&[OBJECT]);
    let class: ClassId = p.class("Object").unwrap();
    assert!(project_lint::lint(&p, class, OBJECT).is_empty());
}

// S018 non-exhaustive-switch, and the typing it rests on.

/// The `(line, message)` of each non-exhaustive `switch` in the last source.
fn switches(sources: &[&str]) -> Vec<(usize, String)> {
    let p = project(sources);
    let last = sources.last().unwrap();
    let class = p.classes().last().unwrap();
    project_lint::lint(&p, class, last)
        .into_iter()
        .filter(|e| e.lint == Some(Lint::NonExhaustiveSwitch))
        .map(|e| (e.span.line.unwrap(), e.message))
        .collect()
}

const COLORS: &str = "class Object;\n\
    enum EColor { C_Red, C_Green, C_Blue };\n\
    struct Pair { var EColor First; var EColor Second[2]; };\n";

/// A class `Holder` with state to switch on.
const HOLDER: &str = "class Holder extends Object;\n\
    var EColor Color;\n\
    var EColor Palette[3];\n\
    var array<EColor> Many;\n\
    var Pair Both;\n\
    var int Count;\n\
    var byte Raw;\n\
    var Mystery Odd;\n\
    function Nada() {}\n\
    function EColor Pick() { return C_Red; }\n\
    static function EColor Make() { return C_Red; }\n";

/// A class `Foo` that extends `Holder`, with `body` as the body of `F`.
fn foo(body: &str) -> String {
    format!(
        "class Foo extends Holder;\nvar Holder Other;\nfunction F(EColor Arg)\n{{\n    local EColor Local;\n{}\n}}\n",
        body
    )
}

#[test]
fn a_switch_that_misses_values() {
    let source = foo(
        "    switch (Color)\n    {\n        case C_Red:\n            break;\n        case C_Blue:\n            break;\n    }",
    );
    assert_eq!(
        switches(&[COLORS, HOLDER, &source]),
        [(
            6,
            "`switch` on `EColor` has no `default` and doesn't handle `C_Green`".to_string()
        )]
    );
}

#[test]
fn what_makes_a_switch_exhaustive() {
    let all = "    switch (Color)\n    {\n        case C_Red:\n        case C_Green:\n        case C_Blue:\n            break;\n    }";
    let qualified = "    switch (Color)\n    {\n        case EColor.C_Red:\n        case EColor.C_Green:\n        case c_BLUE:\n            break;\n    }";
    let with_default = "    switch (Color)\n    {\n        case C_Red:\n            break;\n        default:\n            break;\n    }";
    let only_default = "    switch (Color)\n    {\n        default:\n    }";
    for body in [all, qualified, with_default, only_default] {
        assert_eq!(switches(&[COLORS, HOLDER, &foo(body)]), [], "{}", body);
    }
    // No cases at all handles nothing.
    let none = "    switch (Color)\n    {\n    }";
    assert_eq!(
        switches(&[COLORS, HOLDER, &foo(none)]),
        [(
            6,
            "`switch` on `EColor` has no `default` and doesn't handle `C_Red`, `C_Green`, `C_Blue`"
                .to_string()
        )]
    );
}

#[test]
fn many_missing_values_are_not_all_listed() {
    let object = "class Object;\nenum EDay { D_1, D_2, D_3, D_4, D_5, D_6, D_7 };\nvar EDay Day;\n";
    let foo = "class Foo extends Object;\nfunction F()\n{\n    switch (Day)\n    {\n        case D_1:\n    }\n}\n";
    assert_eq!(
        switches(&[object, foo]),
        [(
            4,
            "`switch` on `EDay` has no `default` and doesn't handle `D_2`, `D_3`, `D_4`, `D_5`, `D_6` and 1 more".to_string()
        )]
    );
}

#[test]
fn the_type_of_the_subject() {
    let case = "        case C_Red:\n            break;\n    }";
    let each = |subject: &str| {
        let body = format!("    switch ({})\n    {{\n{}", subject, case);
        switches(&[COLORS, HOLDER, &foo(&body)]).len()
    };
    for subject in [
        // A name: a local, an argument, one of the class's or a parent's.
        "Local",
        "Arg",
        "Color",
        "Self.Color",
        "(Color)",
        // Other classes' members, and structs'.
        "Other.Color",
        "Both.First",
        "Other.Both.First",
        // Elements of arrays.
        "Palette[1]",
        "Many[0]",
        "Both.Second[1]",
        // What a function returns, and a cast.
        "Pick()",
        "Other.Pick()",
        "Self.Other.Make()",
        "EColor(Raw)",
        "EColor(Count + 1)",
    ] {
        assert_eq!(each(subject), 1, "{}", subject);
    }
    for subject in [
        // Not an enum, or not known to be one.
        "Count",
        "Raw",
        "Count + 1",
        "Palette",
        "Many",
        "Nothing",
        "Other.Nothing",
        "Other.Count",
        "Pick",
        "Palette[1][2]",
        "Count[1]",
        "Both.Nothing",
        "Both.Second",
        "Count.X",
        "Both.First.X",
        "Other.Pick().X",
        "Other().Y",
        "Pick(1)()",
        "byte(Raw)",
        "Nothing()",
        "Nothing.Pick()",
        "Nothing[0]",
        "Both",
        "Nothing.Color",
        "Other.Nothing()",
        "Palette.Color",
        "Odd.X",
        "C_Red",
        "Nada()",
        "class<Holder>(Other)",
        "Both.First()",
        "1",
        "default.Color",
        "Super.Color",
    ] {
        assert_eq!(each(subject), 0, "{}", subject);
    }
}

#[test]
fn a_label_that_isnt_known_to_be_a_value_says_nothing() {
    let wrap = |labels: &str| {
        foo(&format!(
            "    switch (Color)\n    {{\n        case C_Red:\n{}\n    }}",
            labels
        ))
    };
    let other_enum = "class Other extends Object;\nenum EShape { S_Round };\n";
    for labels in [
        "        case 1:",
        "        case Count:",
        "        case Nothing:",
        "        case S_Round:",
        "        case EShape.S_Round:",
        "        case EColor.Nothing:",
        "        case Other.C_Green:",
        "        case Pick():",
        "        case Pair.First:",
        "        case EColor.C_Red.X:",
    ] {
        let source = wrap(labels);
        assert_eq!(
            switches(&[COLORS, HOLDER, other_enum, &source]),
            [],
            "{}",
            labels
        );
    }
}

#[test]
fn a_chain_that_isnt_known_says_nothing() {
    // `Holder`'s parent isn't in the project, so `Color` can't be typed.
    let source = foo("    switch (Color)\n    {\n    }");
    assert_eq!(switches(&[HOLDER, &source]), []);
}

#[test]
fn enums_declared_in_other_classes() {
    // The enum is in a class that neither extends the other.
    let shapes = "class Shapes extends Object;\nenum EShape { S_Round, S_Square };\n";
    let user = "class User extends Object;\nvar Shapes.EShape Shape;\nfunction F()\n{\n    switch (Shape)\n    {\n        case S_Round:\n    }\n}\n";
    // The type is written `Shapes.EShape`, and `User` doesn't extend
    // `Shapes`: the labels are still its values (UCC finds them, as it does
    // for a class that only `dependson`s the one with the enum).
    assert_eq!(
        switches(&[COLORS, shapes, user]),
        [(
            5,
            "`switch` on `EShape` has no `default` and doesn't handle `S_Square`".to_string()
        )]
    );
    let qualified = user.replace("case S_Round:", "case Shapes.EShape.S_Round:");
    assert_eq!(switches(&[COLORS, shapes, &qualified]), []);
    // A subclass of the one declaring the enum sees it.
    let child = "class Child extends Shapes;\nvar EShape Shape;\nfunction F()\n{\n    switch (Shape)\n    {\n        case S_Round:\n    }\n}\n";
    assert_eq!(
        switches(&[COLORS, shapes, child]),
        [(
            5,
            "`switch` on `EShape` has no `default` and doesn't handle `S_Square`".to_string()
        )]
    );
}

#[test]
fn switches_in_states_and_state_code() {
    let source = "class Foo extends Holder;\n\
        state Busy\n\
        {\n\
        \x20   function G()\n\
        \x20   {\n\
        \x20       switch (Color) { case C_Red: }\n\
        \x20   }\n\
        Begin:\n\
        \x20   switch (Pick()) { case C_Red: }\n\
        }\n";
    let found = switches(&[COLORS, HOLDER, source]);
    assert_eq!(found.iter().map(|f| f.0).collect::<Vec<_>>(), [6, 9]);
}

#[test]
fn a_switch_can_be_suppressed_by_naming_it() {
    let body = "    // usagi: ignore[S018]\n    switch (Color) { case C_Red: }\n    switch (Color) { case C_Red: }\n    // usagi: ignore[suspicious]\n    switch (Color) { case C_Red: }";
    assert_eq!(
        switches(&[COLORS, HOLDER, &foo(body)])
            .iter()
            .map(|f| f.0)
            .collect::<Vec<_>>(),
        [8, 10]
    );
}

#[test]
fn an_enum_of_a_struct_of_a_class_the_switching_class_only_depends_on() {
    // `var struct Error { var EKind Type; } Last;` in a class that is not a
    // parent, a local of the struct written `Maker.Error`, and labels that
    // are the enum's values, as in the construction menus of a game.
    let maker = "class Maker extends Object;\n\
        enum EKind { K_None, K_Busy, K_Full, K_Dead };\n\
        var struct Error\n{\n    var EKind Type;\n    var string Text;\n} Last;\n";
    let menu = "class Menu extends Object\n    dependson(Maker);\n\
        function F()\n{\n    local Maker.Error E;\n\
        \x20   switch (E.Type)\n    {\n        case K_Busy:\n            break;\n        case K_Full:\n            break;\n    }\n}\n";
    assert_eq!(
        switches(&[COLORS, maker, menu]),
        [(
            6,
            "`switch` on `EKind` has no `default` and doesn't handle `K_None`, `K_Dead`"
                .to_string()
        )]
    );
    let covered = menu.replace(
        "case K_Full:",
        "case K_Full:\n        case K_None:\n        case K_Dead:",
    );
    assert_eq!(switches(&[COLORS, maker, &covered]), []);
}

// S019 could-be-static.

/// The names of the functions in the last source that could be static.
fn statics(sources: &[&str]) -> Vec<String> {
    let p = project(sources);
    let last = sources.last().unwrap();
    let class = p.classes().last().unwrap();
    project_lint::lint(&p, class, last)
        .into_iter()
        .filter(|e| e.lint == Some(Lint::CouldBeStatic))
        .map(|e| last[e.span.start..e.span.end].to_string())
        .collect()
}

const BASE: &str = "class Base extends Object;\n\
    var int Count;\n\
    const LIMIT = 3;\n\
    static function int Twice(int X) { return X * 2; }\n\
    function Touch() {}\n\
    function Virtual() {}\n\
    event Hook() {}\n\
    native function Nat();\n";

#[test]
fn functions_that_use_no_object() {
    let child = "class Child extends Base;\n\
        var int Own;\n\
        function Pure(int A) { local int L; L = A + LIMIT; return; }\n\
        function CallsStatic(int A) { Twice(A); Default.Count = 1; Static.Twice(1); }\n\
        function UsesOwn() { Own = 1; }\n\
        function UsesInherited() { Count = 1; }\n\
        function ReadsInherited(int A) { A = Count; }\n\
        function CallsInstance() { Touch(); }\n\
        function CallsSelf() { Self.Touch(); }\n\
        function UsesSelf() { Foo(Self); }\n\
        function CallsSuper() { Super.Virtual(); }\n\
        function Virtual() {}\n\
        function UnknownName() { Mystery(); }\n\
        function Pure2() { Pure(1); }\n\
        static function AlreadyStatic() {}\n\
        event Fired() {}\n\
        exec function Console() {}\n\
        native function Nat2();\n\
        function Foo(Object O) {}\n\
        function Assigned() {}\n\
        function Setup() { Handler = Assigned; }\n\
        var delegate Handler;\n\
        delegate OnX() {}\n\
        state S { function InState() {} }\n";
    assert_eq!(
        statics(&[OBJECT, BASE, child]),
        ["Pure", "CallsStatic", "UnknownName", "Foo"]
    );
}

#[test]
fn could_be_static_needs_the_whole_chain() {
    // The parent isn't in the project: `Mystery` might be an inherited variable.
    let orphan = "class Orphan extends Missing;\nfunction F() { Mystery(); }\n";
    assert!(statics(&[OBJECT, orphan]).is_empty());
    // An overridden function can't change its signature, nor can the one it overrides.
    let parent = "class P extends Object;\nfunction F() {}\nfunction G() {}\n";
    let child = "class C extends P;\nfunction F() {}\n";
    assert_eq!(statics(&[OBJECT, parent, child]), Vec::<String>::new());
    assert_eq!(statics(&[OBJECT, parent]), ["F", "G"]);
    // A replicated function needs an object.
    let net = "class N extends Object;\nreplication { reliable if (Role == ROLE_Authority) F; }\nfunction F() {}\n";
    assert!(statics(&[OBJECT, net]).is_empty());
    // A suppression comment works as for other project lints.
    let quiet = "class Q extends Object;\n// usagi: ignore[S019]\nfunction F() {}\n";
    assert!(statics(&[OBJECT, quiet]).is_empty());
}
