# Static analysis

usagi checks UnrealScript code for likely bugs, redundant code and
style problems. Every problem found by a lint carries the lint's **code** and
**name** (e.g. `S003 self-assignment`) on `ProgramError::lint`, alongside a
severity (`Error` or `Warning`).

The registry of lints is in [`src/lint.rs`](../crates/usagi-core/src/lint.rs). The checks are in
[`src/visitor.rs`](../crates/usagi-core/src/visitor.rs), and some of them use the per-file symbol
table in [`src/symbols.rs`](../crates/usagi-core/src/symbols.rs).

## Categories

A lint code is its category's letter followed by three digits.

| Prefix | Category | What it finds |
|---|---|---|
| `C` | correctness | Code that is wrong: it won't compile, or won't do what it says. |
| `S` | suspicious | Code that is very likely a mistake. |
| `P` | performance | Code that is needlessly slow. |
| `R` | redundancy | Code that does nothing and can be removed. |
| `N` | naming | Naming conventions. |
| `D` | documentation | How code is documented. |

## Choosing lints

Every lint is reported by default. `LintSelection` chooses which to keep, like
Ruff's `select` and `ignore`. A selector is `ALL`, a code or code prefix (`S`,
`S00`, `S003`), a lint name (`self-assignment`) or a category name
(`suspicious`), all case-insensitive. A lint is reported if a `select` selector
matches it and no `ignore` selector does.

```rust
let mut result = usagi_core::parse_program(source)?;
LintSelection::new(["C", "S"], ["S008"]).retain(&mut result.errors);
```

`usagi lint` accepts the same selectors, on the command line or in a
`usagi.toml` (see [CLI.md](CLI.md)):

```sh
usagi lint --select C,S --ignore S008 Foo.uc
usagi lint --format tsv --statistics <root> > lints.tsv
```

`--statistics` prints the number of problems and files for each lint.

### Lints that look at other classes

A lint that needs the classes of the whole project (see
[PROJECT_ANALYSIS.md](PROJECT_ANALYSIS.md)) is **opt-in**: only a selector
that names it, by its code or its name, selects it. `ALL`, a code prefix and a
category don't. They're marked "needs the project" in the list below.

```sh
usagi lint --extend-select C017 <root>
```

They run once every class under the paths is indexed, including the files
that `include` and `exclude` skip, and `--no-project` turns them off. A
lint reports a name only when the class's whole chain of parents is in the
project: a class that extends one that isn't is never reported for what it
may inherit. In the library, [`usagi_core::project_lint`] runs them on a
[`Project`](usagi_core::Project); `parse_program` doesn't.

## Suppression comments

A comment that starts with `usagi:` suppresses lints in the code itself:

```unrealscript
A = A; // usagi: ignore[S003] the setter has side effects

// usagi: ignore[switch-case-fallthrough, R001]
case 1: X = 0;

// usagi: ignore-file[naming]
```

- `usagi: ignore[...]` suppresses the listed lints on one line: the
  comment's own line if code shares it, or the next line if the comment is
  alone on its line. A block comment (`/* usagi: ignore[...] */`) works the
  same way. A problem is on the line where it starts.
- `usagi: ignore-file[...]` suppresses them in the whole file, wherever the
  comment is.
- Between the brackets is a comma-separated list of lint selectors, as for
  `LintSelection` (`S003`, `S`, `self-assignment`, `suspicious`, `ALL`).
  Anything after the `]` is ignored, so it can give the reason.

The comment must start with `usagi:` (after the `//` or `/*` and any spaces),
so `// see usagi: ignore[S003]` is an ordinary comment. Stacked comments
don't combine: the line after `// usagi: ignore[S003]` is the next comment,
so list every lint in one comment instead.

Suppression happens in `parse_program_with`, before `LintSelection`, so a
suppressed problem is not in `ProgramResult::errors` and its fix is never
applied. Problems that aren't lints, and R016 and S017 themselves, can't be
suppressed (use `ignore` or `[[lint.per-path]]` for those).

A selector that suppresses nothing is reported as R016 `unused-suppression`,
with a fix that removes it (or the whole comment, when none of its selectors
suppress anything). A selector counts as used if it suppresses a problem of
any lint, even one that `LintSelection` leaves out. A malformed comment
(`usagi: ignore` without brackets, an unknown directive, a missing `]`, empty
brackets) or a selector that matches no lint is reported as S017
`invalid-suppression`, and suppresses nothing.

C016 `class-name-mismatch` is checked by `check_file_name`, after parsing,
which applies the suppressions too. Until it's called, a selector suppressing
only C016 is reported as unused. The lints that need the project are checked
by `project_lint::lint`, which applies them too; a selector that names one is
never reported as unused, because parsing can't tell. Such a lint is
suppressed only by a selector that names it, as it's selected: `ignore[ALL]`
and `ignore[correctness]` don't suppress it.

## Fixes

Some problems come with a suggested fix (`ProgramError::fix`): a list of text
edits. The **Fix** column below says whether a lint's problems have one:
**Yes** for every problem, **Sometimes** for some of them.

A fix is never offered if it would delete or move a comment, or leave one
describing code that's gone: a fix that deletes a statement or block isn't
offered if a comment is on any of its lines (`X=0 // off for now`), and
removing parentheses or trailing commas isn't offered if a comment is among
them. Comments are found by `usagi_core::source::comment_ranges`, which skips
string and name literals, so `"http://..."` isn't a comment.

A fix is either *safe* (it never changes what the code does) or *unsafe*.
`usagi lint --fix` applies safe fixes, and `usagi lint --diff` shows what they
would change without writing anything (see [CLI.md](CLI.md)). Unsafe fixes are
never applied from the command line; they are for an editor to offer.

## Lints

### Correctness

| Code | Name | Severity | Fix | Reports |
|---|---|---|---|---|
| C001 | `multiple-function-types` | Error |  | A function that declares more than one of `function`, `event`, `delegate`, and so on. |
| C002 | `exec-consecutive-float-arguments` | Error |  | An `exec` function with consecutive `float` arguments, which the console can't parse. |
| C003 | `defaultproperties-name-literal` | Error |  | A `defaultproperties` value that is a name literal. These don't work, so use a string. |
| C004 | `break-outside-loop` | Error |  | `break` outside a loop or `switch`. |
| C005 | `continue-outside-loop` | Error |  | `continue` outside a loop. |
| C006 | `division-by-zero` | Warning |  | Division or modulo (`/`, `%`, `/=`) by a literal zero. |
| C007 | `array-index-out-of-bounds` | Warning |  | A literal index outside a static array declared in the file, in code or in `defaultproperties`. |
| C008 | `latent-call-outside-state-code` | Warning |  | A latent function (`Sleep`, `FinishAnim`, `MoveTo`, ...) called from a function rather than from state code. |
| C009 | `conflicting-access-modifiers` | Warning |  | More than one of `private`, `protected` and `public`. |
| C010 | `byte-value-out-of-range` | Warning |  | A literal outside [0..255] assigned to a `byte`, in code or in `defaultproperties`. |
| C011 | `duplicate-declaration` | Error |  | A name declared more than once in the same scope. Operator overloads, and the same value in different enums, are allowed. |
| C012 | `assignment-to-constant` | Error |  | An assignment (including `+=` and `++`) to a const, an enum value or a function. Assigning a function to a delegate is allowed. |
| C014 | `undeclared-replicated-member` | Error |  | A member in the replication block that isn't declared in the class itself. UE2 only lets a class replicate its own members. |
| C015 | `return-form-mismatch` | Error |  | `return X;` in a function with no return type, or a bare `return;` in a function that has one. |
| C016 | `class-name-mismatch` | Error |  | A class whose name differs from its file's name (without `.uc`), ignoring case. UCC rejects it. Only checked when linting files, since it needs the file name. |
| C017 | `undeclared-default-property` | Warning |  | A property assigned in `defaultproperties` that the class doesn't declare, or any class it extends. Needs the project, and is opt-in. Only reported when every parent of the class is in the project. `Begin Object` blocks aren't checked, and neither are names that are declared but aren't variables (a `const`, an enum value). |
| C018 | `actor-reference-in-object` | Error |  | A variable of a class that isn't an `Actor` subclass whose type is an `Actor` (or a subclass), an array of them, or a struct with such a member (however deep, through arrays and other structs). The garbage collector doesn't clear such a reference when the actor is destroyed, and the game can crash on it. `class<Actor>` variables are fine, and so are locals and arguments. `Actor` is `Engine.Actor`, or any class named `Actor` when packages aren't known. A struct's members inherited from a struct it `extends` aren't looked at. Needs the project, and is opt-in. Only reported when every parent of the class is in the project. |
| C019 | `unassigned-object-local` | Error |  | A read of a local that may hold an object (a class or `class<...>` type, not an array) where nothing can have assigned it, so it is always `None`: `P.X` is an Accessed None, `P == None` is always true, and passing or returning `P` passes `None`. The function's flow is followed: an assignment on any path that reaches the read, around a loop too, counts, so `if (Best == None) Best = X;` in a loop is fine. A local is assigned by `L = ...`, by a `foreach` iterator it's given, and by a call it's given to as an `out` argument. A call whose function can't be found (a parent not in the project, a call through `Super` or a `static` call on a class, a delegate held in a variable) may assign anything it's given, so nothing after it is reported. A call to a name that is a class, struct or enum and not a function is a cast. Code after `return`, `break` or `continue` isn't looked at, and neither is a function with `goto` or a label. Needs the project, and is opt-in. |

### Suspicious

| Code | Name | Severity | Fix | Reports |
|---|---|---|---|---|
| S001 | `empty-body` | Warning |  | An `if`, `else`, loop or `foreach` whose body is a lone `;`. Not reported for a `for` loop with a post clause, or a `while` loop whose condition has side effects, because there the header does the work. |
| S002 | `useless-expression` | Warning |  | An expression statement that has no effect, e.g. `A == B;`. |
| S003 | `self-assignment` | Warning |  | A variable assigned to itself. `for (i = i; ...)` is allowed. |
| S004 | `identical-operands` | Warning |  | Both operands of a comparison, logical or arithmetic operator are the same. `X != X`, the NaN check, is allowed. |
| S005 | `constant-condition` | Warning |  | An `if` or `while` condition that is a literal. `while (true)` is allowed. |
| S006 | `unreachable-code` | Warning |  | A statement after `return`, `break`, `continue`, `goto` or `stop`. A `break` straight after a `return` is allowed. |
| S007 | `duplicate-switch-case` | Warning |  | The same `case` value more than once in a `switch`. |
| S008 | `switch-case-fallthrough` | Warning |  | A non-empty `case` that falls through to the next one. |
| S009 | `negated-comparison-operand` | Warning |  | `!A == B`, which means `(!A) == B`. |
| S010 | `duplicate-default-property` | Warning |  | The same property assigned more than once in a `defaultproperties` block. Only the last value is used. |
| S011 | `rotator-component-out-of-range` | Warning |  | A rotator literal component outside the 16-bit range. |
| S012 | `rotator-component-not-integer` | Warning |  | A rotator literal component that isn't an integer. |
| S013 | `default-property-type-mismatch` | Warning |  | A `defaultproperties` value whose literal type doesn't match the variable's type. |
| S014 | `duplicate-replicated-member` | Warning |  | A member listed more than once in the replication block. |
| S015 | `shadowed-variable` | Warning |  | A local or argument with the same name as a variable or const of the class (or state). In a `static` function only consts count, since instance variables aren't accessible there. Arguments of a function without a body, such as a delegate or native function, aren't checked. |
| S016 | `missing-return` | Warning |  | A function with a return type where some path reaches the end without `return`; UCC returns zero or `None` there. Understands `if`/`else`, `switch` with `default` (including fallthrough into a case that returns) and `while (true)`/`for (;;)` loops without `break`. |
| S017 | `invalid-suppression` | Warning |  | A `usagi:` comment that is malformed, such as `usagi: ignore` without brackets, or a selector in one that matches no lint (see [Suppression comments](#suppression-comments)). |
| S018 | `non-exhaustive-switch` | Warning |  | A `switch` on an enum with no `default` that has no `case` for some of the enum's values. The enum can be declared in the class or in any class it extends, and the subject can be a local, an argument, a variable (also a parent's, another object's, a struct's or an array element), what a function returns, or a cast to the enum. A bare `case` label is a value if the enum has one of that name, wherever the enum is declared: a class that only `dependson`s the enum's class sees it too, as in UCC. A `switch` whose subject's type isn't known, or with a `case` that isn't known to be a value of the enum (a literal, another enum's value, an expression), isn't reported. A `switch` with a `default` isn't reported, even if its cases leave out values. Needs the project, and is opt-in. |
| S019 | `could-be-static` | Warning |  | A function that uses nothing of the object it's called on, so could be `static`: it names no variable or non-static function, and no `Self`, `Super` or `global`, whether declared in the class or inherited (`Default.X`, `Static.F()`, consts and static functions are fine). Only a class's own plain functions are reported: not `static`, `native`, `exec`, `latent`, `iterator` or `singular` functions, events, delegates, operators, state functions, functions with no body, functions that override or are overridden (changing them would change the signature), replicated functions, or functions used as a value (assigned to a delegate). A function that calls a non-static function isn't reported, even one that could itself be `static`. A name that can't be looked up, because a parent isn't in the project, stops the function being reported. Needs the project, and is opt-in. |

### Performance

| Code | Name | Severity | Fix | Reports |
|---|---|---|---|---|
| P001 | `actor-iterator-in-per-frame-function` | Warning |  | `foreach AllActors`, `DynamicActors` or `AllObjects` in a function that runs every frame (`Tick`, `PlayerTick`, `PostRender`, `DrawHUD`, `RenderOverlays`). |

### Redundancy

| Code | Name | Severity | Fix | Reports |
|---|---|---|---|---|
| R001 | `empty-statement` | Warning | Sometimes | A lone `;`. |
| R002 | `redundant-parentheses` | Warning | Sometimes | Parentheses that don't change the meaning of an expression. Parentheses around a unary expression that is the operand of `.`, `[]`, a call or another unary operator are needed, e.g. `(-A).B` and `-(-A)`, so they aren't reported. The fix isn't offered when a comment is inside the parentheses. |
| R003 | `duplicate-modifier` | Warning | Yes | The same modifier more than once on a class, variable or function. |
| R004 | `redundant-default-value` | Warning | Sometimes | A `defaultproperties` value equal to the type's zero value. |
| R005 | `empty-defaultproperties` | Warning | Sometimes | An empty `defaultproperties` block. |
| R006 | `empty-replication-block` | Warning | Sometimes | An empty `replication` block. |
| R007 | `duplicate-replication-condition` | Warning |  | Two replication statements with the same reliability and condition. |
| R008 | `trailing-empty-arguments` | Warning | Sometimes | Empty arguments at the end of a call, e.g. `Foo(A,,)`. |
| R009 | `unused-local` | Warning |  | A local variable that is never used. |
| R010 | `local-set-but-never-used` | Warning |  | A local variable that is only ever assigned, incremented or compound-assigned. Passing it to a function, `X.Y = ...` and `X[i] = ...` count as uses. |
| R011 | `unused-private-member` | Warning |  | A `private` variable or function that nothing in the file uses. Replication and `defaultproperties` count as uses. Not reported for native classes or members, events, delegates or `exec` functions. |
| R012 | `redundant-self` | Warning | Yes | A `Self.` qualifier that doesn't change what a name refers to: no argument, local or const of the enclosing function hides the member. |
| R013 | `redundant-return` | Warning | Sometimes | A bare `return;` as the last statement of a function. A body that is only `return;` is left to R017. |
| R014 | `redundant-case-before-default` | Warning | Sometimes | An empty `case` label directly before `default:`, which runs the same code without it. Labels whose value has side effects, such as a function call, are allowed. |
| R015 | `redundant-var-category` | Warning | Yes | A `var(Category)` whose category is the class's name, e.g. `var(Foo)` in class `Foo`. `var()` already uses the class's name. |
| R016 | `unused-suppression` | Warning | Yes | A lint selector in a `usagi: ignore[...]` or `usagi: ignore-file[...]` comment that suppresses nothing (see [Suppression comments](#suppression-comments)). The fix removes the selector, or the comment if none of its selectors suppress anything. |
| R017 | `redundant-default-return` | Warning | Sometimes | A function whose body is only `return` of its return type's default value (`0`, `False`, `""`, `''`, `None`, `vect(0,0,0)`, `rot(0,0,0)`), which is what it returns without a body, or a function with no return type whose body is only `return;`. A function without a body still overrides its parent's version, so `function F() { return; }` and `function F();` both stub it out. The fix replaces the body with `;`, unless a comment is in it. |
| R018 | `empty-function-body` | Warning | Sometimes | A function whose body is empty, `{}` (or holds only stray `;`). A function without a body is the same: it still overrides its parent's version and returns the default value, so `function F() {}` is written `function F();`. The fix replaces the body with `;`, unless a comment is in it. A body with only `local` declarations isn't reported. |

### Naming

| Code | Name | Severity | Fix | Reports |
|---|---|---|---|---|
| N001 | `const-name-case` | Warning |  | A `const` name that isn't SCREAMING_SNAKE_CASE. |
| N002 | `function-name-case` | Warning |  | A function name that doesn't begin with a capital letter. Operators are exempt. |
| N003 | `enum-name-prefix` | Warning |  | An enum name that doesn't begin with `E`. |
| N004 | `bool-name-prefix` | Warning |  | A `bool` variable name that doesn't begin with `b`. |
| N005 | `object-literal-type-case` | Warning |  | An object literal type that doesn't begin with a capital letter, e.g. `texture'Foo'`. |
| N006 | `identifier-case-mismatch` | Warning | Sometimes | A name written with different capitalization than its declaration in the same file. |
| N007 | `class-name-ends-in-digit` | Warning |  | A class name that ends in a digit. UCC warns about it too, but `usagi build` hides that warning, since renaming a class can break everything that refers to it and fixing it may not be practical. The engine strips trailing digits when naming instances, so `Tank_M4A3E8` makes `Tank_M4A3E0`, `Tank_M4A3E1`, …, which is confusing in logs but harmless.

### Documentation

| Code | Name | Severity | Fix | Reports |
|---|---|---|---|---|
| D001 | `continued-trailing-comment` | Warning |  | An end-of-line `//` comment continued by `//` comments on the lines below it, lined up in the same column (a tab reaches the next multiple of 4 columns), e.g. `var int A; // The quick brown fox` followed by `           // jumps over the lazy dog.` Document a declaration with comments above it instead. One problem covers the comment and its continuations. `usagi format` keeps the continuations lined up under the comment (see [FORMATTING.md](FORMATTING.md#comments)). |

### Formatting

There are no formatting lints: `usagi format` formats code (see
[FORMATTING.md](FORMATTING.md)). The formatting lints that came before it,
F001 `boolean-literal-case`, F002 `object-literal-keyword-case` and F003
`unsorted-modifiers`, were removed, and their codes aren't reused.

## Problems that aren't lints

Problems that permissive parsing recovers from have no lint code
(`ProgramError::lint` is `None`) and can't be deselected. Examples are
discarded `defaultproperties` lines and malformed float literals like `0.0.5`.
In strict mode they are returned as `ParsingError::Strict` instead.
