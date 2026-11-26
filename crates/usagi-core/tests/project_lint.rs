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
