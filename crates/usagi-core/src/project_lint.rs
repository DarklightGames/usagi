//! The lints that look at other classes: they run on a class once the
//! [`Project`] is indexed, from what its [`SymbolTable`](crate::SymbolTable)
//! kept, without parsing the file again. Each is opt-in (see
//! [`Lint::needs_project`]).
//!
//! A lint reports a name only when the project knows the whole chain of the
//! class's parents, so a class whose parent isn't in the project is never
//! reported for what it might inherit.

use crate::lint::Lint;
use crate::parser::ProgramError;
use crate::project::{ClassId, Lookup, Project};
use crate::suppression::Suppressions;
use crate::symbols::Namespace;
use crate::{LineIndex, source};

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
