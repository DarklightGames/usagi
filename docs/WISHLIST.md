# Wishlist

Ideas that aren't implemented yet. For the lints that exist, see
[STATIC_ANALYSIS.md](STATIC_ANALYSIS.md).

## Static analysis

### Per file

* Non-exhaustive `switch` on an `enum` value, for enums declared in the file.
* `[]` instead of `()` as the array index in a `defaultproperties` target.
* Statements (`#exec`, `const`) before the class declaration. Parsing this
  is an accepted quirk (see [PERMISSIVE_PARSING.md](PERMISSIVE_PARSING.md)), so
  this would be a lint, not a grammar change.
* More than one `defaultproperties` block. UE2 accepts it, but the behaviour is
  unreliable.
* More than one `default:` in a `switch`. UE2 accepts it, and default cases
  anywhere in the switch.
* The wrong form of `return`: `return X;` in a function with no return type, or
  a bare `return;` in a function that has one.
* A `goto` to a label that isn't declared in the state.
* Flow-sensitive "value assigned is never read" (R010 counts any read anywhere).
* Unused parameters of `private` and functions.

### Needs a project-wide symbol table

See [PROJECT_ANALYSIS.md](PROJECT_ANALYSIS.md) for the design.

* Redundant `defaultproperties` values: a value that a subclass sets to the
  same value its parent already has (see below).
* A non-`Actor` `Object` that holds references to `Actor`s.
* Exhaustive `switch` checking for enums declared in other classes.
* A `config` variable in a class that isn't `config` anywhere in its hierarchy.
* `simulated` outside an `Actor` subclass.
* Argument types checked against function signatures, e.g. a string passed to
  `GotoState`.
* An override whose signature differs from the original.
* An unresolved class in `extends`, `class'X'` or `Spawn`.
* A class that uses one from a package later in `EditPackages`, and cycles
  between packages.
* `private` and `protected` access across classes.

### Redundant `defaultproperties` values: notes

Not started. A class's real default for a property is its own last
assignment, or else the nearest ancestor's, so the lint compares what a class
assigns with what it would inherit, which needs every class's assignments
across the hierarchy. Things to settle first:

* **Cost.** Time should be small: for each key a class assigns, walk up to the
  first ancestor that assigns it (a hash lookup per class, no merged copy of
  every class's defaults). Memory is the worry: a game's `defaultproperties`
  are large, so keeping every assignment's text in the project index would
  cost more than the symbol tables do now. Keep, per assignment, only a span
  and a hash of its normalized value (16 bytes or so), and compare the text on
  a match; or build the table of one hierarchy at a time, from a class down.
  Measure the index's memory before and after on a real tree.
* **Equality.** `1`, `1.0` and `1.00`, `True` and `true`, names and strings
  are different, struct values, `Foo(0)` and `Foo[0]` array elements, object
  references with and without their class, and what a value means for the
  property's type (an enum value or a byte). A value that can't be shown to be
  equal must count as different: a false positive suggests removing a line
  that changes behavior.
* **What isn't a plain assignment.** `Begin Object` blocks and their
  properties, a parent that sets the key in one and the child in another, and
  properties of structs and dynamic arrays.
* **Stock classes.** A parent that's only in a package has its defaults in the
  package's tagged properties, which `usagi-pkg` reads (see
  [ASSET_PIPELINE.md](ASSET_PIPELINE.md)), and the classes between have to be
  chained too; do the source-only case first.
* **Not always wrong.** A child may repeat a parent's value on purpose, so
  that a change in the parent doesn't change the child: it is opt-in, like the
  other lints that need the project (see [PROJECT_ANALYSIS.md](PROJECT_ANALYSIS.md)).
  A fix would remove the line, which is only safe without a comment on it.

### Needs a project-wide symbol table: commands

* Package dependencies beyond `extends` in `usagi graph`: what a class
  refers to (`class'X'`, struct and enum types, `Spawn`), once there's typing.
* `usagi refs`: who overrides, calls, extends or refers to a class or
  member.
* Check `EditPackages` order before running UCC, and rebuild only the
  packages a change affects.

## CLI

* Inline suppression comments, e.g. `// usagi: ignore[S008]`.
* Unsafe fixes for an editor to offer.
* SARIF output, for GitHub code scanning.

## Testing against the engine

* Check what UnrealScript actually does by running it in the engine. usagi
  would generate UnrealScript test classes (e.g. to pin down operator
  precedence, or whether a lint's fix changes behavior), then either run them
  with UCC itself or hand them to the user to run, and read the results back
  from the log.

## Packages

* Write a byte whose property is an enum as the enum's value
  (`LODSet=LODSET_Interface`), as UnrealEd does: the declaration now gives
  the enum.
* Fill in properties that aren't stored from the class's defaults, which are
  a chain: a class's defaults are only those that differ from its
  superclass's (see [ASSET_PIPELINE.md](ASSET_PIPELINE.md)).

### `pkg diff`

* Compare classes by what's in them: functions, states and variables added
  or removed, signatures and flags changed, and bytecode changed (or a
  disassembled diff of it).
* Read more native data rather than comparing its bytes, so that it isn't a
  change when only the rest of the package moved or its names were
  reordered: static meshes, skeletal meshes and animations.
* More properties whose saved value the engine never uses, in
  `crates/usagi-pkg/src/volatile.rs` (which has the texture modifiers' `M`),
  and fields of native data that are like them, marked in their types. Find
  them by saving a package again without changes and comparing, by counting
  which properties change across a repository's history, and by reading the
  engine's source: `Texture.InternalTime` is a candidate, but nothing in the
  source sets it.
* Compare two folders of packages, such as two releases' `System` folders,
  matching packages by name.
* Revision ranges, as git takes them: `abc123..def456`, and `abc123^!` for
  what one commit changed.
* Every package that changed between two revisions, when no path is given,
  or below a folder that is.
* git's external diff (`*.u diff=upkg` in `.gitattributes`), so that `git
  diff` and `git log -p` show packages' changes.
* Other version control systems, such as Perforce (`p4 print`) and SVN (`svn
  cat`).

## A compiler

* Compile UnrealScript to packages without UCC: see
  [COMPILER.md](COMPILER.md).

## A persistent runtime

* A usagi runtime that loads a game's packages (and, potentially, its source)
  into memory once, up front, and keeps indexes and caches: classes and their
  defaults, where each object is, who refers to what. Commands now read every
  file they need each time they run; exporting with class defaults and
  cross-package references (see [ASSET_PIPELINE.md](ASSET_PIPELINE.md)) would
  read many. The same runtime could serve an interactive mode, a TUI, and a
  language server.

## Build

* Clean up after an interrupted build: on Ctrl-C, stop UCC and remove the
  half-built packages from `System/` (the next build removes them now).
* Work when run from inside a mod folder, by finding the game directory above it.
* Run UCC on Windows at a lower priority too.

## Extended Language Specification (UnrealScriptPlus?)

I would also like to add more modern language features that can alleviate some
of the annoyances of UnrealScript. For example:

* Add ternary operators.
* Add inline `local` varable declaration & assignment.

This would require a transpiler step, but investigation needs to be done as to how
feasible this is given the odd environment that `UCC` operates in.

With usagi's own compiler ([COMPILER.md](COMPILER.md#extending-the-language)),
no transpiler is needed: new syntax compiles straight to the engine's
bytecode, as long as it can be expressed in it.
