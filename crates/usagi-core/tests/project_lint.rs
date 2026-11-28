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

/// The `(name, message)` of each C018 problem in the class named `class`.
fn actor_references(p: &Project, class: &str) -> Vec<(String, String)> {
    let class = p.class(class).unwrap();
    let table = p.table(class);
    project_lint::check(p, class)
        .into_iter()
        .filter(|e| e.lint == Some(Lint::ActorReferenceInObject))
        .map(|e| {
            assert_eq!(e.severity, Lint::ActorReferenceInObject.severity());
            let name = table
                .iter()
                .find(|(_, s)| s.span.start == e.span.start)
                .map(|(_, s)| s.name.as_str().to_string())
                .expect("the span is a variable's");
            (name, e.message)
        })
        .collect()
}

fn names(found: Vec<(String, String)>) -> Vec<String> {
    found.into_iter().map(|(name, _)| name).collect()
}

const FOO_ACTOR: &str = "class FooActor extends Actor;\n";

#[test]
fn an_object_that_holds_an_actor() {
    let bar = "class BarObject extends Object;\n\
        var FooActor MyCoolFoo;\n\
        var Actor Plain, Second;\n\
        var array<FooActor> Many;\n\
        var FooActor Fixed[2];\n\
        var array< array<Actor> > Nested;\n\
        var class<FooActor> FooClass;\n\
        var Object Other;\n\
        var int Count;\n\
        var EColor Color;\n\
        var Unknown Mystery;\n\
        function F() { local FooActor L; }\n";
    let p = project(&[OBJECT, ACTOR, FOO_ACTOR, bar]);
    let found = actor_references(&p, "BarObject");
    assert_eq!(
        found[0].1,
        "`MyCoolFoo` in `BarObject`, which isn't an actor, holds a reference to an actor \
         (`FooActor`): the garbage collector can crash the game on it"
    );
    assert_eq!(
        names(found),
        ["MyCoolFoo", "Plain", "Second", "Many", "Fixed", "Nested"]
    );
}

#[test]
fn an_actor_may_hold_an_actor() {
    let p = project(&[
        OBJECT,
        ACTOR,
        FOO_ACTOR,
        "class Baz extends FooActor;\nvar Actor A;\n",
    ]);
    assert!(actor_references(&p, "Baz").is_empty());
    // A class whose parents aren't all known may be an actor.
    let p = project(&[
        OBJECT,
        ACTOR,
        "class Orphan extends Missing;\nvar Actor A;\n",
    ]);
    assert!(actor_references(&p, "Orphan").is_empty());
    // Without `Actor` in the project, nothing is an actor.
    let p = project(&[OBJECT, "class Lone extends Object;\nvar Lone L;\n"]);
    assert!(actor_references(&p, "Lone").is_empty());
}

#[test]
fn an_actor_in_a_struct() {
    let bar = "class BarObject extends Object;\n\
        struct Inner { var int N; var FooActor Foo; };\n\
        struct Outer { var float F; var array<Inner> Inners; };\n\
        struct Safe { var int N; var enum ELocal { L_A } E; };\n\
        struct Loop1 { var Loop2 L; };\n\
        struct Loop2 { var Loop1 L; var Safe S; };\n\
        var Inner I;\n\
        var Outer O;\n\
        var Safe S;\n\
        var Loop1 L;\n";
    let p = project(&[OBJECT, ACTOR, FOO_ACTOR, bar]);
    let found = actor_references(&p, "BarObject");
    assert_eq!(names(found.clone()), ["I", "O"]);
    assert!(found[1].1.contains("(`FooActor`) (through `Inners.Foo`)"));
    // A struct declared in a parent class.
    let p = project(&[
        OBJECT,
        ACTOR,
        FOO_ACTOR,
        bar,
        "class Sub extends BarObject;\nvar Inner I;\n",
    ]);
    assert_eq!(names(actor_references(&p, "Sub")), ["I"]);
}

#[test]
fn the_engines_actor_is_preferred() {
    let file = |package: &str, path: &str, source: &str| {
        let origin = Origin {
            root: 0,
            package: Some(package.to_string()),
            path: PathBuf::from(path),
        };
        (origin, parse_program(source).unwrap().symbols)
    };
    let p = Project::build(vec![
        file("Core", "a.uc", OBJECT),
        // A class named `Actor` in another package, found first.
        file("Aaa", "b.uc", "class Actor extends Object;\n"),
        file("Engine", "c.uc", ACTOR),
        file(
            "Mod",
            "d.uc",
            "class Bar extends Object;\nvar Engine.Actor A;\nvar Aaa.Actor B;\n",
        ),
    ]);
    assert_eq!(names(actor_references(&p, "Bar")), ["A"]);
}

#[test]
fn an_actor_reference_can_be_suppressed() {
    let bar = "class BarObject extends Object;\n\
        var Actor A; // usagi: ignore[C018]\n\
        var Actor B;\n";
    let p = project(&[OBJECT, ACTOR, bar]);
    let class = p.class("BarObject").unwrap();
    let lines: Vec<usize> = project_lint::lint(&p, class, bar)
        .iter()
        .map(|e| e.span.line.unwrap())
        .collect();
    assert_eq!(lines, [3]);
}

const HELPERS: &str = "class Helpers extends Object;\n\
    struct SPoint { var int X; };\n\
    var Helpers Next;\n\
    var int Ints[2];\n\
    var Helpers Many[2];\n\
    function Fill(out Helpers H, Helpers In) {}\n\
    function Take(Helpers H) { local int L; }\n\
    static function Helpers Make() { return None; }\n\
    function Helpers Self2() { return self; }\n\
    delegate OnDone(Helpers H);\n";

/// The `(line, text)` of each C019 read in the last source's class.
fn unassigned(sources: &[&str]) -> Vec<(usize, String)> {
    let p = project(sources);
    let last = sources.last().unwrap();
    let class = p.classes().last().unwrap();
    project_lint::lint(&p, class, last)
        .into_iter()
        .filter(|e| e.lint == Some(Lint::UnassignedObjectLocal))
        .map(|e| {
            assert_eq!(e.severity, Lint::UnassignedObjectLocal.severity());
            (
                e.span.line.unwrap(),
                last[e.span.start..e.span.end].to_string(),
            )
        })
        .collect()
}

/// [`unassigned`] for a class extending `Helpers` whose only function has
/// `body`, with `locals` declared first; lines count from the body's first.
fn unassigned_in(locals: &str, body: &str) -> Vec<(usize, String)> {
    let source = format!(
        "class T extends Helpers;\nfunction F()\n{{\n{}\n{}\n}}\n",
        locals, body
    );
    unassigned(&[OBJECT, HELPERS, &source])
        .into_iter()
        .map(|(line, text)| (line - 3 - locals.lines().count(), text))
        .collect()
}

fn found(reads: &[(usize, &str)]) -> Vec<(usize, String)> {
    reads.iter().map(|&(l, t)| (l, t.to_string())).collect()
}

#[test]
fn a_local_that_nothing_assigns() {
    let p = project(&[
        OBJECT,
        HELPERS,
        "class T extends Helpers;\nfunction F()\n{\n    local Helpers H;\n    H.Take(None);\n}\n",
    ]);
    let class = p.class("T").unwrap();
    let errors: Vec<_> = project_lint::check(&p, class)
        .into_iter()
        .filter(|e| e.lint == Some(Lint::UnassignedObjectLocal))
        .collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "`H` is always `None` here: nothing can have assigned it"
    );
    assert_eq!(
        unassigned_in(
            "local Helpers H, G;\nlocal class<Helpers> C;\nlocal int I;\nlocal SPoint S;\nlocal Helpers Arr[2];\nlocal array<Helpers> Dyn;",
            "if (H == None) {}\nI = I + 1;\nTake(H);\nTake(C.default.Next);\nS.X = I;\nTake(Arr[0]);\nTake(Dyn[0]);\nG = H;\nTake(G);",
        ),
        found(&[(1, "H"), (3, "H"), (4, "C"), (8, "H")])
    );
}

#[test]
fn assignments_that_reach_a_read() {
    assert!(unassigned_in("local Helpers H;", "H = Make();\nTake(H);\nH.Take(H);").is_empty());
    // The value is worked out before it's assigned, and anything else on
    // the left is a read.
    assert_eq!(
        unassigned_in(
            "local Helpers H, G;",
            "H = H.Self2();\nG.Next = H;\n(H) = None;\n"
        ),
        found(&[(1, "H"), (2, "G")])
    );
    // Code after `return` is never run.
    assert!(unassigned_in("local Helpers H;", "return;\nTake(H);").is_empty());
}

#[test]
fn out_arguments() {
    // `Fill` assigns its first argument, not its second; `Take` assigns none.
    assert_eq!(
        unassigned_in(
            "local Helpers A, B, C, D;",
            "Fill(A, B);\nTake(A);\nTake(B);\nTake(C);\nTake(C);\nFill(D, D);"
        ),
        found(&[(1, "B"), (3, "B"), (4, "C"), (5, "C"), (6, "D")])
    );
    // A call on an object, and arguments past the last one.
    assert_eq!(
        unassigned_in(
            "local Helpers A, B, O;",
            "O = Make();\nO.Fill(A, None);\nTake(A);\nO.Take(B);\nTake(None, B);\nTake(, B);"
        ),
        found(&[(4, "B"), (5, "B"), (6, "B")])
    );
    // `static.F` is the class's own; `class'X'.static.F` isn't looked up.
    assert_eq!(
        unassigned_in(
            "local Helpers A, B;",
            "static.Take(A);\nclass'Helpers'.static.Fill(B);\nTake(B);"
        ),
        found(&[(1, "A")])
    );
    // A call whose function can't be found may assign anything it's given:
    // on a static array, a struct or a name nobody declares.
    assert!(
        unassigned_in(
            "local Helpers A, B, C;\nlocal SPoint S;",
            "Many.Fill(A);\nTake(A);\nS.Fill(B);\nTake(B);\nNobody.Fill(C);\nTake(C);"
        )
        .is_empty()
    );
    // A call whose function can't be found may assign anything it's given.
    assert!(
        unassigned_in(
            "local Helpers A, B, C, D, E, F2;\nlocal int N;",
            "Many[0](D);\nTake(D);\nself(E);\nTake(E);\nMystery(A);\nTake(A);\nN.Fill(B);\nTake(B);\nInts[0].Fill(C);\nTake(C);\nSuper.Take(D);\nTake(D);\nNext.Mystery(E);\nTake(E);\nMake()(F2);\nTake(F2);"
        )
        .is_empty()
    );
    // A cast is not a call, to a class, a struct or an enum; but a function
    // of that name is called.
    assert_eq!(
        unassigned_in(
            "local Helpers A;\nlocal Object O;\nlocal int N;",
            "O = Helpers(A);\nN = SPoint(A);\nN = EColor(A);\nTake(A);"
        ),
        found(&[(1, "A"), (2, "A"), (3, "A"), (4, "A")])
    );
    assert!(
        unassigned(&[
            OBJECT,
            HELPERS,
            "class T extends Helpers;\nfunction Helpers(out Object O) {}\nfunction F() { local Object A; Helpers(A); Take(A); }\n",
        ])
        .is_empty()
    );
    // `global.F` and delegates.
    assert_eq!(
        unassigned_in("local Helpers A, B;", "global.Take(A);\nOnDone(B);"),
        found(&[(1, "A"), (2, "B")])
    );
}

#[test]
fn functions_declared_in_the_class_and_its_states() {
    let source = "class T extends Helpers;\n\
        function Mine(out Helpers H) {}\n\
        function Theirs(Helpers H) {}\n\
        function F()\n\
        {\n\
        \x20   local Helpers A, B;\n\
        \x20   Mine(A);\n\
        \x20   Theirs(A);\n\
        \x20   Theirs(B);\n\
        }\n\
        state S\n\
        {\n\
        \x20   function InState(out Helpers H) {}\n\
        \x20   function G() { local Helpers C; InState(C); Take(C); }\n\
        }\n";
    assert_eq!(unassigned(&[OBJECT, HELPERS, source]), found(&[(9, "B")]));
}

#[test]
fn branches_join() {
    let locals = "local Helpers H;\nlocal bool B;";
    assert!(unassigned_in(locals, "if (B) H = Make();\nTake(H);").is_empty());
    assert!(
        unassigned_in(
            locals,
            "if (B) {} else if (B) {} else H = Make();\nTake(H);"
        )
        .is_empty()
    );
    assert!(unassigned_in(locals, "if (B) {} else if (B) H = Make();\nTake(H);").is_empty());
    assert!(unassigned_in(locals, "if (B) return; else H = Make();\nTake(H);").is_empty());
    assert_eq!(
        unassigned_in(
            locals,
            "if (B);\nif (B) Take(H); else Take(H);\nif (B) {}\nTake(H);"
        ),
        found(&[(2, "H"), (2, "H"), (4, "H")])
    );
    assert_eq!(
        unassigned_in(
            locals,
            "if (B) Take(H);\nelse if (B) Take(H);\nelse return;\nTake(H);"
        ),
        found(&[(1, "H"), (2, "H"), (4, "H")])
    );
}

#[test]
fn loops_carry_assignments_round() {
    let locals = "local Helpers Best, H, Prev;\nlocal int I;";
    // The best-candidate idiom: the assignment reaches the read by the loop.
    assert!(
        unassigned_in(
            locals,
            "for (I = 0; I < 3; I++) { if (Best == None) Best = Make(); }"
        )
        .is_empty()
    );
    assert!(unassigned_in(locals, "while (I < 3) { Take(Prev); Prev = Make(); I++; }").is_empty());
    assert!(unassigned_in(locals, "do { Take(Prev); Prev = Make(); } until (I > 3);").is_empty());
    assert!(unassigned_in(locals, "for (;;) { Take(Prev); Prev = Make(); }").is_empty());
    // A loop that never assigns.
    assert_eq!(
        unassigned_in(
            locals,
            "for (I = 0; I < 3; I++) Take(H);\nwhile (H != None) {}\ndo Take(H); until (I > 3);"
        ),
        found(&[(1, "H"), (2, "H"), (3, "H")])
    );
    // `break` leaves the loop with its state; `continue` goes round.
    assert!(unassigned_in(locals, "while (I < 3) { H = Make(); break; }\nTake(H);").is_empty());
    assert!(
        unassigned_in(
            locals,
            "for (I = 0; I < 3; I++) { Take(Prev); if (I > 0) { Prev = Make(); continue; } }"
        )
        .is_empty()
    );
    assert!(
        unassigned_in(
            locals,
            "do { Take(Prev); if (I > 0) { Prev = Make(); continue; } } until (I > 3);"
        )
        .is_empty()
    );
    assert!(
        unassigned_in(
            locals,
            "while (I < 3) { Take(Prev); if (I > 0) { Prev = Make(); continue; } }"
        )
        .is_empty()
    );
    // Outside a loop, `break` and `continue` lead nowhere.
    assert!(unassigned_in(locals, "break;\nTake(H);\ncontinue;\nTake(H);").is_empty());
}

#[test]
fn foreach_assigns_its_variables() {
    let locals = "local Helpers H, G;\nlocal Actor A;\nlocal vector V;";
    assert!(
        unassigned_in(
            locals,
            "foreach AllObjects(class'Helpers', H) Take(H);\nTake(H);"
        )
        .is_empty()
    );
    assert!(
        unassigned_in(
            locals,
            "foreach (Next.AllObjects(class'Helpers', H, V)) { if (G == None) G = H; }\nTake(G);"
        )
        .is_empty()
    );
    assert_eq!(
        unassigned_in(locals, "foreach AllObjects(class'Helpers', H) Take(G);"),
        found(&[(1, "G")])
    );
}

#[test]
fn switches_fall_through() {
    let locals = "local Helpers H;\nlocal int I;";
    // Without a default, the switch may run no case.
    assert_eq!(
        unassigned_in(
            locals,
            "switch (I) { case 0: H = Make(); break; }\nTake(H);"
        ),
        Vec::<(usize, String)>::new()
    );
    assert_eq!(
        unassigned_in(
            locals,
            "switch (I) { case 0: Take(H); case 1: H = Make(); default: Take(H); }"
        ),
        found(&[(1, "H")])
    );
    assert_eq!(
        unassigned_in(
            locals,
            "switch (I) { case 0: return; default: break; }\nTake(H);"
        ),
        found(&[(2, "H")])
    );
    // `continue` in a switch goes round the loop around it.
    assert!(
        unassigned_in(
            locals,
            "while (I < 3) { Take(H); switch (I) { case 0: H = Make(); continue; } return; }"
        )
        .is_empty()
    );
}

#[test]
fn expressions_are_looked_into() {
    let locals = "local Helpers H;\nlocal class<Helpers> C;\nlocal Object O;";
    assert_eq!(
        unassigned_in(
            locals,
            "O = new(H) C;\nO = new C;\nO = new class'Helpers';\nO = Helpers(H);\nO = (H);\nO = H.Next.default.Next;\nO = H.static.Make();\nO = default.Next;\nO = static.Make();"
        ),
        found(&[
            (1, "H"),
            (1, "C"),
            (2, "C"),
            (4, "H"),
            (5, "H"),
            (6, "H"),
            (7, "H")
        ])
    );
}

#[test]
fn unassigned_locals_need_their_types() {
    // Any type may be a struct or an enum of the missing parent, even one
    // with a class's name.
    let orphan = "class Orphan extends Missing;\n\
        function F() { local Thing T; local Orphan O; T.X(); O.X(); }\n";
    assert!(unassigned(&[OBJECT, orphan]).is_empty());
    // A type nobody declares.
    let lost = "class Lost extends Object;\nfunction F() { local Nowhere N; N.X(); }\n";
    assert!(unassigned(&[OBJECT, lost]).is_empty());
}

#[test]
fn functions_with_goto_are_not_looked_at() {
    for body in [
        "goto 'L';\nL:\nTake(H);",
        "if (I > 0) { L: Take(H); } else Take(H);",
        "while (I > 0) L: Take(H);",
        "for (;;) { L: Take(H); }",
        "do { L: Take(H); } until (I > 0);",
        "foreach AllObjects(class'Helpers', G) { L: Take(H); }",
        "switch (I) { case 0: L: Take(H); }",
    ] {
        assert!(
            unassigned_in("local Helpers H, G;\nlocal int I;", body).is_empty(),
            "{}",
            body
        );
    }
    // The same without the label.
    assert_eq!(
        unassigned_in(
            "local Helpers H;\nlocal int I;",
            "switch (I) { case 0: Take(H); }"
        ),
        found(&[(1, "H")])
    );
}
