//! The index of a project's classes: which classes there are, how they
//! extend each other, and what each declares. It chains the per-file
//! [`SymbolTable`]s, so that a name one class doesn't declare can be found in
//! its parents, or in another class. See `docs/PROJECT_ANALYSIS.md`.
//!
//! Names are case-insensitive, as in UnrealScript. A project is built once
//! ([`Project::build`]) and then only read, so it can be shared between
//! threads.
//!
//! What can't be found is unknown, not an error: a lookup answers
//! [`Lookup::NotFound`] only when the whole chain of parents, up to a class
//! with none, is in the project. If a parent is missing (its source isn't
//! part of the project), or the classes extend each other in a circle, the
//! answer is [`Lookup::Unknown`], so that a lint built on it says nothing
//! rather than guess.

use crate::ast::Identifier;
use crate::symbols::{Access, Namespace, ScopeId, ScopeKind, SymbolId, SymbolKind, SymbolTable};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Identifies a class in its [`Project`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClassId(usize);

impl ClassId {
    pub fn index(self) -> usize {
        self.0
    }
}

/// A name with its case folded and interned: comparing and hashing it is
/// comparing and hashing an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Name(u32);

/// Folds a name's case, as UnrealScript compares names (ASCII only).
fn fold(name: &str) -> Cow<'_, str> {
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(name.to_ascii_lowercase())
    } else {
        Cow::Borrowed(name)
    }
}

#[derive(Debug, Default)]
struct Interner {
    names: HashMap<Box<str>, Name>,
}

impl Interner {
    fn intern(&mut self, name: &str) -> Name {
        let folded = fold(name);
        if let Some(&interned) = self.names.get(folded.as_ref()) {
            return interned;
        }
        let interned = Name(self.names.len() as u32);
        self.names.insert(folded.as_ref().into(), interned);
        interned
    }

    /// The name, if it's been interned: a name that hasn't is declared
    /// nowhere in the project.
    fn get(&self, name: &str) -> Option<Name> {
        self.names.get(fold(name).as_ref()).copied()
    }
}

/// Where a class's source came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    /// Which source root: a lower one takes precedence, so a mod's class is
    /// before, and replaces, the game's class of the same name.
    pub root: usize,
    /// The package the class is compiled into, if known. Classes of the same
    /// name in different packages are different classes, told apart by
    /// writing `Package.Class`.
    pub package: Option<String>,
    pub path: PathBuf,
}

/// What a class extends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parent {
    /// It extends nothing (`Object`).
    Root,
    Class(ClassId),
    /// It extends a class that isn't in the project.
    Missing(Identifier),
    /// It's one of classes that extend each other in a circle. Its link is
    /// cut, so that its chain ends here.
    Cycle,
}

/// A problem with the classes as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Classes that extend each other in a circle, starting at the one that
    /// was added first.
    Cycle(Vec<ClassId>),
    /// Two files declare a class of the same name in the same package (or in
    /// none that's known). `kept` is the one in the project: the one in the
    /// lower source root, or the first by path.
    DuplicateClass {
        name: Identifier,
        kept: Origin,
        dropped: Origin,
    },
    /// Classes of the same name in different packages: both are in the
    /// project, but a bare name means `used`, the one in the lower source
    /// root, or the first by path, except from the same package.
    AmbiguousClass {
        name: Identifier,
        used: Origin,
        also: Origin,
    },
}

/// A declaration found by a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declaration {
    /// The class that declares it.
    pub class: ClassId,
    pub symbol: SymbolId,
}

/// The answer to a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    Found(Declaration),
    /// There's no such declaration in the class or its parents, all of which
    /// are in the project.
    NotFound,
    /// There may be: part of the chain of parents isn't in the project.
    Unknown,
}

impl Lookup {
    pub fn found(self) -> Option<Declaration> {
        match self {
            Lookup::Found(declaration) => Some(declaration),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct State {
    symbol: SymbolId,
    extends: Option<Name>,
    functions: HashMap<Name, SymbolId>,
}

#[derive(Debug)]
struct Class {
    origin: Origin,
    table: SymbolTable,
    parent: Parent,
    /// Whether the chain of parents ends in a class that extends nothing.
    complete: bool,
    children: Vec<ClassId>,
    /// What the class scope declares, by the first declaration of each name.
    members: HashMap<(Namespace, Name), SymbolId>,
    states: HashMap<Name, State>,
}

/// The classes of a project. See the module docs.
#[derive(Debug)]
pub struct Project {
    names: Interner,
    classes: Vec<Class>,
    /// The classes of each name, in the order of their origins.
    by_name: HashMap<Name, Vec<ClassId>>,
    problems: Vec<Problem>,
}

impl Project {
    /// Indexes `files`: each file's origin and symbol table. A file that
    /// declares no class is left out. The result doesn't depend on the order
    /// of `files`.
    pub fn build(mut files: Vec<(Origin, SymbolTable)>) -> Project {
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut names = Interner::default();
        let mut classes: Vec<Class> = Vec::new();
        let mut by_name: HashMap<Name, Vec<ClassId>> = HashMap::new();
        let mut problems = Vec::new();

        for (origin, table) in files {
            let Some(info) = table.class() else { continue };
            let name = names.intern(&info.name);
            let same = by_name.get(&name).map(Vec::as_slice).unwrap_or_default();
            if let Some(&kept) = same
                .iter()
                .find(|c| classes[c.0].origin.package == origin.package)
            {
                problems.push(Problem::DuplicateClass {
                    name: info.name.clone(),
                    kept: classes[kept.0].origin.clone(),
                    dropped: origin,
                });
                continue;
            }
            if let Some(&used) = same.first() {
                problems.push(Problem::AmbiguousClass {
                    name: info.name.clone(),
                    used: classes[used.0].origin.clone(),
                    also: origin.clone(),
                });
            }
            by_name
                .entry(name)
                .or_default()
                .push(ClassId(classes.len()));
            let (members, states) = declarations(&table, &mut names);
            classes.push(Class {
                origin,
                table,
                parent: Parent::Root,
                complete: false,
                children: Vec::new(),
                members,
                states,
            });
        }

        for id in 0..classes.len() {
            let parent = match classes[id].table.class().and_then(|c| c.parent.clone()) {
                None => Parent::Root,
                Some(parent) => {
                    let package = classes[id].origin.package.as_deref();
                    match find_class(&names, &by_name, &classes, &parent, package) {
                        Some(parent) => Parent::Class(parent),
                        None => Parent::Missing(parent),
                    }
                }
            };
            classes[id].parent = parent;
        }
        problems.extend(cut_cycles(&mut classes));
        for id in 0..classes.len() {
            if let Parent::Class(parent) = classes[id].parent {
                classes[parent.0].children.push(ClassId(id));
            }
        }
        mark_complete(&mut classes);

        Project {
            names,
            classes,
            by_name,
            problems,
        }
    }

    /// The number of classes.
    pub fn len(&self) -> usize {
        self.classes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// Every class, in the order of their origins.
    pub fn classes(&self) -> impl Iterator<Item = ClassId> + use<> {
        (0..self.classes.len()).map(ClassId)
    }

    /// Cycles and duplicate classes.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// The class named `name` (in any case): `Class`, or `Package.Class`. If
    /// classes in several packages are named `Class`, the first (see
    /// [`Problem::AmbiguousClass`]).
    pub fn class(&self, name: &str) -> Option<ClassId> {
        find_class(&self.names, &self.by_name, &self.classes, name, None)
    }

    /// The class `name` means when written in `from`: `Class`, or
    /// `Package.Class`. A bare name means the class of that name in the
    /// package of `from`, if there is one.
    pub fn class_from(&self, from: ClassId, name: &str) -> Option<ClassId> {
        let package = self.classes[from.0].origin.package.as_deref();
        find_class(&self.names, &self.by_name, &self.classes, name, package)
    }

    /// The class's name, as it's declared.
    pub fn name(&self, class: ClassId) -> &str {
        let info = self.classes[class.0].table.class();
        &info.expect("an indexed class has one").name
    }

    pub fn origin(&self, class: ClassId) -> &Origin {
        &self.classes[class.0].origin
    }

    pub fn table(&self, class: ClassId) -> &SymbolTable {
        &self.classes[class.0].table
    }

    pub fn parent(&self, class: ClassId) -> &Parent {
        &self.classes[class.0].parent
    }

    /// Whether every class from `class` up to the one that extends nothing is
    /// in the project. Only then is a name that isn't found really missing.
    pub fn is_complete(&self, class: ClassId) -> bool {
        self.classes[class.0].complete
    }

    /// The classes that extend `class`.
    pub fn children(&self, class: ClassId) -> &[ClassId] {
        &self.classes[class.0].children
    }

    /// `class`, its parent, and so on up, as far as the project has them.
    pub fn chain(&self, class: ClassId) -> impl Iterator<Item = ClassId> + '_ {
        std::iter::successors(Some(class), |&c| match self.classes[c.0].parent {
            Parent::Class(parent) => Some(parent),
            _ => None,
        })
    }

    /// Whether `class` is `ancestor` or extends it, however far.
    pub fn is_subclass(&self, class: ClassId, ancestor: ClassId) -> bool {
        self.chain(class).any(|c| c == ancestor)
    }

    /// The symbol a declaration is.
    pub fn symbol(&self, declaration: Declaration) -> &crate::symbols::Symbol {
        self.classes[declaration.class.0]
            .table
            .symbol(declaration.symbol)
    }

    /// The nearest declaration of `name` in `namespace`, in `class` or the
    /// classes it extends. A parent's private member is found too: see
    /// [`Project::is_accessible`].
    pub fn lookup(&self, class: ClassId, namespace: Namespace, name: &str) -> Lookup {
        let Some(name) = self.names.get(name) else {
            return self.missing(class);
        };
        self.lookup_name(class, namespace, name)
    }

    fn lookup_name(&self, class: ClassId, namespace: Namespace, name: Name) -> Lookup {
        for c in self.chain(class) {
            if let Some(&symbol) = self.classes[c.0].members.get(&(namespace, name)) {
                return Lookup::Found(Declaration { class: c, symbol });
            }
        }
        self.missing(class)
    }

    /// What a failed lookup in `class` is.
    fn missing(&self, class: ClassId) -> Lookup {
        if self.classes[class.0].complete {
            Lookup::NotFound
        } else {
            Lookup::Unknown
        }
    }

    /// The function `name`, called from the code of `state` in `class`
    /// (a state's own functions replace the class's), looking in the state,
    /// the state it `extends`, and then the class.
    pub fn lookup_function_in_state(&self, class: ClassId, state: &str, name: &str) -> Lookup {
        let Some(function) = self.names.get(name) else {
            return self.missing(class);
        };
        let mut next = self.names.get(state);
        let mut seen = HashSet::new();
        while let Some(state) = next.filter(|s| seen.insert(*s)) {
            next = None;
            for c in self.chain(class) {
                let Some(found) = self.classes[c.0].states.get(&state) else {
                    continue;
                };
                if let Some(&symbol) = found.functions.get(&function) {
                    return Lookup::Found(Declaration { class: c, symbol });
                }
                next = next.or(found.extends);
            }
        }
        self.lookup_name(class, Namespace::Function, function)
    }

    /// The struct or enum `name`, as written in `class`: a name (the
    /// nearest declaration in the class and its parents), or `Class.Name`
    /// (declared in that class or its parents).
    pub fn resolve_type(&self, class: ClassId, name: &str) -> Lookup {
        match name.rsplit_once('.') {
            None => self.lookup(class, Namespace::Type, name),
            Some((owner, name)) => match self.class_from(class, owner) {
                Some(owner) => self.lookup(owner, Namespace::Type, name),
                None => Lookup::Unknown,
            },
        }
    }

    /// The values of an enum, in order.
    pub fn enum_values(&self, enum_: Declaration) -> Vec<SymbolId> {
        let table = &self.classes[enum_.class.0].table;
        let mut values: Vec<(usize, SymbolId)> = table
            .iter()
            .filter_map(|(id, symbol)| match symbol.kind {
                SymbolKind::EnumValue { enum_: e, index } if e == enum_.symbol => Some((index, id)),
                _ => None,
            })
            .collect();
        values.sort();
        values.into_iter().map(|(_, id)| id).collect()
    }

    /// The function `function` overrides: the nearest declaration of its
    /// name in the classes it extends. Only functions of the class itself,
    /// not of its states.
    pub fn overridden(&self, function: Declaration) -> Option<Declaration> {
        let name = self.declared_name(function);
        let parent = match self.classes[function.class.0].parent {
            Parent::Class(parent) => parent,
            _ => return None,
        };
        self.lookup_name(parent, Namespace::Function, name).found()
    }

    /// The functions that override `function`: those of its class's
    /// descendants, nearest first.
    pub fn overriders(&self, function: Declaration) -> Vec<Declaration> {
        let name = self.declared_name(function);
        let mut found = Vec::new();
        let mut queue: std::collections::VecDeque<ClassId> =
            self.children(function.class).iter().copied().collect();
        while let Some(class) = queue.pop_front() {
            queue.extend(self.children(class));
            let key = (Namespace::Function, name);
            if let Some(&symbol) = self.classes[class.0].members.get(&key) {
                found.push(Declaration { class, symbol });
            }
        }
        found
    }

    /// The interned name of a declaration, which every one has been.
    fn declared_name(&self, declaration: Declaration) -> Name {
        self.names
            .get(&self.symbol(declaration).name)
            .expect("a declared name is interned")
    }

    /// Whether code in `from` may use `declaration`: a private member is for
    /// its own class, a protected one for it and its subclasses.
    pub fn is_accessible(&self, from: ClassId, declaration: Declaration) -> bool {
        match self.symbol(declaration).access {
            Access::Public => true,
            Access::Protected => self.is_subclass(from, declaration.class),
            Access::Private => from == declaration.class,
        }
    }
}

/// The class a name written in a class of package `near` means.
fn find_class(
    names: &Interner,
    by_name: &HashMap<Name, Vec<ClassId>>,
    classes: &[Class],
    written: &str,
    near: Option<&str>,
) -> Option<ClassId> {
    let (package, bare) = match written.split_once('.') {
        Some((package, bare)) => (Some(package), bare),
        None => (None, written),
    };
    let candidates = by_name.get(&names.get(bare)?)?;
    let in_package = |wanted: &str| {
        candidates.iter().copied().find(|c| {
            let have = classes[c.0].origin.package.as_deref();
            have.is_some_and(|have| have.eq_ignore_ascii_case(wanted))
        })
    };
    match package {
        // A package that isn't known is taken to be whatever it needs to be.
        Some(package) => in_package(package).or_else(|| {
            let unknown = |c: &&ClassId| classes[c.0].origin.package.is_none();
            candidates.iter().find(unknown).copied()
        }),
        None => near
            .and_then(in_package)
            .or_else(|| candidates.first().copied()),
    }
}

type Members = HashMap<(Namespace, Name), SymbolId>;

/// The names a class declares: its members, and its states' functions.
fn declarations(table: &SymbolTable, names: &mut Interner) -> (Members, HashMap<Name, State>) {
    let mut members = Members::new();
    let mut states = HashMap::new();
    for &id in table.scope(ScopeId::CLASS).symbols() {
        let symbol = table.symbol(id);
        let name = names.intern(&symbol.name);
        members.entry((symbol.kind.namespace(), name)).or_insert(id);
        if let SymbolKind::State { extends } = &symbol.kind {
            let extends = extends.as_ref().map(|e| names.intern(e));
            states.entry(name).or_insert_with(|| State {
                symbol: id,
                extends,
                functions: HashMap::new(),
            });
        }
    }
    for (_, scope) in table.scopes() {
        let ScopeKind::State(state) = scope.kind else {
            continue;
        };
        let state_name = names.intern(&table.symbol(state).name);
        if states.get(&state_name).is_none_or(|s| s.symbol != state) {
            continue; // a repeated declaration of the state
        }
        for &id in scope.symbols() {
            let symbol = table.symbol(id);
            if matches!(symbol.kind, SymbolKind::Function { .. }) {
                let name = names.intern(&symbol.name);
                states
                    .get_mut(&state_name)
                    .expect("checked above")
                    .functions
                    .entry(name)
                    .or_insert(id);
            }
        }
    }
    (members, states)
}

/// Finds classes that extend each other in a circle, and cuts the link that
/// closes each, so that the classes form a forest.
fn cut_cycles(classes: &mut [Class]) -> Vec<Problem> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Walking,
        Done,
    }
    let mut marks = vec![Mark::New; classes.len()];
    let mut problems = Vec::new();
    for start in 0..classes.len() {
        let mut path = Vec::new();
        let mut at = Some(start);
        while let Some(id) = at {
            match marks[id] {
                Mark::Done => break,
                Mark::Walking => {
                    let from = path.iter().position(|&p| p == id).expect("on the path");
                    let mut cycle: Vec<usize> = path[from..].to_vec();
                    let first = cycle
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, id)| **id)
                        .map(|(i, _)| i)
                        .expect("a cycle has classes");
                    cycle.rotate_left(first);
                    for &c in &cycle {
                        classes[c].parent = Parent::Cycle;
                    }
                    problems.push(Problem::Cycle(cycle.into_iter().map(ClassId).collect()));
                    break;
                }
                Mark::New => {
                    marks[id] = Mark::Walking;
                    path.push(id);
                    at = match classes[id].parent {
                        Parent::Class(parent) => Some(parent.0),
                        _ => None,
                    };
                }
            }
        }
        for id in path {
            marks[id] = Mark::Done;
        }
    }
    problems
}

/// Sets `complete` for every class, once the cycles are cut.
fn mark_complete(classes: &mut [Class]) {
    let mut known: Vec<Option<bool>> = vec![None; classes.len()];
    for start in 0..classes.len() {
        let mut path = Vec::new();
        let mut at = start;
        let complete = loop {
            if let Some(complete) = known[at] {
                break complete;
            }
            path.push(at);
            match classes[at].parent {
                Parent::Root => break true,
                Parent::Class(parent) => at = parent.0,
                Parent::Missing(_) | Parent::Cycle => break false,
            }
        };
        for id in path {
            known[id] = Some(complete);
            classes[id].complete = complete;
        }
    }
}
