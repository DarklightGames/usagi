# Project-wide analysis (design)

A design for analysis that looks across classes: parent hierarchies, types
and members declared in other classes, and dependencies between classes and
packages. It is the "needs a project-wide symbol table" part of
[WISHLIST.md](WISHLIST.md). See [Status](#status) for how much exists.

Today every file is analysed alone. The [`SymbolTable`] covers one class, and
a name it can't resolve (an inherited variable, another class's enum) is
unknown, not an error, so no lint fires on it. `ClassInfo::parent` is
was recorded but unused until now. This design adds a **project index** that chains the
per-file tables, so that unknown names can become known.

## Status

Done (steps 0 to 4, the graph of 6 and the first typing of 7, of [Order of work](#order-of-work)):

- `usagi_pkg::Packages` is `Send + Sync` (`Arc` and locks in place of `Rc` and
  `RefCell`); threads that race to read a package get the same `Arc`.
- `usagi_core::project`: the index (`Project`) and the resolver, with 100%
  coverage from `tests/project.rs`.
- `usagi lint --statistics` builds the index and prints a summary (see
  [CLI.md](CLI.md#the-projects-classes)); `--no-project` skips it.
- The first lint that uses it: C017 `undeclared-default-property` (step 4b),
  opt-in (see [Lints](#lints-this-allows)). It runs from `usagi_core::
  project_lint`, on data the symbol table kept while it was built: the
  `defaultproperties` keys it couldn't resolve in the class. The lint needs
  neither the AST nor a second parse, which is why the visitor doesn't take a
  `Project`: a project lint is a function of the index. A lint that needs more
  than the table keeps will have the table keep it, the same way.

- S019 `could-be-static`, from the names function bodies use that the file
  doesn't declare (`SymbolTable::unresolved_uses`) and the instance members
  they use that it does (`SymbolTable::instance_use`).
- Minimal typing, and the lint that needed it: S018 `non-exhaustive-switch`,
  opt-in (steps 4d and 7, in part). The symbol table records each `switch`
  (its subject and `case` labels as `Operand`s, names that can be looked up
  once the project is known, and whether it has a `default`), and function
  return types. `usagi_core::typing::type_of` types a subject through the
  project: names (locals, arguments, a class's or a parent's variables),
  `Self`, members of classes and structs, array elements, what a function
  returns, and casts. Everything else has no type, and a lint says nothing.
- `usagi graph` (step 6, brought forward): the class hierarchy and the
  package dependencies (an extends from one package to another), as a tree,
  Graphviz DOT, a Mermaid flowchart or JSON, with `--class`, `--ancestors`,
  `--descendants` and `--depth` to cut a readable part out of thousands of
  classes (see [CLI.md](CLI.md#usagi-graph-paths)).

Not yet: other project lints, packages as a source of declarations (step 5),
`graph` and the dependency lints (step 6), and typing (step 7).

On a real tree (Red Orchestra and Darkest Hour: 7,551 classes, 12 cores):

| | wall time | peak memory |
|---|---|---|
| `lint`, before | 1.4 s | 82 MB |
| `lint --statistics` (indexing) | 1.4 s | 147 MB |

Every class's chain of parents resolves. The index's cost is memory (the
tables of every file are kept), not time: building it is small next to
parsing, which already runs in parallel.

## Goals

- Resolve names through the class hierarchy and across classes.
- Keep the guarantee that what can't be resolved stays unknown: a missing
  parent or an unreadable package must never produce a false positive.
- Keep single-file use (`parse`, `format`, `lint` on one file, the tests)
  exactly as it is: project information is optional.
- Share the declaration model with the compiler ([COMPILER.md](COMPILER.md),
  part 3 and 4), which needs the same lookups.

Not goals: a full type checker. Expression typing is added only as far as a
lint needs it (see [Typing](#typing)).

## The parts

### 1. The project index

A `Project` in `usagi-core`, built from many parsed files:

- Classes by name, **case-insensitively** (UnrealScript is). Each has its
  `SymbolTable`, its package (when known, from `<Package>/Classes/<Class>.uc`)
  and where it came from.
- The parent of each class, with the chain resolved lazily. A chain ends in
  one of: reaches `Object`; **missing parent** (unknown, see below); or
  **cycle** (reported once, then treated as missing).
- The children of each class, for subclass queries.
- A per-class declaration index: enums, structs, consts, variables,
  functions, states. The table already has these; the index only has to find
  them by class.
- Packages: classes of one name in different packages are different classes,
  told apart by writing `Package.Class` (which `extends` does, e.g.
  `extends DH_Weapons.DH_Kz8cmGrW42Weapon`). Both are in the index; a bare name
  means the one in the same package as where it's written, else the first (by
  source root, then path), and the ambiguity is reported
  (`Problem::AmbiguousClass`).
- Duplicates: two files that declare a class of one name in one package: the
  one from the higher-precedence source root wins (a mod's class overrides the
  game's), and the other isn't indexed. It's reported as a note, not an
  error (`Problem::DuplicateClass`).

It stores tables, not ASTs, to keep memory down on a large tree.

### 2. The resolver

Queries over the index, each returning `Known(decl)`, `Unknown` or
`NotFound`. Only `NotFound` may be reported, and only when the whole chain
to `Object` was known.

- `lookup(class, name, namespace)`: walks the parents. Handles shadowing,
  access (`private` is the declaring class only, `protected` its
  subclasses), and states (a state's function overrides the class's).
- `resolve_type(from, name)`: a bare name (this class, then its parents) or
  `Class.Name`. Not yet: the scope of a `within` class's outer.
- Enum values: `EFoo.Bar` and a bare `Bar` in the hierarchy.
- `is_subclass(a, b)`, `overridden(function)` and `overriders(function)`,
  for the functions of a class (not of its states).
- `lookup_function_in_state(class, state, name)`: a state's own function, then
  those of the state it `extends`, then the class's.
- `is_accessible(from, declaration)`: `private` and `protected`.

### 3. Declarations from packages

A mod's source tree usually lacks `Core` and `Engine`. Their declarations
come from the compiled `.u` files through `usagi_pkg::Packages` (which
already reads class, enum, struct and property declarations; see
[PACKAGE_FORMAT.md](PACKAGE_FORMAT.md)).

`usagi-core` mustn't depend on `usagi-pkg`, so core defines a trait:

```rust
trait DeclarationSource {
    fn class(&self, name: &str) -> Option<ClassDecls>;
}
```

`ClassDecls` is the same declaration index shape the source tables produce.
The CLI implements the trait over `Packages`. The resolver asks sources in
order (the project's own files, then packages), so a class has the same
shape wherever it came from. This is the piece the compiler shares.

Unreadable or absent packages make their classes missing, not wrong.

### 4. Plumbing

- Project lints don't go through the visitor. Each is a function of the
  [`Project`] (`project_lint::check`), over what the table kept, so a lint
  that needs the project doesn't re-parse the file. They're opt-in: only a
  selector that names one selects it (`Lint::needs_project`). `lint::`
  applies the file's suppression comments, which only a selector naming the
  lint suppresses, and parsing doesn't report such a selector as unused.
- `usagi lint` keeps the symbol table of each file it checks, and builds the
  index afterwards (done, behind `--statistics` until a lint needs it). The
  index covers every file under the paths, also those `include` and `exclude`
  keep out of the lint (read only for their classes), since a linted class
  extends them. A lint
  that needs the project can't run in that first pass; it runs on the index
  after it. `--no-project` keeps the old behaviour. It also needs a way to say where
  the game's packages are (the `Paths` in `pkg/game.rs`) and the order of
  source roots.
- Suppression comments, `--fix` and `--diff` are unchanged.
- Files outside the index (a single file given on the command line) are
  analysed against whatever the index can find, or alone with
  `--no-project`.

### Typing

Switch exhaustion needs the type of the subject, which is a local, a
parameter, a member (possibly inherited), a function's return value, or a
member access. A minimal `type_of(expr)` covers exactly those and returns
`Unknown` for anything else (operators, casts it doesn't understand, array
methods). Each lint that needs more adds a case, with a test; the full
checker is part 4 of [COMPILER.md](COMPILER.md#4-semantic-analysis).

## Lints this allows

Added to [STATIC_ANALYSIS.md](STATIC_ANALYSIS.md) with each, as usual.

First step, hierarchy and enums:

- Non-exhaustive `switch` on an enum, for enums declared in any class in
  the hierarchy (S018; done).
- A `defaultproperties` key not declared anywhere in the hierarchy (C017;
  done). A delegate is declared as a function, so it counts.
- An override with a different signature from the one it overrides.
- A redundant `defaultproperties` value equal to the parent's.
- A `config` variable that isn't `config` anywhere in its hierarchy.
- `simulated` outside an `Actor` subclass.
- An unresolved class: `extends`, `class'X'`, `Spawn(class'X')`, state
  `extends`.

Later, with more typing:

- Argument types against function signatures (a string given to
  `GotoState`).
- A non-`Actor` `Object` holding `Actor` references.
- `private` and `protected` access across classes.
- A missing `Super.X()` in overrides of functions that need one (an
  opt-in list in the configuration).

Dependencies:

- A class using a class from a package later in `EditPackages` (UCC fails
  on this; usagi would say so with the file and line).
- Cycles between packages.
- Classes nothing refers to or extends (opt-in; the engine finds classes
  by name, so this is a hint).

## Commands this allows

- `usagi graph` (done): the class hierarchy or the package dependencies, as a
  tree, DOT, Mermaid or JSON, optionally from one class (its ancestors,
  descendants, or both) and with a depth. Package dependencies are `extends`
  only so far: what a class refers to (`class'X'`, types) comes with typing.
- `usagi refs <Class[.Member]>`: who overrides, calls, extends or refers to
  it. The base for a safe rename.
- `usagi build`: use the dependency order to check `EditPackages` before
  running UCC, and later to rebuild only the packages a change affects.
- Everything a language server needs for go-to-definition, hover and
  find-references; see "A persistent runtime" in
  [WISHLIST.md](WISHLIST.md#a-persistent-runtime).

## Performance

A game's whole source tree, plus the stock packages, is a lot to index on
every run. The design is meant to stay cheap by these rules.

**Layering.** A file's `SymbolTable` depends only on that file's text.
Cross-class information lives in the `Project`, which points at tables and
never copies into them. If a per-file result ever starts to depend on
another file, parallelism and caching (below) stop working, so this is a
rule, not a preference.

**First version** (plain, per run, nothing persistent). Built so far: the
`Arc` and locks in `Packages`, interned names, the flat index and lazy chain
walks. Member lookups aren't memoized yet: a lookup is a hash lookup per
class of the chain, and nothing is slow enough to measure, so the memo waits
for a lint that profiles badly (a failed lookup walks the whole chain, which
is the case to cache). Table building is in parallel already (the CLI checks
files on a thread pool).

- Parse files and build tables in parallel (`rayon`, as `usagi lint` already
  does); only merging into the index is serial. Linting after the index exists is read-only, so it is
  parallel too. That needs `Project` to be `Sync`; `usagi_pkg::Packages`
  uses `Rc`, so the packages implementation of `DeclarationSource` extracts
  its `ClassDecls` up front (or `Packages` moves to `Arc`).
- Intern names: a case-folded `Name(u32)` made once, so lookups hash
  integers, not fold case on every comparison, and the thousands of
  `Actor`s and `Location`s share one string.
- A flat index (class name to `ClassId`, and per class its declarations),
  with IDs into `Vec`s as `SymbolId` already is, not pointer graphs.
- Parent chains are walked on demand, not resolved for every class.
  Hierarchies are shallow, so the walk is cheap; what is memoized is the
  result: `(ClassId, Name, Namespace)` to where the member was found.
  **Negative results are cached too** (`NotFound`, `Unknown`): a failed
  lookup walks the whole chain, and it is the common case.
- Declarations are kept apart from bodies. Cross-class queries read only
  the small declaration index; function bodies and local scopes stay in the
  per-file table, which only that file's lints touch. Tables of files that
  aren't being linted can be dropped after the index is built.
- Skip work: open a `.u` package only when a lookup falls through to it;
  when linting part of a tree, find a parent's file by following `extends`
  through the source roots, rather than indexing everything; and have a
  lint that needs the project check cheaply that it can fire (the file has
  a `switch`) before asking the resolver.

**Keep queries pure.** Each resolver query is a function of explicit inputs,
with no hidden global state. A batch run memoizes in plain per-run maps with
no invalidation. If usagi gets a long-lived runtime or language server (see
[WISHLIST.md](WISHLIST.md#a-persistent-runtime)), the same queries can move
to a demand-driven query layer in the style of Salsa, where changing a file
invalidates only what read it. What makes that pay off is early cutoff: an
edit that leaves the file's declaration index equal (a function body
changed) invalidates nothing downstream, so the declaration index must be
small and comparable (`Eq`).

**Later: a persistent cache** (not in the first version):

- Per-file declaration indexes, keyed by path, a content hash (with size
  and mtime as a quick check, never mtime alone), usagi's version and a
  cache format version. A run re-parses only changed files.
- Per package, the extracted `ClassDecls`, keyed by the package's size and
  GUID and generations (see [PACKAGE_FORMAT.md](PACKAGE_FORMAT.md)). Reading
  declarations out of a `.u` costs far more than parsing source, so this
  helps most. `Package::map` avoids reading whole files for a few tables.
- A cache never changes results, only speed: any read error, version
  mismatch or missing entry means rebuild. The directory is safe to delete,
  and a corpus check compares cached and uncached lint output byte for byte.

**Measure first.** Before any of this, record wall time and peak memory of
`usagi lint` over the corpus (step 0 below), and again after each step.
Profile (`cargo flamegraph`, `samply`) before optimizing: the cost is
usually string folding, allocation or re-reading files, not the algorithm.
Corpus timings are too noisy on shared CI runners to gate on.

## Order of work

0. Baseline: wall time and peak memory of `usagi lint` and `usagi parse`
   over the corpus, before any of this. *Done.*
1. The index and the parent chain, with a corpus check: every class's chain
   resolves, or ends at a known missing parent. Report cycles and
   duplicates. *Done.*
2. `lookup` through the hierarchy, and `resolve_type`. *Done.*
3. `Project` plumbing in `lint`, with `--no-project`, interning names.
   *Done. Project lints don't go through the visitor (see above).*
4. The first lints, each run on the corpus with its hits read through, as for
   any lint: undeclared `defaultproperties` key (*done*, C017); then
   `usagi graph` (step 6, brought forward; *done*); then, with the minimal typing
   it needs, non-exhaustive `switch` (*done*, S018).
5. `DeclarationSource` and the packages implementation; then the stock
   classes resolve and the "chain ends in a missing parent" cases shrink.
6. `usagi graph` (*done*), then the dependency lints.
7. Minimal typing (*done*: what S018 needs), then the lints that need more:
   argument types against signatures, operators and literals, `private` and
   `protected` access, and the `Actor`-reference lint.

## Risks

- **Case.** Names compare case-insensitively everywhere, but are printed as
  written.
- **Missing source.** Without the engine's source or packages, most chains
  are incomplete; step 5 matters more than it looks. Lints must say
  nothing, not guess.
- **Memory and time.** A game's whole tree in memory; measure on the
  corpus after step 1 before building on it (see
  [Performance](#performance)).
- **Precedence.** A mod's classes override the game's by name. The order of
  roots must be explicit in the configuration, or the result depends on
  how files were found.
- **Packages versus source.** A package's declarations can differ from the
  source it was built from (the source may have changed since). Prefer
  source when both exist, and say which was used.
- **Coverage.** `visitor.rs` and `parser.rs` keep 100% coverage, so each
  new branch needs a multi-file test fixture; add a small helper to
  `tests/lints.rs` for them first.
