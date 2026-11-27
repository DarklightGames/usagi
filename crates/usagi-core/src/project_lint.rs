//! The lints that look at other classes: they run on a class once the
//! [`Project`] is indexed, from what its [`SymbolTable`](crate::SymbolTable)
//! kept, without parsing the file again. Each is opt-in (see
//! [`Lint::needs_project`]).
//!
//! A lint reports a name only when the project knows the whole chain of the
//! class's parents, so a class whose parent isn't in the project is never
//! reported for what it might inherit.

use crate::ast::{FunctionModifierType, FunctionTypeType, Identifier};
use crate::lint::Lint;
use crate::parser::ProgramError;
use crate::project::Declaration;
use crate::project::{ClassId, Lookup, Project};
use crate::suppression::Suppressions;
use crate::symbols::{Namespace, Operand, ReferenceKind, SymbolId, SymbolKind};
use crate::typing;
use crate::{LineIndex, source};
use std::collections::HashSet;

/// The problems the project lints find in `class`, with the lines of their
/// spans set and the ones that a suppression comment in `source` (the text of
/// the class's file) suppresses removed. Not filtered by the selection of
/// lints: that's up to the caller, with [`Lint::UndeclaredDefaultProperty`]
/// and the others.
pub fn lint(project: &Project, class: ClassId, source: &str) -> Vec<ProgramError> {
    let mut errors = check(project, class);
    if errors.is_empty() {
        return errors;
    }
    let lines = LineIndex::new(source);
    for error in &mut errors {
        error.span.line = Some(lines.line(error.span.start));
    }
    let comments = source::comment_ranges(source);
    Suppressions::parse(source, &comments).retain(source, &mut errors);
    errors
}

/// The problems the project lints find in `class`, without lines or
/// suppressions.
pub fn check(project: &Project, class: ClassId) -> Vec<ProgramError> {
    let mut errors = Vec::new();
    undeclared_default_properties(project, class, &mut errors);
    non_exhaustive_switches(project, class, &mut errors);
    could_be_static(project, class, &mut errors);
    errors
}

fn undeclared_default_properties(
    project: &Project,
    class: ClassId,
    errors: &mut Vec<ProgramError>,
) {
    let lint = Lint::UndeclaredDefaultProperty;
    for unresolved in project.table(class).unresolved_default_properties() {
        // A delegate is assigned there too, and is declared as a function.
        let declared = |namespace| project.lookup(class, namespace, &unresolved.name);
        if declared(Namespace::Value) == Lookup::NotFound
            && declared(Namespace::Function) == Lookup::NotFound
        {
            errors.push(ProgramError {
                message: format!(
                    "`{}` isn't declared in `{}` or any class it extends",
                    unresolved.name.as_str(),
                    project.name(class)
                ),
                span: unresolved.span,
                severity: lint.severity(),
                lint: Some(lint),
                fix: None,
            });
        }
    }
}

/// How many missing values a message lists.
const MISSING_SHOWN: usize = 5;

fn non_exhaustive_switches(project: &Project, class: ClassId, errors: &mut Vec<ProgramError>) {
    let lint = Lint::NonExhaustiveSwitch;
    let table = project.table(class);
    for switch in table.switches().iter().filter(|s| !s.has_default) {
        let Some(subject) = typing::type_of(project, class, switch.scope, &switch.subject) else {
            continue;
        };
        let Some(enum_) = typing::enum_of(project, &subject) else {
            continue;
        };
        // A label that isn't a value of this enum, or isn't known to be, says
        // nothing about which values are handled.
        let handled: Option<HashSet<SymbolId>> = switch
            .labels
            .iter()
            .map(|label| enum_value(project, class, enum_, label))
            .collect();
        let Some(handled) = handled else { continue };
        let missing: Vec<&str> = project
            .enum_values(enum_)
            .into_iter()
            .filter(|value| !handled.contains(value))
            .map(|value| project.table(enum_.class).symbol(value).name.as_str())
            .collect();
        if missing.is_empty() {
            continue;
        }
        let listed = missing
            .iter()
            .take(MISSING_SHOWN)
            .map(|name| format!("`{}`", name))
            .collect::<Vec<_>>()
            .join(", ");
        let more = match missing.len().saturating_sub(MISSING_SHOWN) {
            0 => String::new(),
            n => format!(" and {} more", n),
        };
        let enum_name = &project.symbol(enum_).name;
        errors.push(ProgramError {
            message: format!(
                "`switch` on `{}` has no `default` and doesn't handle {}{}",
                enum_name.as_str(),
                listed,
                more
            ),
            span: switch.span,
            severity: lint.severity(),
            lint: Some(lint),
            fix: None,
        });
    }
}

/// The value of `enum_` that a `case` label is, if it is one: `Value` or
/// `Enum.Value`. A bare `Value` is looked for among the enum's own values, not
/// through the class's parents: the enum may be declared in a class that this
/// one only depends on (`dependson`), whose enum values UCC finds all the same.
fn enum_value(
    project: &Project,
    class: ClassId,
    enum_: Declaration,
    label: &Operand,
) -> Option<SymbolId> {
    let table = project.table(enum_.class);
    let named = |name: &Identifier| {
        project
            .enum_values(enum_)
            .into_iter()
            .find(|&value| table.symbol(value).name == *name)
    };
    match label {
        Operand::Name(name) => named(name),
        Operand::Member(owner, name) => {
            let Operand::Name(owner) = owner.as_ref() else {
                return None;
            };
            let written = project.resolve_type(class, owner).found()?;
            (written == enum_).then(|| named(name)).flatten()
        }
        _ => None,
    }
}

/// Functions that aren't plain, free-standing code: they are called by the
/// engine or by console commands on an object, or have no body.
const NEVER_STATIC: &[FunctionModifierType] = &[
    FunctionModifierType::Exec,
    FunctionModifierType::Iterator,
    FunctionModifierType::Latent,
    FunctionModifierType::Native,
    FunctionModifierType::Intrinsic,
    FunctionModifierType::Singular,
];

/// Whether a use of `name` needs an object, or `None` if the project can't
/// tell: a name that isn't declared anywhere in a complete chain of classes
/// (a class name in a cast, for one) needs nothing.
fn needs_object(
    project: &Project,
    class: ClassId,
    namespaces: &[Namespace],
    name: &str,
) -> Option<bool> {
    if ["self", "super", "global"]
        .iter()
        .any(|keyword| name.eq_ignore_ascii_case(keyword))
    {
        return Some(true);
    }
    for &namespace in namespaces {
        match project.lookup(class, namespace, name) {
            Lookup::Found(declaration) => {
                return Some(match &project.symbol(declaration).kind {
                    SymbolKind::Variable { .. } => true,
                    SymbolKind::Function { modifiers, .. } => {
                        !modifiers.contains(&FunctionModifierType::Static)
                    }
                    _ => false,
                });
            }
            Lookup::Unknown => return None,
            Lookup::NotFound => {}
        }
    }
    Some(false)
}

fn could_be_static(project: &Project, class: ClassId, errors: &mut Vec<ProgramError>) {
    let lint = Lint::CouldBeStatic;
    let table = project.table(class);
    for (id, symbol) in table.iter() {
        let SymbolKind::Function {
            types,
            modifiers,
            has_body: true,
            ..
        } = &symbol.kind
        else {
            continue;
        };
        // Only a class's own plain functions: not a state's, not an event,
        // delegate or operator.
        if symbol.scope != crate::symbols::ScopeId::CLASS
            || types != &[FunctionTypeType::Function]
            || modifiers.contains(&FunctionModifierType::Static)
            || NEVER_STATIC.iter().any(|m| modifiers.contains(m))
        {
            continue;
        }
        let scope = table
            .scope_of(symbol.span.start)
            .expect("a function with a body has a scope");
        // Replicated, or used as a value (assigned to a delegate): it needs
        // an object.
        if table
            .references_to(id)
            .any(|r| !matches!(r.kind, ReferenceKind::Call | ReferenceKind::Name))
        {
            continue;
        }
        let declaration = Declaration { class, symbol: id };
        if project.overridden(declaration).is_some() || !project.overriders(declaration).is_empty()
        {
            continue;
        }
        if table.instance_use(scope).is_some() {
            continue;
        }
        let uses_object = table
            .unresolved_uses()
            .iter()
            .filter(|u| u.scope == scope && u.instance)
            .map(|u| needs_object(project, class, &u.namespaces, u.name.as_str()))
            .try_fold(false, |any, needs| needs.map(|needs| any || needs));
        if uses_object != Some(false) {
            continue;
        }
        errors.push(ProgramError {
            message: format!(
                "`{}` doesn't use the object it's called on, and could be `static`",
                symbol.name.as_str()
            ),
            span: symbol.span,
            severity: lint.severity(),
            lint: Some(lint),
            fix: None,
        });
    }
}
