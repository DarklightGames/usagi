//! The type of an expression, as far as a lint needs it: names, `Self`,
//! members, array elements and the results of calls, looked up through the
//! [`Project`]. It is not a type checker. What it can't work out (operators,
//! literals, anything it doesn't know) has no type, and a lint says nothing
//! about it.

use crate::ast::Identifier;
use crate::project::{ClassId, Declaration, Lookup, Project};
use crate::symbols::{Namespace, Operand, ScopeId, ScopeKind, SymbolKind, TypeRef};

/// The type of an expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Typed {
    pub type_: TypeRef,
    /// A static array (`var int X[4]`): the type is its elements'.
    pub static_array: bool,
    /// The class the names in `type_` are written in, which may be a parent
    /// of the one the expression is in.
    pub class: ClassId,
}

/// The type of `operand`, in `scope` of `class`.
pub fn type_of(
    project: &Project,
    class: ClassId,
    scope: ScopeId,
    operand: &Operand,
) -> Option<Typed> {
    match operand {
        Operand::Name(name) => {
            let table = project.table(class);
            match table.lookup(scope, Namespace::Value, name) {
                Some(symbol) => typed(&table.symbol(symbol).kind, class),
                None => declared(project, project.lookup(class, Namespace::Value, name)),
            }
        }
        Operand::This => Some(Typed {
            type_: TypeRef::Named(Identifier::new(project.name(class))),
            static_array: false,
            class,
        }),
        Operand::Member(base, name) => {
            let base = type_of(project, class, scope, base)?;
            member(project, &base, Namespace::Value, name)
        }
        Operand::Index(base) => {
            let base = type_of(project, class, scope, base)?;
            match base.type_ {
                _ if base.static_array => Some(Typed {
                    static_array: false,
                    ..base
                }),
                TypeRef::Array(element) => Some(Typed {
                    type_: *element,
                    static_array: false,
                    class: base.class,
                }),
                _ => None,
            }
        }
        Operand::Call(callee) => match callee.as_ref() {
            Operand::Name(name) => {
                let function = project.lookup(class, Namespace::Function, name);
                match function {
                    Lookup::Found(_) => declared(project, function),
                    // `EColor(X)`, `Pawn(X)`: a cast to the type.
                    _ => named_type(project, class, name),
                }
            }
            Operand::Member(base, name) => {
                let base = type_of(project, class, scope, base)?;
                member(project, &base, Namespace::Function, name)
            }
            _ => None,
        },
        Operand::Cast(type_) => Some(Typed {
            type_: type_.clone(),
            static_array: false,
            class,
        }),
        Operand::Unknown => None,
    }
}

/// The type of what a declaration declares, if it has one: a variable's, or
/// what a function returns.
fn declared(project: &Project, lookup: Lookup) -> Option<Typed> {
    let declaration = lookup.found()?;
    typed(&project.symbol(declaration).kind, declaration.class)
}

fn typed(kind: &SymbolKind, class: ClassId) -> Option<Typed> {
    let (type_, static_array) = match kind {
        SymbolKind::Function { return_type, .. } => (return_type.clone()?, false),
        _ => (kind.type_()?.clone(), kind.array_size().is_some()),
    };
    Some(Typed {
        type_,
        static_array,
        class,
    })
}

/// A type that is named, as the type of a cast to it.
fn named_type(project: &Project, class: ClassId, name: &Identifier) -> Option<Typed> {
    let known = project.class_from(class, name).is_some()
        || project.resolve_type(class, name).found().is_some();
    known.then(|| Typed {
        type_: TypeRef::Named(name.clone()),
        static_array: false,
        class,
    })
}

/// The type of `name`, a member (or a function) of the type of `base`: a
/// class's, or a struct's.
fn member(
    project: &Project,
    base: &Typed,
    namespace: Namespace,
    name: &Identifier,
) -> Option<Typed> {
    if base.static_array {
        return None;
    }
    let TypeRef::Named(type_name) = &base.type_ else {
        return None;
    };
    if let Some(class) = project.class_from(base.class, type_name) {
        return declared(project, project.lookup(class, namespace, name));
    }
    if namespace != Namespace::Value {
        return None;
    }
    let struct_ = project.resolve_type(base.class, type_name).found()?;
    if !matches!(project.symbol(struct_).kind, SymbolKind::Struct) {
        return None;
    }
    let table = project.table(struct_.class);
    let scope = table
        .scopes()
        .find(|(_, scope)| scope.kind == ScopeKind::Struct(struct_.symbol))
        .expect("a struct has a scope")
        .0;
    let symbol = *table
        .scope(scope)
        .declarations(Namespace::Value, name)
        .first()?;
    typed(&table.symbol(symbol).kind, struct_.class)
}

/// The enum a type is, if it is one.
pub fn enum_of(project: &Project, type_: &Typed) -> Option<Declaration> {
    if type_.static_array {
        return None;
    }
    let TypeRef::Named(name) = &type_.type_ else {
        return None;
    };
    let declaration = project.resolve_type(type_.class, name).found()?;
    matches!(project.symbol(declaration).kind, SymbolKind::Enum).then_some(declaration)
}
