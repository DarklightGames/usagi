# Formatting

`usagi format` reprints UnrealScript files in a single house style. It doesn't
edit the text it's given: it parses each file, then prints the syntax tree from
scratch, so a file's layout depends only on its code and the configuration,
never on how it was written. Nobody has to argue about formatting: the
formatter decides.

The style follows the Darkest Hour codebase (`DH_*/Classes`, 4,969 files),
which is far more consistent than the UT2004 code it's built on. Where this
document quotes how DH writes something, the figure is a count of lines or
matches in that code, with comments, strings and `defaultproperties` removed.

`usagi_core::format::format_text` implements it, and `usagi format` runs it
(see [CLI.md](CLI.md#usagi-format-paths)). Tests are in `tests/formatter.rs`
(the output, a test per rule) and `tests/format.rs` (the safety check).
Formatting the UT2004 and Red Orchestra/DH sources (10,088 files) changes no
program, and formatting the output again changes nothing; 4 files, none of
them DH's, have a comment that can't be placed.

## Principles

- **Never change what the code means.** After printing, the formatter parses
  its output again and compares the result with the input's tree (see
  [Safety](#safety)). If they differ, the file is left alone and reported as
  an internal error.
- **Stable output.** Formatting a formatted file changes nothing.
- **Whole files only.** No line ranges and no `format-off` regions (for now).
- **Few settings.** See [Configuration](#configuration).
- **Leave alone what it doesn't understand.** It never reorders declarations or
  statements, never adds or removes parentheses, and never changes
  identifiers, strings, names, `#exec` lines, `defaultproperties` bodies, or
  the text of comments.
- **Formatting is not linting.** `usagi lint` doesn't report formatting
  problems: the formatting lints, F001, F002 and F003, were removed, and their
  codes aren't reused. Identifier case (a reference spelled differently from
  its declaration) stays a lint.

## The command

```sh
usagi format [PATHS]...         # format files in place
usagi format --check [PATHS]... # write nothing; list files that would change
usagi format -                  # read stdin, write the result to stdout
```

| Flag | |
|---|---|
| `--check` | Write nothing. List each file that would change. |
| `--include`, `--exclude` | As for `lint`. |

There's no `--fix` (writing is the default) and no `--diff`. A file that can't
be formatted is reported as `path:line:column: message`.

Exit codes:

| Code | |
|---|---|
| 0 | Success. A file that couldn't be formatted (see below) is reported, but doesn't fail the run. |
| 1 | `--check`: a file would change. |
| 2 | `--check`: a file couldn't be formatted. |

A file can't be formatted when it doesn't parse, when a comment can't be
placed, or when the safety check fails.

## Pipeline

For each file:

1. Decode it (Latin-1 or UTF-16, as `source.rs` does), remembering its
   encoding, BOM and line endings.
2. Parse it. The comments come from a separate lexical pass
   (`source::comment_ranges`), which suppression comments use too.
3. Repair what permissive parsing recovered from (see
   [Permissively parsed files](#permissively-parsed-files)).
4. Print the tree, attaching each comment to the node it belongs to.
5. Parse the output again and compare (see [Safety](#safety)).
6. Encode it in the original encoding and write it, or with `--check`, report
   it if it changed.

The tests, and the corpus run, also format the output a second time and check
that nothing changes.

### Comments

The grammar discards comments, but `source::comment_ranges` finds them in a
separate pass over the text (skipping string and name literals, and nesting
block comments as the grammar does), and `ProgramResult::comments` holds their
byte ranges. The printer attaches each to a node by its position, as rustfmt
does. The AST and the lints are unaffected.

Comments inside text that's copied exactly (`#exec` lines, `defaultproperties`
and `cpptext` bodies) are copied with it, not placed separately.

- A comment on its own line stays on its own line, above the code that follows
  it, re-indented to that code's level.
- A comment at the end of a line stays at the end of the line after the same
  token, after one space (R29). Comments aren't aligned in columns, so nobody
  has to keep them lined up by hand.
- `//` comments lined up below an end-of-line `//` comment, each alone on its
  line, continue it (R29b): they stay lined up under it, wherever it moves.
  The lint D001 `continued-trailing-comment` lists them, so they can be
  rewritten as comments above the code (see
  [STATIC_ANALYSIS.md](STATIC_ANALYSIS.md#documentation)).
- A comment after `case X:` stays on the `case` line.
- A comment before a block's `}` is indented like the block's contents. In a
  `switch`, that's the indentation of the `case` labels.
- A comment's text is never changed, and block comments aren't re-indented
  inside.
- A comment inside a line of code goes between the same two parts of it as in
  the source: two items of a list (arguments, parameters, declared names),
  two operands, two calls of a chain, a list's parentheses and its items, or
  an expression and the `;` after it. Within that gap, it goes after the
  separator: `A /* B */, C` becomes `A, /* B */ C`.
  - A block comment on one line stays inline, after a space. The line breaks
    only if it doesn't fit with its comments.
  - An end-of-line comment ends the line, after one space (R29), and the
    line breaks after it. In arguments or parameters, it puts them one per
    line (L2, R33).
  - Between arguments or parameters laid out one per line, a comment that
    was on its own line stays on its own line.
  - An end-of-line comment just before the `)` of an `if`, `while`,
    `switch`, `until` or `replication` condition goes after the `)`.
- A file with a comment that can't be placed is reported and left
  unchanged. Comments are never silently moved. That's a comment anywhere
  else in a line, e.g. between modifiers (`var config /* X */ int Y;`), inside
  a member access (`A./* B */C`) or in a `const`; an end-of-line comment
  right after an opening parenthesis, or before a closing one in an
  expression; and a block comment over several lines inside a line.
- A comment after a stray `;` (R17b) goes on its own line, since the `;` it
  followed is removed.

### Safety

The printed file must parse to the same tree as the input, ignoring spans,
with the same comments in the same order. `usagi_core::format::equivalent`
compares two programs: every AST node implements `PartialEq`, ignoring spans,
and `format::normalize` first undoes the changes the formatter is allowed to
make. Those are exactly the changes the rules below make, and nothing else:

- keyword case (R43–R48), which the AST doesn't keep, and identifiers
  compare case-insensitively anyway;
- modifier order (R32);
- braces added around a single statement (R14b);
- extra semicolons removed (R17b);
- literals spelled differently (R49, R50): numbers compare by value;
- repairs of permissively parsed files;
- trailing whitespace on `#exec` lines.

`defaultproperties` blocks compare by the text between their braces, ignoring
whitespace at the ends of lines and of the block, and whether the block is
closed.

A comparison can only see what the AST keeps, so anything that changes what
the code means must be in the AST. `goto 'Label'` and `goto Label` differ in
state code (`goto` takes a name expression there, so the unquoted label is a
variable), so `CodeStatement::Goto` records whether the label is quoted.

### Permissively parsed files

A file that parses only with recovery (see
[`PERMISSIVE_PARSING.md`](PERMISSIVE_PARSING.md)), such as one missing a `;`,
is formatted and repaired: the printed file is what the compiler effectively
read, written out strictly. The safety check compares the output with the
recovered tree. Each repair is listed in `PERMISSIVE_PARSING.md` and has a
test.

The repairs are:

- A malformed float is printed as the part that was read: `0.0.5` becomes
  `0.0`.
- A `defaultproperties` block with no closing `}` (UE2 reads it to the end of
  the file) is closed.

Other recovered problems in `defaultproperties` are copied as written, since
the block is.

### Encoding and line endings

Each file keeps its encoding and its line endings. The output always ends with
exactly one newline.

## Configuration

A `[format]` table in `usagi.toml`; a mod's own file overrides each key it
sets (see [CLI.md](CLI.md#configuration)). Every key is optional, and
`indent-width` can't be set with `indent-style = "tabs"`.

```toml
[format]
line-width = 120
indent-style = "spaces"   # or "tabs"
indent-width = 4
```

No other rule has a setting.

## Rules

### Whitespace and blank lines

| # | Rule |
|---|---|
| R1 | Indent with 4 spaces per level. |
| R2 | No trailing whitespace. |
| R3 | Exactly one newline at the end of the file. |
| R4 | At most one blank line in a row. |
| R5 | No blank line after `{` or before `}`. |
| R6 | Exactly one blank line between top-level declarations (functions, states, structs, enums, `replication`, `defaultproperties`). A comment directly above a declaration belongs to it, so the blank line goes above the comment. Functions without a body (`function F();`) are stacked: there's no blank line between two of them, or before the comments above the second. |
| R7 | Exactly one blank line after a function's `local` declarations. |
| R8 | Blank lines between statements are kept (R4 applies). |
| R9 | Blank lines between `var`, `const` and other class-level declarations are kept (R4 applies). |

### Braces and blocks

| # | Rule |
|---|---|
| R10 | Allman braces: `{` on its own line, at the indentation of the line before it, for every block. |
| R11 | `else` on its own line after `}`. |
| R12 | `else if (...)` on one line. |
| R13 | One statement per line. |
| R14 | A block written on one line, e.g. `{ return X; }`, is expanded to three lines. |
| R14b | The body of an `if`, `else`, `for`, `foreach` or `while` is always braced; braces are added around a single statement. |
| R15 | An empty block is `{` and `}` on separate lines, with nothing between them. |
| R16 | `do` blocks: `}` and `until (...);` on separate lines. |
| R17 | `};` after a struct or enum is kept: UCC requires the `;` (`RequireSymbol` in `UnScrCom.cpp`). |
| R17b | Extra semicolons, such as `;` after a function's `}` or `;;`, are removed. |

```unrealscript
if (Info == none)
{
    return;
}

do
{
    i++;
}
until (i >= 10);
```

### Spacing

| # | Rule |
|---|---|
| R18 | One space between a keyword and `(`: `if (`, `for (`, `while (`, `switch (`, `until (`, `return (`. |
| R19 | No space between a function's name and `(`, including iterators: `foreach AllActors(...)`. |
| R20 | No space inside `()` or `[]`. |
| R21 | One space after a comma, none before. |
| R22 | One space on each side of binary and assignment operators (`=`, `+=`, `==`, `&&`, `+`, `*`, `@`, `$`, `~=`, `<<`, ...). |
| R23 | No space between a unary operator and its operand: `!bX`, `-X`, `++i`, `i++`. |
| R24 | `for (i = 0; i < N; ++i)`: one space after each `;`. |
| R25 | No space around `.`, including `default.`, `static.`, `super.` and `super(Class).`. |
| R26 | No space inside `<>`: `class<Actor>`, `array<int>`. Closing brackets of nested types are written `> >`, since UCC reads `>>` as a shift. |
| R27 | No space before the `(` of `var(Group)`, `config(User)` or `native(123)`. |
| R28 | No space before `;`. |
| R29 | One space before an end-of-line comment, however it was aligned. |
| R29b | `//` comments that continue an end-of-line comment, lined up below it, each alone on its line, stay lined up under it. Columns count a tab as reaching the next multiple of 4. If something is printed between them, such as braces added around a statement (R14b), they're placed like any other comment. |

### Declarations

| # | Rule |
|---|---|
| R30 | A class declaration without modifiers is one line: `class X extends Y;`. |
| R31 | Class modifiers each go on their own line, indented once, with `;` after the last. |
| R32 | Modifiers are ordered: access modifier first, then the rest alphabetically. This applies to classes, functions and variables (formerly F003). |
| R33 | A function header is on one line if it fits. Otherwise each parameter goes on its own line, indented one level, followed by a line for the `)` (and the `;` of a declaration without a body) at the same indentation. A header with an end-of-line comment between its parentheses is broken the same way. Between parameters, a comment that was on its own line stays on its own line, and any other goes at the end of the line, after the comma. |
| R34 | A declaration with several names (`local int i, j, k;`) is kept as written, never split or merged; it's filled (L2b) if it doesn't fit. |
| R35 | One space between the parts of `var`, `local` and struct member declarations. Nothing is aligned in columns. |
| R36 | `const X = 8;` is spaced like an assignment. |
| R37 | `struct X` and `enum X` with `{` on the next line, members indented, closed by `};`. Enum values one per line, each followed by a comma, the last one too, so adding, removing or reordering a value changes only its own line. UCC accepts the trailing comma (`CompileEnum` in `UnScrCom.cpp`). |
| R38 | In `replication`, the condition (`reliable if (...)`) is on one line and its names on the next, indented once, separated by `, ` and wrapped to fill each line (L6). |

```unrealscript
class DHObstacleManager extends Actor
    notplaceable;

const BITFIELD_COUNT = 8;

var byte Bitfield0[BITFIELD_LENGTH];
var DHObstacleInfo Info;
var config bool bDebug;

struct Minefield
{
    var float MVKillTime;
    var float MVWarnInterval;
};

enum ETriggerAction
{
    TR_DoesNothing,
    TR_Triggers,
    TR_Resets,
};
```

### Statements and control flow

| # | Rule |
|---|---|
| R39 | `case` labels are indented one level inside the `switch` braces, and their statements one level further. |
| R41 | State labels (`Begin:`) are at column 0. |
| R42 | `goto`, `break`, `continue` and `return` are spaced by R18 and R28. |

### Keyword case

Keywords are lower case. Identifiers, including type names such as `Vector`
and `Rotator` (which are structs, not keywords), are never changed.

| # | Rule |
|---|---|
| R43 | Declaration and statement keywords are lower case: `class`, `function`, `local`, `if`, `foreach`, `switch`, `defaultproperties`, ... |
| R44 | `true`, `false` and `none` are lower case (formerly F001). |
| R45 | `self`, `super`, `default` and `static` are lower case. |
| R46 | Built-in types are lower case: `int`, `float`, `bool`, `byte`, `string`, `name`. `class<X>` is a built-in type too, so it's lower case (R48). |
| R47 | Object literals start with an upper-case letter, like the object types they name: `Class'X'`, `Enum'X'` (formerly F002). |
| R48 | `class<Actor>` is lower case. |

### Literals

| # | Rule |
|---|---|
| R49 | Floats have digits on both sides of the point: `1.` is written `1.0` and `.5` is written `0.5`. Otherwise a number is printed as written: no zeros added or removed. |
| R50 | Hex integers have a lower-case `0x` prefix; their digits are kept as written. |
| R51 | Strings and names are never changed. |
| R52 | `vect(...)` and `rot(...)` are spaced like calls. |

The AST stores a number's value, not its text, but every literal is an
`AstNode` with a span, so the printer copies its text from the source.

### Directives and verbatim code

| # | Rule |
|---|---|
| R53 | `#exec` lines are copied exactly, at column 0. |
| R54 | `defaultproperties` bodies are copied exactly (`DefaultProperties::body`), apart from trailing whitespace (R2). The keyword and braces follow R10 and R43. A T3D formatter in the `t3d` crate will replace this later. |
| R55 | `cpptext` blocks are copied exactly. |

### Line width

Lines are at most `line-width` columns (default 120), counting code only: an
end-of-line comment doesn't count. The layout is canonical: anything that fits
on one line is joined onto one line, whatever the author wrote, and anything
that doesn't is broken by the rules below.

| # | Rule |
|---|---|
| L1 | Breaks are never made inside a literal, or between a function's name and its `(`, and only at the `.` of a chain of calls (L7). A line that can't be broken (a long string, an object literal, an `A.B.C.D` path) is left over the width. |
| L2 | A call's arguments that don't fit on its line go one per line, followed by a line for the `)`, all indented one level from the call's line, like parameters (R33). Each argument is laid out on its own line: one that fits there stays on it, and a call that doesn't breaks the same way, a level further in. An empty slot's comma stays on the line before (`A,,`). |
| L2b | A list of declared names (`local`, `var`, `ignores`) that doesn't fit is filled: as many names per line as fit, with continuation lines indented one level from the line the list starts on. |
| L3 | A chain of binary operators that doesn't fit is broken after each operator, one operand per line, with continuation lines indented one level from the line the chain starts on. |
| L4 | The loosest-binding operator is broken first (`||` before `&&`), and each tighter group stays on one line if it fits. An assignment never breaks at its operator: its right-hand side does. |
| L5 | String concatenation (`@`, `$`) that doesn't fit is filled: as many pieces per line as fit, breaking after the operator. |
| L6 | `replication` names that don't fit are filled, indented one level below their condition. |
| L7 | A chain of two or more calls (`A.B(...).C(...)`) that doesn't fit is broken before each `.Call(...)`, one call per line, indented one level. A call that's an argument of a call in the chain is indented a level more. |
| L8 | Inside parentheses, continuation lines are indented one level more than outside them. |

Indentation is counted in levels, so it's the same with tabs. A tab counts as
4 columns. A line too long only because of an end-of-line comment isn't
broken.

```unrealscript
static function string GetString(
    optional int Switch,
    optional PlayerReplicationInfo RelatedPRI_1,
    optional PlayerReplicationInfo RelatedPRI_2,
    optional Object OptionalObject
    )
{
    ...
}

return Repl(
    Repl(Repl(default.KickedFromSquadMessage, "{0}", RelatedPRI_1.PlayerName), "{1}", RelatedPRI_2.PlayerName),
    "{2}",
    static.GetSquadName(RelatedPRI_2.Team.TeamIndex, ExtraValue, DHSquadReplicationInfo(OptionalObject))
    );

if (P == none ||
    Interaction == none ||
    OptionIndex >= Options.Length)
{
    return;
}

while (Controller != none &&
    Controller.Pawn != none &&
    Controller.Pawn.Health > 0)
{
    ...
}

bDedicated = Level.NetMode == NM_DedicatedServer ||
    (Level.NetMode == NM_ListenServer &&
        PlayerOwner != none &&
        PlayerOwner.Player.Console == none);

replication
{
    reliable if (bNetDirty && Role == ROLE_Authority)
        Bitfield0, Bitfield1, Bitfield2, Bitfield3, Bitfield4, Bitfield5, Bitfield6,
        Bitfield7, SavedBitfield0, SavedBitfield1;
}

return (new Class'JSONObject')
    .PutString("damage_type", DamageType.Name)
    .Put(
        "killer",
        (new Class'JSONObject')
            .PutString("id", KillerID)
            .PutInteger("team", KillerTeam)
        )
    .Put("victim", VictimObject);
```

With `if (`, a one-level indent happens to line up with the condition, as DH
writes it. With `while (` and `switch (` it doesn't; continuation lines are
still indented one level, for consistency with L2. (The `.Put("killer", ...)`
call above is shown broken as it would be at a narrower width.)

`for` headers, `case` labels, `const`s, class modifiers and `#exec` lines
aren't broken.

## Later

- A T3D formatter in the `t3d` crate, for `defaultproperties` bodies (R54).
- Comment formatting, such as `//comment` to `// comment`, and reflowing block
  comments.
- `format-off` and `format-on` regions.
