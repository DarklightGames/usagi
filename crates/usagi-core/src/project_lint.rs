//! The lints that look at other classes: they run on a class once the
//! [`Project`] is indexed, from what its [`SymbolTable`](crate::SymbolTable)
//! kept, without parsing the file again. Each is opt-in (see
//! [`Lint::needs_project`]).
//!
//! A lint reports a name only when the project knows the whole chain of the
//! class's parents, so a class whose parent isn't in the project is never
//! reported for what it might inherit.

use crate::ast::Identifier;
use crate::lint::Lint;
use crate::parser::ProgramError;
use crate::project::Declaration;
use crate::project::{ClassId, Lookup, Project};
use crate::suppression::Suppressions;
use crate::symbols::{Namespace, Operand, SymbolId};
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
