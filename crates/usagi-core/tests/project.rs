// Tests for the project index: classes, their parents, and lookups that go
// through the hierarchy.
use std::path::PathBuf;
use usagi_core::ast::{Identifier, Program};
use usagi_core::symbols::{Access, Namespace};
use usagi_core::{
    ClassId, Declaration, Lookup, Origin, Parent, Problem, Project, SymbolKind, SymbolTable,
    parse_program,
};

fn origin(root: usize, path: &str) -> Origin {
    Origin {
        root,
        package: None,
        path: PathBuf::from(path),
    }
}

/// A project of classes in one root, named `<Name>.uc`.
fn project(sources: &[&str]) -> Project {
    let files = sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let table = parse_program(source)
                .unwrap_or_else(|e| panic!("{}\n{:?}", source, e))
                .symbols;
            (origin(0, &format!("{:03}.uc", i)), table)
        })
        .collect();
    Project::build(files)
}

fn class(project: &Project, name: &str) -> ClassId {
    project.class(name).unwrap_or_else(|| panic!("no {}", name))
}

#[track_caller]
fn found(project: &Project, lookup: Lookup) -> (String, String) {
    let declaration = lookup.found().expect("found");
    let symbol = project.symbol(declaration);
    (
        project.name(declaration.class).to_string(),
        symbol.name.to_string(),
    )
}

const OBJECT: &str =
    "class Object;\nenum EBool { B_False, B_True };\nstruct Vector { var float X; };\n";
const ACTOR: &str = "class Actor extends Object;\n\
    var int Health;\n\
    var private int Secret;\n\
    var protected int Shared;\n\
    enum EState { S_Idle, S_Busy, S_Dead };\n\
    function Tick() {}\n\
    function Hurt();\n\
    state Walking {\n\
        function Tick() {}\n\
        function Step() {}\n\
    }\n";
const PAWN: &str = "class Pawn extends Actor;\n\
    var EState Current;\n\
    function Tick() {}\n\
    state Running extends Walking {\n\
        function Other() {}\n\
    }\n\
    state Walking {\n\
        function Step() {}\n\
    }\n";
const PLAYER: &str = "class Player extends Pawn;\nfunction Tick() {}\n";

#[test]
fn indexes_classes_by_name_in_any_case() {
    let p = project(&[OBJECT, ACTOR, PAWN]);
    assert_eq!(p.len(), 3);
    assert!(!p.is_empty());
    let actor = class(&p, "ACTOR");
    assert_eq!(p.name(actor), "Actor");
    assert_eq!(p.class("actor"), Some(actor));
    assert_eq!(p.class("Nothing"), None);
    assert_eq!(p.class("Health"), None); // a name, but not of a class
    assert_eq!(p.classes().count(), 3);
    assert_eq!(actor.index(), 1);
    assert_eq!(p.origin(actor), &origin(0, "001.uc"));
    assert!(p.table(actor).class().is_some());
    assert!(p.problems().is_empty());
    assert!(Project::build(Vec::new()).is_empty());
}

#[test]
fn files_without_a_class_are_left_out() {
    // The grammar always has one; a program built by hand needn't.
    let empty = SymbolTable::build(&Program { statements: vec![] });
    let object = parse_program(OBJECT).unwrap().symbols;
    let p = Project::build(vec![
        (origin(0, "a.uc"), empty),
        (origin(0, "b.uc"), object),
    ]);
    assert_eq!(p.len(), 1);
}

#[test]
fn builds_the_same_in_any_order() {
    let tables = |order: &[usize]| {
        let sources = [OBJECT, ACTOR, PAWN];
        let files = order
            .iter()
            .map(|&i| {
                let table = parse_program(sources[i]).unwrap().symbols;
                (origin(0, &format!("{}.uc", i)), table)
            })
            .collect();
        Project::build(files)
    };
    let (a, b) = (tables(&[0, 1, 2]), tables(&[2, 0, 1]));
    for name in ["Object", "Actor", "Pawn"] {
        assert_eq!(class(&a, name), class(&b, name));
    }
}

#[test]
fn parents_and_children() {
    let p = project(&[OBJECT, ACTOR, PAWN, PLAYER]);
    let (object, actor, pawn, player) = (
        class(&p, "Object"),
        class(&p, "Actor"),
        class(&p, "Pawn"),
        class(&p, "Player"),
    );
    assert_eq!(p.parent(object), &Parent::Root);
    assert_eq!(p.parent(pawn), &Parent::Class(actor));
    assert_eq!(p.children(actor), &[pawn]);
    assert_eq!(p.children(object), &[actor]);
    assert_eq!(
        p.chain(player).collect::<Vec<_>>(),
        [player, pawn, actor, object]
    );
    assert!(p.is_subclass(player, object));
    assert!(p.is_subclass(player, player));
    assert!(!p.is_subclass(actor, pawn));
    assert!(p.is_complete(player));
}

#[test]
fn a_missing_parent_makes_the_chain_incomplete() {
    let p = project(&[ACTOR, PAWN]);
    let (actor, pawn) = (class(&p, "Actor"), class(&p, "Pawn"));
    match p.parent(actor) {
        Parent::Missing(name) => assert_eq!(name.as_str(), "Object"),
        other => panic!("{:?}", other),
    }
    assert!(!p.is_complete(actor));
    assert!(!p.is_complete(pawn));
    // Not there, but that may be because Object isn't: unknown.
    assert_eq!(p.lookup(pawn, Namespace::Value, "Nothing"), Lookup::Unknown);
    assert_eq!(p.lookup(pawn, Namespace::Value, "nothing"), Lookup::Unknown);
    // What is there is found.
    assert_eq!(
        found(&p, p.lookup(pawn, Namespace::Value, "health")).0,
        "Actor"
    );
}

#[test]
fn cycles_are_reported_once_and_cut() {
    let p = project(&[
        "class A extends B;",
        "class B extends C;",
        "class C extends A;",
        "class D extends B;",
        "class E extends E;",
        "class F extends D;",
    ]);
    let id = |n| class(&p, n);
    assert_eq!(
        p.problems(),
        &[
            Problem::Cycle(vec![id("A"), id("B"), id("C")]),
            Problem::Cycle(vec![id("E")]),
        ]
    );
    assert_eq!(p.parent(id("C")), &Parent::Cycle);
    // Classes that lead into the cycle are incomplete, not stuck.
    assert_eq!(p.chain(id("F")).count(), 3); // F, D, B
    assert!(!p.is_complete(id("F")));
    assert_eq!(p.lookup(id("F"), Namespace::Value, "X"), Lookup::Unknown);
    assert_eq!(p.chain(id("E")).count(), 1);
}

#[test]
fn duplicate_classes_keep_the_lowest_root_then_path() {
    let table = |source: &str| parse_program(source).unwrap().symbols;
    let p = Project::build(vec![
        (origin(1, "game/Actor.uc"), table("class Actor; var int A;")),
        (origin(0, "mod/Actor.uc"), table("class actor; var int B;")),
        (origin(0, "mod/Other/Actor.uc"), table("class ACTOR;")),
    ]);
    assert_eq!(p.len(), 1);
    let actor = class(&p, "Actor");
    assert_eq!(p.origin(actor), &origin(0, "mod/Actor.uc"));
    assert_eq!(found(&p, p.lookup(actor, Namespace::Value, "B")).1, "B");
    let dropped: Vec<_> = p
        .problems()
        .iter()
        .map(|problem| match problem {
            Problem::DuplicateClass {
                name,
                kept,
                dropped,
            } => (name.to_string(), kept.clone(), dropped.clone()),
            other => panic!("{:?}", other),
        })
        .collect();
    assert_eq!(
        dropped,
        [
            (
                "ACTOR".to_string(),
                origin(0, "mod/Actor.uc"),
                origin(0, "mod/Other/Actor.uc")
            ),
            (
                "Actor".to_string(),
                origin(0, "mod/Actor.uc"),
                origin(1, "game/Actor.uc")
            ),
        ]
    );
}

#[test]
fn lookup_walks_the_hierarchy() {
    let p = project(&[OBJECT, ACTOR, PAWN, PLAYER]);
    let player = class(&p, "Player");
    // Inherited, and the nearest wins.
    assert_eq!(
        found(&p, p.lookup(player, Namespace::Value, "HEALTH")),
        ("Actor".into(), "Health".into())
    );
    assert_eq!(
        found(&p, p.lookup(player, Namespace::Function, "Tick")).0,
        "Player"
    );
    assert_eq!(
        found(&p, p.lookup(class(&p, "Pawn"), Namespace::Function, "Tick")).0,
        "Pawn"
    );
    assert_eq!(
        found(&p, p.lookup(player, Namespace::Function, "Hurt")).0,
        "Actor"
    );
    // Names of different kinds are looked up separately.
    assert_eq!(
        p.lookup(player, Namespace::Function, "Health"),
        Lookup::NotFound
    );
    assert_eq!(
        found(&p, p.lookup(player, Namespace::State, "Walking")).0,
        "Pawn"
    );
    // A name nothing declares, with every parent known.
    assert_eq!(
        p.lookup(player, Namespace::Value, "Nothing"),
        Lookup::NotFound
    );
    assert_eq!(Lookup::NotFound.found(), None);
    assert_eq!(Lookup::Unknown.found(), None);
}

#[test]
fn access() {
    let p = project(&[OBJECT, ACTOR, PAWN]);
    let (actor, pawn) = (class(&p, "Actor"), class(&p, "Pawn"));
    let get = |from, name| p.lookup(from, Namespace::Value, name).found().unwrap();
    let (secret, shared, health) = (
        get(pawn, "Secret"),
        get(pawn, "Shared"),
        get(pawn, "Health"),
    );
    assert_eq!(p.symbol(secret).access, Access::Private);
    assert!(p.is_accessible(actor, secret));
    assert!(!p.is_accessible(pawn, secret));
    assert!(p.is_accessible(pawn, shared));
    assert!(!p.is_accessible(class(&p, "Object"), shared));
    assert!(p.is_accessible(class(&p, "Object"), health));
}

#[test]
fn types_and_enums() {
    let p = project(&[OBJECT, ACTOR, PAWN]);
    let pawn = class(&p, "Pawn");
    // A bare name, from a parent; and qualified by the class.
    let state = p.resolve_type(pawn, "EState").found().unwrap();
    assert_eq!(found(&p, p.resolve_type(pawn, "estate")).0, "Actor");
    assert_eq!(found(&p, p.resolve_type(pawn, "Actor.EState")).0, "Actor");
    assert_eq!(found(&p, p.resolve_type(pawn, "Object.Vector")).1, "Vector");
    // Declared in a class the one asking doesn't extend: found as qualified.
    assert_eq!(
        found(&p, p.resolve_type(class(&p, "Object"), "Pawn.EState")).0,
        "Actor"
    );
    assert_eq!(p.resolve_type(pawn, "Nothing"), Lookup::NotFound);
    assert_eq!(p.resolve_type(pawn, "Nothing.EState"), Lookup::Unknown);
    assert_eq!(p.resolve_type(pawn, "Actor.Nothing"), Lookup::NotFound);

    let values: Vec<_> = p
        .enum_values(state)
        .into_iter()
        .map(|id| p.table(state.class).symbol(id).name.to_string())
        .collect();
    assert_eq!(values, ["S_Idle", "S_Busy", "S_Dead"]);
    // Values are found by name through the hierarchy too.
    let value = p.lookup(pawn, Namespace::Value, "S_Busy").found().unwrap();
    assert!(matches!(
        p.symbol(value).kind,
        SymbolKind::EnumValue { index: 1, .. }
    ));
    // Not an enum: no values.
    let vector = p.resolve_type(pawn, "Vector").found().unwrap();
    assert!(p.enum_values(vector).is_empty());
}

#[test]
fn functions_in_states() {
    let p = project(&[OBJECT, ACTOR, PAWN, PLAYER]);
    let (actor, pawn, player) = (class(&p, "Actor"), class(&p, "Pawn"), class(&p, "Player"));
    let in_state = |class, state, name| p.lookup_function_in_state(class, state, name);
    // The state's own function replaces the class's.
    assert_eq!(found(&p, in_state(actor, "Walking", "Tick")).0, "Actor");
    let step = in_state(player, "walking", "step").found().unwrap();
    assert_eq!(step.class, pawn); // the subclass's state of that name
    // A state that extends another finds its functions.
    assert_eq!(found(&p, in_state(player, "Running", "Other")).0, "Pawn");
    assert_eq!(found(&p, in_state(player, "Running", "Step")).0, "Pawn");
    // Then the class's.
    assert_eq!(found(&p, in_state(player, "Running", "Hurt")).0, "Actor");
    assert_eq!(
        found(&p, in_state(player, "NoSuchState", "Tick")).0,
        "Player"
    );
    assert_eq!(in_state(player, "Running", "Nothing"), Lookup::NotFound);
    assert_eq!(in_state(player, "Running", "Walking"), Lookup::NotFound);
}

#[test]
fn states_that_extend_each_other_in_a_circle_end() {
    let p = project(&[
        "class Object;\nstate A extends B { const K = 1; function F() {} }\nstate B extends A { }\n",
    ]);
    let object = class(&p, "Object");
    assert_eq!(
        p.lookup_function_in_state(object, "B", "F")
            .found()
            .map(|d| d.class),
        Some(object)
    );
    assert_eq!(
        p.lookup_function_in_state(object, "B", "G"),
        Lookup::NotFound
    );
}

#[test]
fn a_state_declared_twice_uses_the_first() {
    let p = project(&["class Object;\nstate A { function F() {} }\nstate A { function G() {} }\n"]);
    let object = class(&p, "Object");
    assert!(
        p.lookup_function_in_state(object, "A", "F")
            .found()
            .is_some()
    );
    assert_eq!(
        p.lookup_function_in_state(object, "A", "G"),
        Lookup::NotFound
    );
}

#[test]
fn overrides() {
    let p = project(&[OBJECT, ACTOR, PAWN, PLAYER]);
    let (actor, pawn, player) = (class(&p, "Actor"), class(&p, "Pawn"), class(&p, "Player"));
    let tick = |c| p.lookup(c, Namespace::Function, "Tick").found().unwrap();
    assert_eq!(p.overridden(tick(player)), Some(tick(pawn)));
    assert_eq!(p.overridden(tick(pawn)), Some(tick(actor)));
    assert_eq!(p.overridden(tick(actor)), None);
    assert_eq!(p.overriders(tick(actor)), [tick(pawn), tick(player)]);
    assert!(p.overriders(tick(player)).is_empty());
    // Hurt is never overridden.
    let hurt = p
        .lookup(actor, Namespace::Function, "Hurt")
        .found()
        .unwrap();
    assert_eq!(p.overridden(hurt), None);
    assert!(p.overriders(hurt).is_empty());
    // A function that overrides nothing has no original.
    let other = Declaration {
        class: pawn,
        symbol: tick(pawn).symbol,
    };
    assert_eq!(p.overridden(other), Some(tick(actor)));
}

#[test]
fn the_object_class_itself() {
    // A class that extends nothing and isn't `Object` is a root too.
    let p = project(&["class Loose;\nvar int X;\n"]);
    let loose = class(&p, "Loose");
    assert!(p.is_complete(loose));
    assert_eq!(p.lookup(loose, Namespace::Value, "Y"), Lookup::NotFound);
    assert!(
        p.overridden(Declaration {
            class: loose,
            symbol: p.table(loose).iter().next().unwrap().0
        })
        .is_none()
    );
}

/// A class in `package`, in one root.
fn in_package(package: &str, source: &str) -> (Origin, SymbolTable) {
    let table = parse_program(source).unwrap().symbols;
    let origin = Origin {
        root: 0,
        package: Some(package.to_string()),
        path: PathBuf::from(format!("{}/Classes/{}.uc", package, source.len())),
    };
    (origin, table)
}

#[test]
fn classes_of_the_same_name_in_different_packages() {
    let p = Project::build(vec![
        in_package("Base", "class Object;"),
        in_package("A", "class Thing extends Object; var int FromA;"),
        in_package("B", "class thing extends Object; var int FromB;"),
        // Each extends the one of its own package, unless it says which.
        in_package("A", "class InA extends Thing;"),
        in_package("B", "class InB extends Thing;"),
        in_package("B", "class Far extends A.Thing;"),
        in_package("B", "class Wrong extends C.Thing;"),
        in_package("B", "class Lost extends Nothing.Thing;"),
        in_package("B", "class Absent extends Nope;"),
    ]);
    let (a, b) = (class(&p, "A.Thing"), class(&p, "b.THING"));
    assert_ne!(a, b);
    assert_eq!(p.origin(a).package.as_deref(), Some("A"));
    assert_eq!(p.class("Thing"), Some(a)); // the first, by path
    assert_eq!(p.class("C.Thing"), None);
    assert_eq!(p.class("A.Nothing"), None);
    assert_eq!(p.parent(class(&p, "InA")), &Parent::Class(a));
    assert_eq!(p.parent(class(&p, "InB")), &Parent::Class(b));
    assert_eq!(p.parent(class(&p, "Far")), &Parent::Class(a));
    // A package nothing is in: that class is missing, with its name as written.
    for name in ["Wrong", "Lost"] {
        assert!(
            matches!(p.parent(class(&p, name)), Parent::Missing(_)),
            "{}",
            name
        );
    }
    assert!(matches!(p.parent(class(&p, "Absent")), Parent::Missing(_)));
    // From the class, a bare name means the one in its package.
    let in_b = class(&p, "InB");
    assert_eq!(p.class_from(in_b, "Thing"), Some(b));
    assert_eq!(p.class_from(in_b, "A.Thing"), Some(a));
    assert_eq!(p.class_from(in_b, "Nothing"), None);
    assert_eq!(p.class_from(in_b, "Object"), p.class("Object"));
    // Types are looked up in the class named, whichever package it is in.
    assert_eq!(p.resolve_type(in_b, "A.Thing.X"), Lookup::NotFound);
    assert_eq!(
        p.problems(),
        &[Problem::AmbiguousClass {
            name: Identifier::new("thing"),
            used: p.origin(a).clone(),
            also: p.origin(b).clone(),
        }]
    );
}

#[test]
fn a_class_with_no_known_package_stands_in_for_any() {
    let table = |source: &str| parse_program(source).unwrap().symbols;
    let (known, _) = in_package("K", "class Known extends Loose;");
    let p = Project::build(vec![
        (origin(0, "Loose.uc"), table("class Loose;")),
        (known, table("class Known extends Loose;")),
        (
            origin(0, "Other.uc"),
            table("class Other extends Anywhere.Loose;"),
        ),
    ]);
    assert_eq!(
        p.parent(class(&p, "Other")),
        &Parent::Class(class(&p, "Loose"))
    );
    assert_eq!(
        p.parent(class(&p, "Known")),
        &Parent::Class(class(&p, "Loose"))
    );
}

#[test]
fn types_qualified_by_a_class_in_a_package() {
    let p = Project::build(vec![
        in_package("Base", "class Object; enum EOne { O_A };"),
        in_package("Other", "class Thing extends Object; enum ETwo { T_A };"),
    ]);
    let thing = class(&p, "Thing");
    assert_eq!(
        p.resolve_type(thing, "Other.Thing.ETwo")
            .found()
            .map(|d| d.class),
        Some(thing)
    );
    assert_eq!(
        p.resolve_type(thing, "Thing.EOne").found().map(|d| d.class),
        Some(class(&p, "Object"))
    );
}
