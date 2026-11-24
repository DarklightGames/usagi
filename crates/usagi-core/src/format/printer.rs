//! Printing a program in the house style (`docs/FORMATTING.md`).
//!
//! The printer walks the tree in source order and writes one line at a time.
//! Every line it prints from the source knows the range of source text it
//! came from, which is how comments are placed: comments before a line go on
//! their own lines above it, comments after it on the same source line go at
//! its end, and a comment inside it can't be placed, so the file isn't
//! formatted.

use super::{FormatOptions, Indent};
use crate::ast::{
    AstNode, ClassDeclaration, CodeStatement, CodeStatementOrBlock, ConstDeclaration,
    DefaultProperties, DyadicVerb, EnumDeclaration, Expression, ExpressionList,
    FunctionBodyStatement, FunctionDeclaration, Literal, LocalDeclaration, MonadicVerb,
    NumericLiteral, PodType, Program, ProgramStatement, ReplicationBlock, StateDeclaration,
    StateStatement, StructDeclaration, StructVarDeclaration, SwitchCaseType, Type, VarDeclaration,
    VarName, VarSize,
};
use std::ops::Range;

pub(super) fn print(
    source: &str,
    comments: &[Range<usize>],
    program: &Program,
    options: &FormatOptions,
) -> Result<String, (usize, String)> {
    let mut printer = Printer {
        src: source,
        comments,
        continuations: crate::source::comment_continuations(source, comments),
        next_comment: 0,
        pos: 0,
        lines: Vec::new(),
        level: 0,
        indent: match options.indent {
            Indent::Spaces(n) => " ".repeat(n),
            Indent::Tabs => "\t".to_string(),
        },
        indent_columns: match options.indent {
            Indent::Spaces(n) => n,
            Indent::Tabs => 4,
        },
        width: options.line_width,
        column: 0,
        line_level: 0,
        flat: false,
        after_open: false,
        force_blank: false,
        after_bodiless_function: false,
        no_blank: false,
        error: None,
    };
    printer.program(program);
    printer.leading(source.len(), true);
    if let Some(error) = printer.error {
        return Err(error);
    }
    let mut text = printer.lines.join("\n");
    text.truncate(text.trim_end().len());
    text.push('\n');
    Ok(text)
}

struct Printer<'a> {
    src: &'a str,
    /// Every comment's range, in order.
    comments: &'a [Range<usize>],
    /// For each comment, whether it continues an end-of-line comment
    /// (`source::comment_continuations`).
    continuations: Vec<bool>,
    /// The first comment not printed yet.
    next_comment: usize,
    /// How far the source has been printed.
    pos: usize,
    lines: Vec<String>,
    level: usize,
    indent: String,
    /// How many columns one level of indentation takes; a tab counts as 4.
    indent_columns: usize,
    /// The longest a line may be.
    width: usize,
    /// The column at the end of the last line.
    column: usize,
    /// The indentation level of the last line.
    line_level: usize,
    /// Whether to print without breaking lines, because the code fits with
    /// the block comments inside it.
    flat: bool,
    /// Whether the last line opened a block, so no blank line may follow.
    after_open: bool,
    /// Whether the next line must be preceded by a blank line.
    force_blank: bool,
    /// Whether the last declaration printed was a function without a body,
    /// which another one may follow without a blank line (R6).
    after_bodiless_function: bool,
    /// Whether the next line, and the comments before it, mustn't be
    /// preceded by a blank line.
    no_blank: bool,
    /// Where the first thing that can't be printed is, and what it is.
    error: Option<(usize, String)>,
}

impl<'a> Printer<'a> {
    fn fail(&mut self, offset: usize, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some((offset, message.into()));
        }
    }

    // ---------------------------------------------------------------------
    // Source positions

    fn comment_at(&self, pos: usize) -> Option<&'a Range<usize>> {
        let i = self.comments.partition_point(|c| c.start < pos);
        self.comments.get(i).filter(|c| c.start == pos)
    }

    /// The position of the next token at or after `pos`, skipping whitespace
    /// and comments.
    fn next_token(&self, mut pos: usize) -> usize {
        let bytes = self.src.as_bytes();
        loop {
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            match self.comment_at(pos) {
                Some(c) => pos = c.end,
                None => return pos,
            }
        }
    }

    /// The end of the last token before `pos`, skipping whitespace and
    /// comments.
    fn prev_token_end(&self, mut pos: usize) -> usize {
        let bytes = self.src.as_bytes();
        loop {
            while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
                pos -= 1;
            }
            // At the end of a comment, or inside one that ends in whitespace.
            let i = self.comments.partition_point(|c| c.end < pos);
            match self.comments.get(i).filter(|c| c.start < pos) {
                Some(c) => pos = c.start,
                None => return pos,
            }
        }
    }

    /// The end of the token `token` that's next after `pos`, or `pos` if the
    /// next token is something else.
    fn past(&self, pos: usize, token: &str) -> usize {
        let next = self.next_token(pos);
        if self.src[next..].starts_with(token) {
            next + token.len()
        } else {
            pos
        }
    }

    /// Where the code of a node whose span ends at `end` ends. A span can
    /// include comments after the code, when the grammar looks for something
    /// optional after them.
    fn code_end(&self, end: usize) -> usize {
        self.prev_token_end(end)
    }

    fn has_comment_in(&self, range: Range<usize>) -> bool {
        let i = self.comments.partition_point(|c| c.start < range.start);
        self.comments.get(i).is_some_and(|c| c.start < range.end)
    }

    fn text(&self, range: Range<usize>) -> &'a str {
        &self.src[range]
    }

    // ---------------------------------------------------------------------
    // Lines and comments

    fn indentation(&self) -> String {
        self.indent.repeat(self.level)
    }

    fn push_line(&mut self, line: String) {
        self.after_open = line.trim_end().ends_with('{');
        self.lines.push(line);
    }

    fn blank(&mut self) {
        if !self.lines.is_empty()
            && !self.after_open
            && self.lines.last().is_some_and(|l| !l.is_empty())
        {
            self.lines.push(String::new());
        }
    }

    /// Whether the source between `from` and `to` has a blank line.
    fn has_blank_line(&self, from: usize, to: usize) -> bool {
        self.src[from.min(to)..to].matches('\n').count() >= 2
    }

    /// Prints the comments before `start`, and a blank line before them or
    /// before `start` where the source has one (if `blank` allows it) or a
    /// blank line is forced.
    fn leading(&mut self, start: usize, blank: bool) {
        while let Some(c) = self.comments.get(self.next_comment).cloned() {
            if c.start >= start {
                break;
            }
            let index = self.next_comment;
            self.next_comment += 1;
            let gap = &self.src[self.pos.min(c.start)..c.start];
            if !gap.contains('\n') && self.pos > 0 && gap.trim().is_empty() {
                // After code on the same source line, such as a block copied
                // as written: an end-of-line comment.
                let last = self
                    .lines
                    .last_mut()
                    .expect("the code before `pos` has been printed");
                last.push(' ');
                last.push_str(&comment_text(self.src, &c));
                self.pos = c.end;
                continue;
            }
            if let Some(line) = self.continuation(index) {
                self.push_line(line);
                self.pos = c.end;
                continue;
            }
            if !self.no_blank && (self.force_blank || self.has_blank_line(self.pos, c.start)) {
                self.blank();
            }
            self.force_blank = false;
            let line = format!("{}{}", self.indentation(), comment_text(self.src, &c));
            self.push_line(line);
            self.after_open = false;
            self.pos = c.end;
        }
        if !self.no_blank && (self.force_blank || (blank && self.has_blank_line(self.pos, start))) {
            self.blank();
        }
        self.force_blank = false;
        self.no_blank = false;
    }

    /// The line for the comment at `index` if it continues the end-of-line
    /// comment above it, which ends the last line: the comment, lined up
    /// under that one.
    fn continuation(&self, index: usize) -> Option<String> {
        if !self.continuations[index] {
            return None;
        }
        let above = comment_text(self.src, &self.comments[index - 1]);
        let last = self
            .lines
            .last()
            .expect("the end-of-line comment above has been printed");
        let column = last.strip_suffix(above.as_str())?.len();
        let mut line: String = last[..column]
            .chars()
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        line.push_str(&comment_text(self.src, &self.comments[index]));
        Some(line)
    }

    /// Prints `text` as a line, at the current indentation, for the source
    /// in `range`.
    fn line(&mut self, range: Range<usize>, text: &str, blank: bool) {
        let level = self.level;
        self.code_line(range, blank, level, |p| p.put(text));
    }

    /// Prints `text` as a line at column 0.
    fn line_at_column_0(&mut self, range: Range<usize>, text: &str, blank: bool) {
        self.code_line(range, blank, 0, |p| p.put(text));
    }

    /// Prints the code in `range` as a line at `level`, and the lines it
    /// continues on if it's too long: the comments before it, what `print`
    /// prints, and the comments after it on the same source line. A comment
    /// inside it that `print` doesn't place can't be placed.
    fn code_line(
        &mut self,
        range: Range<usize>,
        blank: bool,
        level: usize,
        print: impl FnOnce(&mut Self),
    ) {
        self.leading(range.start, blank);
        let line = self.indent.repeat(level);
        self.push_line(line);
        self.column = level * self.indent_columns;
        self.line_level = level;
        print(self);
        while let Some(c) = self.comments.get(self.next_comment)
            && c.start < range.end
        {
            let offset = c.start;
            self.fail(offset, "a comment here can't be placed");
            self.next_comment += 1;
        }
        self.pos = self.pos.max(range.end);
        // Comments after the line's code, on the same source line.
        while let Some(c) = self.comments.get(self.next_comment).cloned() {
            let gap = &self.src[self.pos.min(c.start)..c.start];
            if gap.contains('\n') || !gap.trim().is_empty() {
                break;
            }
            let text = format!(" {}", comment_text(self.src, &c));
            self.lines
                .last_mut()
                .expect("a line is open")
                .push_str(&text);
            self.pos = c.end;
            self.next_comment += 1;
        }
        self.after_open = self
            .lines
            .last()
            .is_some_and(|l| l.trim_end().ends_with('{'));
    }

    /// Prints a line that isn't in the source, such as braces added around
    /// a statement.
    fn synthetic(&mut self, text: &str) {
        let line = format!("{}{}", self.indentation(), text);
        self.push_line(line);
    }

    /// Skips the comments in `range`, which is copied as written.
    fn verbatim(&mut self, range: Range<usize>) {
        self.leading(range.start, true);
        while self
            .comments
            .get(self.next_comment)
            .is_some_and(|c| c.start < range.end)
        {
            self.next_comment += 1;
        }
    }

    // ---------------------------------------------------------------------
    // Declarations

    fn program(&mut self, program: &Program) {
        for statement in &program.statements {
            let span = statement.span;
            let stacked = std::mem::take(&mut self.after_bodiless_function);
            match &statement.data {
                // A stray `;` is removed (R17b), so functions on either side
                // of it are still stacked.
                ProgramStatement::Empty => self.after_bodiless_function = stacked,
                ProgramStatement::ClassDeclaration(class) => {
                    self.class(class);
                    self.force_blank = true;
                }
                ProgramStatement::CompilerDirective(directive) => {
                    self.directive(span.start..span.end, &directive.command)
                }
                ProgramStatement::ConstDeclaration(c) => self.constant(c),
                ProgramStatement::VarDeclaration(v) => self.var(v),
                ProgramStatement::EnumDeclaration(e) => {
                    self.spaced(|p| p.enumeration(e, Some(span.end)))
                }
                ProgramStatement::StructDeclaration(s) => {
                    self.spaced(|p| p.structure(s, span.start, None))
                }
                ProgramStatement::FunctionDeclaration(f) => self.spaced_function(f, stacked),
                ProgramStatement::ReplicationBlock(r) => self.spaced(|p| p.replication(r)),
                ProgramStatement::StateDeclaration(s) => {
                    self.spaced(|p| p.state(s, span.start..span.end))
                }
                ProgramStatement::DefaultProperties(d) => self.spaced(|p| p.default_properties(d)),
                ProgramStatement::CppText(body) => {
                    self.spaced(|p| p.cpp("cpptext", body, span.start..span.end))
                }
            }
        }
    }

    /// Prints a declaration with exactly one blank line on each side (R6).
    fn spaced(&mut self, print: impl FnOnce(&mut Self)) {
        self.force_blank = true;
        print(self);
        self.force_blank = true;
    }

    /// Prints a function like any declaration (R6), except that a function
    /// without a body that follows another (`stacked`) has no blank line
    /// before it, or before the comments above it.
    fn spaced_function(&mut self, f: &AstNode<FunctionDeclaration>, stacked: bool) {
        let bodiless = f.data.body.is_none();
        self.no_blank = bodiless && stacked;
        self.force_blank = !self.no_blank;
        self.function(f);
        self.force_blank = true;
        self.after_bodiless_function = bodiless;
    }

    fn class(&mut self, class: &AstNode<ClassDeclaration>) {
        let c = &class.data;
        let mut header = format!("class {}", c.name.string);
        let mut end = c.name.span.end;
        if let Some(parent) = &c.parent_class {
            header.push_str(&format!(" extends {}", parent.string));
            end = parent.span.end;
        }
        if let Some(within) = &c.within {
            header.push_str(&format!(" within {}", within.string));
            end = within.span.end;
        }
        if c.modifiers.is_empty() {
            if self.has_comment_in(end..class.span.end) {
                // `class A extends B // abstract` and `;` on the next line.
                self.line(class.span.start..end, &header, true);
                self.level += 1;
                self.line(class.span.end - 1..class.span.end, ";", false);
                self.level -= 1;
            } else {
                self.line(
                    class.span.start..class.span.end,
                    &format!("{header};"),
                    true,
                );
            }
            return;
        }
        self.line(class.span.start..end, &header, true);
        let mut modifiers: Vec<_> = c
            .modifiers
            .iter()
            .map(|m| {
                let mut text = m.type_.data.to_string();
                if let Some(arguments) = &m.arguments {
                    text.push_str(&format!("({})", self.expression_list(arguments)));
                }
                (m.span, text)
            })
            .collect();
        sort_modifiers(&mut modifiers, |(_, t)| t);
        self.level += 1;
        let last = modifiers.len() - 1;
        for (i, (span, text)) in modifiers.into_iter().enumerate() {
            if i < last {
                self.line(span.start..span.end, &text, false);
            } else if !self.has_comment_in(span.end..class.span.end) {
                self.line(span.start..class.span.end, &format!("{text};"), false);
            } else {
                // A comment between the last modifier and the `;`.
                self.line(span.start..span.end, &text, false);
                self.line(class.span.end - 1..class.span.end, ";", false);
            }
        }
        self.level -= 1;
    }

    fn directive(&mut self, span: Range<usize>, command: &str) {
        let end = span.start + 1 + command.trim_end().len();
        self.verbatim(span.start..end);
        self.line_at_column_0(span.start..end, &format!("#{}", command.trim_end()), true);
    }

    fn constant(&mut self, c: &AstNode<ConstDeclaration>) {
        let text = format!("const {} = {};", c.name.string, self.literal(&c.value));
        self.line(c.span.start..c.span.end, &text, true);
    }

    fn var(&mut self, v: &AstNode<VarDeclaration>) {
        let mut head = "var".to_string();
        if let Some(category) = &v.category {
            head.push_str(&format!("({})", category.data));
        }
        let mut modifiers: Vec<String> = v.modifiers.iter().map(|m| m.data.to_string()).collect();
        sort_modifiers(&mut modifiers, |m| m);
        for m in modifiers {
            head.push(' ');
            head.push_str(&m);
        }
        self.declaration(v.span.start..v.span.end, &head, &v.type_, &v.names);
    }

    /// Prints `head type names;`, where the type may be a struct or enum
    /// declared inline.
    fn declaration(
        &mut self,
        span: Range<usize>,
        head: &str,
        type_: &Type,
        names: &[AstNode<VarName>],
    ) {
        let names_text = names
            .iter()
            .map(|n| self.var_name(n))
            .collect::<Vec<_>>()
            .join(", ");
        match type_ {
            Type::Struct(s) => {
                self.structure(s, span.start, Some((head, &names_text, span.end)));
            }
            Type::Enum(e) => {
                let header = format!("{head} enum {}", e.name.string);
                self.line(span.start..e.name.span.end, &header, true);
                self.enumeration_body(e);
                let close = e.span.end - 1;
                self.line(close..span.end, &format!("}} {names_text};"), false);
            }
            _ => match self.type_text(type_) {
                Some(t) => {
                    let items: Vec<Item> = names
                        .iter()
                        .map(|n| Item::Text(n.span.start..n.span.end, self.var_name(n)))
                        .collect();
                    let first = names[0].span.start;
                    let type_end = self.prev_token_end(first);
                    let level = self.level;
                    self.code_line(span.clone(), true, level, |p| {
                        p.put(&format!("{head} {t} "));
                        p.opening_comments(type_end, first, true);
                        p.layout_list(first, span.end - 1, &items, 1, level + 1);
                        p.put(";");
                    });
                }
                None => self.fail(span.start, "a struct or enum declared inside an array"),
            },
        }
    }

    fn var_name(&self, name: &AstNode<VarName>) -> String {
        let mut text = name.identifier.string.clone();
        if let Some(size) = &name.size {
            let size = match &size.data {
                VarSize::IntegerLiteral(n) => self.number(n),
                VarSize::Identifier(i) => i.string.clone(),
            };
            text.push_str(&format!("[{size}]"));
        }
        text
    }

    fn type_text(&self, type_: &Type) -> Option<String> {
        Some(match type_ {
            Type::Pod(pod) => match pod {
                PodType::Byte => "byte",
                PodType::Int => "int",
                PodType::Float => "float",
                PodType::String => "string",
                PodType::Name => "name",
                PodType::Bool => "bool",
            }
            .to_string(),
            Type::Array(inner) => {
                let inner = self.type_text(inner)?;
                // `>>` would be read as a shift.
                let space = if inner.ends_with('>') { " " } else { "" };
                format!("array<{inner}{space}>")
            }
            Type::Class(class) => format!("class<{}>", class.string),
            Type::Identifier(i) => i.string.clone(),
            Type::Struct(_) | Type::Enum(_) => return None,
        })
    }

    fn enumeration(&mut self, e: &AstNode<EnumDeclaration>, statement_end: Option<usize>) {
        self.line(
            e.span.start..e.name.span.end,
            &format!("enum {}", e.name.string),
            true,
        );
        self.enumeration_body(e);
        let close = e.span.end - 1;
        let end = statement_end.unwrap_or(e.span.end);
        self.line(close..end, "};", false);
    }

    /// The braces and values of an enum, without the `}`.
    fn enumeration_body(&mut self, e: &AstNode<EnumDeclaration>) {
        let open = self.next_token(e.name.span.end);
        self.line(open..open + 1, "{", false);
        self.level += 1;
        for value in &e.values {
            let end = self.past(value.span.end, ",");
            let level = self.level;
            let comma_at = if end > value.span.end { end - 1 } else { end };
            self.code_line(value.span.start..end, true, level, |p| {
                p.put(&value.string);
                p.gap_comments(value.span.end, comma_at, false);
                p.put(",");
            });
        }
        let close = e.span.end - 1;
        self.leading(close, false);
        self.level -= 1;
    }

    /// Prints a struct. `start` is where its declaration starts. `var` is
    /// the `var ...` before it, the names after it, and where the
    /// declaration ends, for a struct declared in a `var`; otherwise the
    /// struct is closed by `};`.
    fn structure(
        &mut self,
        s: &AstNode<StructDeclaration>,
        start: usize,
        var: Option<(&str, &str, usize)>,
    ) {
        let d = &s.data;
        let mut header = String::new();
        if let Some((head, _, _)) = var {
            header.push_str(head);
            header.push(' ');
        }
        header.push_str("struct");
        let mut modifiers: Vec<String> = d.modifiers.iter().map(|m| m.data.to_string()).collect();
        sort_modifiers(&mut modifiers, |m| m);
        for m in modifiers {
            header.push(' ');
            header.push_str(&m);
        }
        header.push(' ');
        header.push_str(&d.name.string);
        let mut end = d.name.span.end;
        if let Some(parent) = &d.parent {
            header.push_str(&format!(" extends {}", parent.string));
            end = parent.span.end;
        }
        self.line(start..end, &header, true);
        let open = self.next_token(end);
        self.line(open..open + 1, "{", false);
        self.level += 1;
        for member in &d.members {
            self.struct_member(member);
        }
        let close = s.span.end - 1;
        if let Some(cpp) = &d.cpp {
            let keyword = self.src[self.pos..close]
                .to_ascii_lowercase()
                .find("cppstruct")
                .map_or(self.pos, |i| self.pos + i);
            let open = self.next_token(keyword + "cppstruct".len());
            self.cpp("cppstruct", cpp, keyword..open + cpp.len() + 2);
        }
        self.leading(close, false);
        self.level -= 1;
        match var {
            Some((_, names, end)) => self.line(close..end, &format!("}} {names};"), false),
            None => {
                let end = self.past(close + 1, ";");
                self.line(close..end, "};", false)
            }
        }
    }

    fn struct_member(&mut self, member: &AstNode<StructVarDeclaration>) {
        let m = &member.data;
        let mut head = "var".to_string();
        if m.is_editable {
            head.push_str("()");
        }
        let mut modifiers: Vec<String> = m.modifiers.iter().map(|m| m.to_string()).collect();
        sort_modifiers(&mut modifiers, |m| m);
        for modifier in modifiers {
            head.push(' ');
            head.push_str(&modifier);
        }
        self.declaration(
            member.span.start..member.span.end,
            &head,
            &m.type_,
            &m.names,
        );
    }

    /// Prints `keyword`, then `{body}` as written. `span` is the whole
    /// block, from the keyword to the `}`.
    fn cpp(&mut self, keyword: &str, body: &str, span: Range<usize>) {
        let open = self.next_token(span.start + keyword.len());
        let text = format!("{{{body}}}");
        self.copied_block(
            span.start..span.start + keyword.len(),
            keyword,
            open,
            &text,
            span.end,
        );
    }

    fn default_properties(&mut self, d: &AstNode<DefaultProperties>) {
        let keyword = d.span.start.."defaultproperties".len() + d.span.start;
        let open = d.body.span.start - 1;
        // No trailing whitespace (R2), and an unclosed block is closed (a
        // repair).
        let lines: Vec<&str> = d.body.data.split('\n').collect();
        let last = lines.len() - 1;
        let mut body = lines
            .iter()
            .enumerate()
            // The last line ends with the `}`, not a newline.
            .map(|(i, l)| if i < last { l.trim_end() } else { l })
            .collect::<Vec<_>>()
            .join("\n");
        if !d.closed {
            body = format!("{}\n", body.trim_end());
        }
        let text = format!("{{{body}}}");
        self.copied_block(keyword, "defaultproperties", open, &text, d.span.end);
    }

    /// Prints a keyword, then a block copied as written: `text`, from the `{`
    /// at `open` to `end`.
    fn copied_block(
        &mut self,
        keyword: Range<usize>,
        keyword_text: &str,
        open: usize,
        text: &str,
        end: usize,
    ) {
        self.line(keyword, keyword_text, true);
        self.leading(open, false);
        while self
            .comments
            .get(self.next_comment)
            .is_some_and(|c| c.start < end)
        {
            self.next_comment += 1;
        }
        let indentation = self.indentation();
        for (i, line) in text.split('\n').enumerate() {
            if i == 0 {
                self.push_line(format!("{indentation}{line}"));
            } else {
                self.push_line(line.to_string());
            }
        }
        self.after_open = false;
        self.pos = end;
    }

    fn function(&mut self, f: &AstNode<FunctionDeclaration>) {
        let d = &f.data;
        let mut parts: Vec<String> = Vec::new();
        let mut modifiers: Vec<String> = d
            .modifiers
            .iter()
            .map(|m| {
                let mut text = m.data.type_.to_string();
                if !m.data.arguments.is_empty() {
                    let arguments: Vec<String> =
                        m.data.arguments.iter().map(|a| self.number(a)).collect();
                    text.push_str(&format!("({})", arguments.join(", ")));
                }
                text
            })
            .collect();
        sort_modifiers(&mut modifiers, |m| m);
        parts.extend(modifiers);
        for t in &d.types {
            let mut text = t.type_.to_string();
            if let Some(arguments) = &t.arguments {
                let arguments: Vec<String> = arguments.iter().map(|a| self.number(a)).collect();
                text.push_str(&format!("({})", arguments.join(", ")));
            }
            parts.push(text);
        }
        if let Some(return_type) = &d.return_type {
            match self.type_text(return_type) {
                Some(t) => parts.push(t),
                None => self.fail(f.span.start, "a struct or enum declared as a return type"),
            }
        }
        let arguments: Vec<(Range<usize>, String)> = d
            .arguments
            .iter()
            .map(|a| {
                let mut modifiers: Vec<String> =
                    a.data.modifiers.iter().map(|m| m.to_string()).collect();
                sort_modifiers(&mut modifiers, |m| m);
                let type_ = self.type_text(&a.data.type_).unwrap_or_default();
                modifiers.push(type_);
                modifiers.push(self.var_name(&a.data.name));
                (a.span.start..a.span.end, modifiers.join(" "))
            })
            .collect();
        let head = format!("{} {}(", parts.join(" "), d.name.string);
        let open_parenthesis = self.next_token(d.name.span.end) + 1;
        let level = self.level;
        let Some(body) = &d.body else {
            let close = self.prev_token_end(f.span.end - 1) - 1;
            self.code_line(f.span.start..f.span.end, true, level, |p| {
                p.put(&head);
                p.layout_parameters(open_parenthesis, close, &arguments, 2, level + 1);
                p.put(");");
            });
            return;
        };
        let open = self.prev_token_end(body.span.start) - 1;
        let header_end = self.prev_token_end(open);
        self.code_line(f.span.start..header_end, true, level, |p| {
            p.put(&head);
            p.layout_parameters(open_parenthesis, header_end - 1, &arguments, 1, level + 1);
            p.put(")");
        });
        self.line(open..open + 1, "{", false);
        self.level += 1;
        let statements = &body.data.statements;
        for (i, statement) in statements.iter().enumerate() {
            match statement {
                FunctionBodyStatement::ConstDeclaration(c) => self.constant(c),
                FunctionBodyStatement::LocalDeclaration(l) => {
                    self.local(l);
                    // R7: a blank line after the locals.
                    if !matches!(
                        statements.get(i + 1),
                        None | Some(FunctionBodyStatement::LocalDeclaration(_))
                    ) {
                        self.force_blank = true;
                    }
                }
                FunctionBodyStatement::CodeStatement(s) => self.statement(s),
            }
        }
        self.close(f.span.end - 1);
    }

    fn local(&mut self, l: &AstNode<LocalDeclaration>) {
        self.declaration(l.span.start..l.span.end, "local", &l.type_, &l.names);
    }

    /// Prints the comments before the `}` at `close`, at the block's
    /// indentation, then the `}`, at the indentation outside the block.
    fn close(&mut self, close: usize) {
        self.force_blank = false;
        self.leading(close, false);
        self.level -= 1;
        self.line(close..close + 1, "}", false);
    }

    fn replication(&mut self, r: &AstNode<ReplicationBlock>) {
        let keyword_end = r.span.start + "replication".len();
        self.line(r.span.start..keyword_end, "replication", true);
        let open = self.next_token(keyword_end);
        self.line(open..open + 1, "{", false);
        self.level += 1;
        for statement in &r.statements {
            let start = self.next_token(self.pos);
            let end = self.past(statement.condition.span.end, ")");
            let keyword = format!("{} if", statement.reliability);
            self.predicate_line(start..end, true, &keyword, &statement.condition, "");
            let last = statement
                .variables
                .last()
                .expect("a replication statement names something");
            let mut end = self.past(last.span.end, ",");
            end = self.past(end, ";");
            let items: Vec<Item> = statement
                .variables
                .iter()
                .map(|v| Item::Text(v.span.start..v.span.end, v.string.clone()))
                .collect();
            let first = statement.variables[0].span.start;
            let level = self.level + 1;
            // L6: the names fill lines at the same indentation.
            self.code_line(first..end, false, level, |p| {
                p.layout_list(first, end - 1, &items, 1, level);
                p.put(";");
            });
        }
        self.close(r.span.end - 1);
    }

    fn state(&mut self, s: &StateDeclaration, span: Range<usize>) {
        let mut modifiers: Vec<String> = s.modifiers.iter().map(|m| m.to_string()).collect();
        sort_modifiers(&mut modifiers, |m| m);
        let mut header = modifiers.join(" ");
        if !header.is_empty() {
            header.push(' ');
        }
        header.push_str("state");
        if s.is_editable {
            header.push_str("()");
        }
        header.push(' ');
        header.push_str(&s.name.string);
        let mut end = s.name.span.end;
        if let Some(parent) = &s.parent {
            header.push_str(&format!(" extends {}", parent.string));
            end = parent.span.end;
        }
        self.line(span.start..end, &header, true);
        let open = self.next_token(end);
        self.line(open..open + 1, "{", false);
        self.level += 1;
        if let Some(last) = s.ignores.last() {
            let start = self.next_token(self.pos);
            let end = self.past(last.span.end, ";");
            let items: Vec<Item> = s
                .ignores
                .iter()
                .map(|i| Item::Text(i.span.start..i.span.end, i.string.clone()))
                .collect();
            let first = self.next_token(start + "ignores".len());
            let level = self.level;
            self.code_line(start..end, true, level, |p| {
                p.put("ignores ");
                p.opening_comments(start, first, true);
                p.layout_list(first, end - 1, &items, 1, level + 1);
                p.put(";");
            });
        }
        for statement in &s.statements {
            let stacked = std::mem::take(&mut self.after_bodiless_function);
            match statement {
                StateStatement::ConstDeclaration(c) => self.constant(c),
                StateStatement::FunctionDeclaration(f) => self.spaced_function(f, stacked),
            }
        }
        self.after_bodiless_function = false;
        for label in &s.labels {
            let end = self.past(label.label.span.end, ":");
            self.line_at_column_0(
                label.label.span.start..end,
                &format!("{}:", label.label.string),
                true,
            );
            for statement in &label.statements {
                self.statement(statement);
            }
        }
        self.close(span.end - 1);
    }

    // ---------------------------------------------------------------------
    // Statements

    fn statement(&mut self, s: &AstNode<CodeStatement>) {
        let span = s.span.start..s.span.end;
        match &s.data {
            CodeStatement::Empty => {}
            CodeStatement::Expression(e) => {
                let level = self.level;
                let end = self.code_end(e.span.end);
                self.code_line(span.clone(), true, level, |p| {
                    p.root(e, 1);
                    p.gap_comments(end, span.end - 1, false);
                    p.put(";");
                });
            }
            CodeStatement::Return(None) => self.line(span, "return;", true),
            CodeStatement::Return(Some(e)) => {
                let level = self.level;
                let end = self.code_end(e.span.end);
                self.code_line(span.clone(), true, level, |p| {
                    p.put("return ");
                    p.root(e, 1);
                    p.gap_comments(end, span.end - 1, false);
                    p.put(";");
                });
            }
            CodeStatement::Break => self.line(span, "break;", true),
            CodeStatement::Continue => self.line(span, "continue;", true),
            CodeStatement::Goto { label, quoted } => {
                let text = if *quoted {
                    format!("goto '{}';", label.string)
                } else {
                    format!("goto {};", label.string)
                };
                self.line(span, &text, true);
            }
            CodeStatement::JumpLabel(label) => {
                self.line_at_column_0(span, &format!("{}:", label.string), true)
            }
            CodeStatement::CompilerDirective(d) => self.directive(span, &d.command),
            CodeStatement::ConstDeclaration(c) => self.constant(c),
            CodeStatement::ForEach(f) => {
                let level = self.level;
                let end = self.prev_token_end(f.body.span.start);
                self.code_line(span.start..end, true, level, |p| {
                    p.put("foreach ");
                    p.root(&f.predicate, 0);
                });
                self.body(Some(&f.body));
            }
            CodeStatement::For(f) => {
                let part = |e: &Option<Box<AstNode<Expression>>>| match e {
                    Some(e) => self.expression(e),
                    None => String::new(),
                };
                let mut header = format!("for ({};", part(&f.init));
                if f.predicate.is_some() {
                    header.push(' ');
                }
                header.push_str(&part(&f.predicate));
                header.push(';');
                if f.post.is_some() {
                    header.push(' ');
                }
                header.push_str(&part(&f.post));
                header.push(')');
                self.header(span.start, &f.body, &header);
                self.body(Some(&f.body));
            }
            CodeStatement::While(w) => {
                let end = self.prev_token_end(w.body.span.start);
                self.predicate_line(span.start..end, true, "while", &w.predicate, "");
                self.body(Some(&w.body));
            }
            CodeStatement::DoUntil(d) => {
                self.line(span.start..span.start + 2, "do", true);
                self.body(Some(&d.body));
                if let Some(predicate) = &d.predicate {
                    let start = self.next_token(self.pos);
                    let end = self.past(predicate.span.end, ")");
                    let end = self.past(end, ";");
                    self.predicate_line(start..end, false, "until", predicate, ";");
                }
            }
            CodeStatement::Switch(sw) => {
                let end = self.past(sw.predicate.span.end, ")");
                self.predicate_line(span.start..end, true, "switch", &sw.predicate, "");
                let open = self.next_token(end);
                self.line(open..open + 1, "{", false);
                self.level += 1;
                for case in &sw.cases {
                    let label = match &case.type_ {
                        SwitchCaseType::Expression(e) => format!("case {}:", self.expression(e)),
                        SwitchCaseType::Default => "default:".to_string(),
                    };
                    self.line(case.label_span.start..case.label_span.end, &label, true);
                    self.level += 1;
                    for statement in &case.statements {
                        self.statement(statement);
                    }
                    self.level -= 1;
                }
                self.close(span.end - 1);
            }
            CodeStatement::Conditional(c) => {
                let i = &c.if_statement;
                let end = self.past(i.predicate.span.end, ")");
                self.predicate_line(span.start..end, true, "if", &i.predicate, "");
                self.body(i.body.as_ref());
                for elif in &c.elif_statements {
                    let start = self.next_token(self.pos);
                    let end = self.past(elif.predicate.span.end, ")");
                    self.predicate_line(start..end, false, "else if", &elif.predicate, "");
                    self.body(Some(&elif.body));
                }
                if let Some(e) = &c.else_statement {
                    let start = self.next_token(self.pos);
                    self.line(start..start + 4, "else", false);
                    self.body(Some(&e.body));
                }
            }
        }
    }

    /// Prints the header of a statement with a body, from `start` to the
    /// body.
    fn header(&mut self, start: usize, body: &AstNode<CodeStatementOrBlock>, text: &str) {
        let end = self.prev_token_end(body.span.start);
        self.line(start..end, text, true);
    }

    /// Prints a body as a block, adding braces if it's a single statement.
    fn body(&mut self, body: Option<&AstNode<CodeStatementOrBlock>>) {
        match body.map(|b| &b.data) {
            Some(CodeStatementOrBlock::CodeBlock(block)) => {
                let open = block.span.start;
                self.line(open..open + 1, "{", false);
                self.level += 1;
                for statement in &block.statements {
                    self.statement(statement);
                }
                self.close(block.span.end - 1);
            }
            Some(CodeStatementOrBlock::CodeStatement(statement)) => {
                self.synthetic("{");
                self.level += 1;
                self.statement(statement);
                self.level -= 1;
                self.synthetic("}");
            }
            None => {
                self.synthetic("{");
                self.synthetic("}");
            }
        }
    }

    // ---------------------------------------------------------------------
    // Line width

    /// Prints `e` at the start of a statement's line.
    fn root(&mut self, e: &AstNode<Expression>, suffix: usize) {
        let outer = self.line_level + 1;
        self.layout(e, suffix, outer);
    }

    /// Prints a line `{keyword} ({predicate}){end}`, for the source in
    /// `range`.
    fn predicate_line(
        &mut self,
        range: Range<usize>,
        blank: bool,
        keyword: &str,
        predicate: &AstNode<Expression>,
        end: &str,
    ) {
        let level = self.level;
        let predicate_end = self.code_end(predicate.span.end);
        let close = self.next_token(predicate_end);
        let open = self.prev_token_end(predicate.span.start);
        self.code_line(range, blank, level, |p| {
            p.put(keyword);
            p.put(" (");
            p.opening_comments(open, predicate.span.start, true);
            p.root(predicate, end.len() + 1);
            // An end-of-line comment before the `)` goes after it, at the
            // end of the line.
            let deferred = p.closing_comments(predicate_end, close);
            p.put(")");
            p.put(end);
            if let Some(comment) = deferred {
                p.put(&comment);
            }
        });
    }

    /// Adds `text` to the last line.
    fn put(&mut self, text: &str) {
        self.lines
            .last_mut()
            .expect("a line is open")
            .push_str(text);
        self.column += text.chars().count();
    }

    /// Starts a continuation line at `level`, or prints `flat` instead when
    /// printing without breaks.
    fn break_line(&mut self, level: usize, flat: &str) {
        if self.flat {
            self.put(flat);
            return;
        }
        self.lines.push(self.indent.repeat(level));
        self.column = level * self.indent_columns;
        self.line_level = level;
    }

    /// Whether `width` more columns fit on the last line.
    fn fits(&self, width: usize) -> bool {
        self.flat || self.column + width <= self.width
    }

    /// The comments in `range`: `None` if there are none, `Some(width)` if
    /// they're all block comments on one line, which can go inline (`width`
    /// is the room they take), and `Some(usize::MAX)` otherwise.
    fn comments_in(&self, range: Range<usize>) -> Option<usize> {
        let first = self.comments.partition_point(|c| c.start < range.start);
        let mut width = None;
        for c in self.comments[first..]
            .iter()
            .take_while(|c| c.start < range.end)
        {
            let text = &self.src[c.clone()];
            if text.starts_with("//") || text.contains('\n') {
                return Some(usize::MAX);
            }
            width = Some(width.unwrap_or(0) + text.chars().count() + 1);
        }
        width
    }

    /// Prints the comments between two parts of a line, from `from` to `to`:
    /// block comments inline, after a space, and an end-of-line comment if
    /// `line_comments` allows it. Returns whether there was an end-of-line
    /// comment, after which the line must break.
    fn gap_comments(&mut self, from: usize, to: usize, line_comments: bool) -> bool {
        let mut must_break = false;
        while let Some(c) = self.comments.get(self.next_comment).cloned() {
            if c.start >= to {
                break;
            }
            self.next_comment += 1;
            let text = comment_text(self.src, &c);
            let line_comment = text.starts_with("//");
            if c.start < from
                || must_break
                || text.contains('\n')
                || (line_comment && !line_comments)
            {
                self.fail(c.start, "a comment here can't be placed");
                continue;
            }
            self.put(" ");
            self.put(&text);
            must_break |= line_comment;
        }
        must_break
    }

    /// Prints the comments from `from` to `to`, before a closing parenthesis
    /// that ends the line: block comments inline, and returns the last
    /// comment if it's an end-of-line comment, to print after the
    /// parenthesis.
    fn closing_comments(&mut self, from: usize, to: usize) -> Option<String> {
        let last = self.comments.partition_point(|c| c.start < to);
        let line_comment = last > self.next_comment
            && self.comments[last - 1].start >= from
            && self.src[self.comments[last - 1].clone()].starts_with("//");
        if !line_comment {
            self.gap_comments(from, to, false);
            return None;
        }
        let comment = self.comments[last - 1].clone();
        self.gap_comments(from, comment.start, false);
        self.next_comment += 1;
        Some(format!(" {}", comment_text(self.src, &comment)))
    }

    /// Prints the block comments from `from` to `to`, after an opening
    /// parenthesis, each followed by a space if `space`.
    fn opening_comments(&mut self, from: usize, to: usize, space: bool) {
        while let Some(c) = self.comments.get(self.next_comment).cloned() {
            if c.start >= to {
                break;
            }
            self.next_comment += 1;
            let text = comment_text(self.src, &c);
            if c.start < from || text.starts_with("//") || text.contains('\n') {
                self.fail(c.start, "a comment here can't be placed");
                continue;
            }
            self.put(&text);
            if space {
                self.put(" ");
            }
        }
    }

    /// Prints `e` on the current line, followed by `suffix` more columns,
    /// breaking it if it doesn't fit (L1-L6, and call chains). Lines it
    /// continues on are indented at least to `outer`.
    fn layout(&mut self, e: &AstNode<Expression>, suffix: usize, outer: usize) {
        let flat = self.expression(e);
        let width = flat.chars().count() + suffix;
        match self.comments_in(e.span.start..self.code_end(e.span.end)) {
            None if self.fits(width) => return self.put(&flat),
            Some(comments) if comments < usize::MAX && self.fits(width + comments) => {
                let was_flat = std::mem::replace(&mut self.flat, true);
                self.layout_parts(e, suffix, outer);
                self.flat = was_flat;
                return;
            }
            _ => {}
        }
        self.layout_parts(e, suffix, outer);
    }

    /// Prints `e` part by part, so that it can break between them.
    fn layout_parts(&mut self, e: &AstNode<Expression>, suffix: usize, outer: usize) {
        match &e.data {
            Expression::DyadicExpression { lhs, verb, rhs } if verb.is_assignment() => {
                // An assignment never breaks at its operator.
                let verb = verb.to_string();
                self.layout(lhs, verb.len() + 1, outer);
                self.put(" ");
                self.put(&verb);
                let end = self.code_end(lhs.span.end);
                self.gap_comments(end, rhs.span.start, false);
                self.put(" ");
                self.layout(rhs, suffix, outer);
            }
            Expression::DyadicExpression { lhs, verb, rhs } => {
                self.layout_operators(lhs, verb, rhs, suffix, outer)
            }
            Expression::Call { operand, arguments } => {
                if let Some((base, calls)) = chain(e) {
                    return self.layout_chain(base, &calls, suffix, outer);
                }
                self.layout(operand, 1, outer);
                self.put("(");
                let open = self.next_token(operand.span.end) + 1;
                self.layout_arguments(open, e.span.end - 1, arguments, suffix + 1, outer);
                self.put(")");
            }
            Expression::GlobalCall { name, arguments } => {
                self.put(&format!("global.{}(", name.string));
                let open = self.next_token(name.span.end) + 1;
                self.layout_arguments(open, e.span.end - 1, arguments, suffix + 1, outer);
                self.put(")");
            }
            Expression::MonadicPreExpression { operand, verb } => {
                let prefix = self.prefix(verb, operand);
                self.put(&prefix);
                self.layout(operand, suffix, outer);
            }
            Expression::MonadicPostExpression { operand, verb } => {
                let verb = monadic(verb);
                self.layout(operand, verb.len() + suffix, outer);
                self.put(&verb);
            }
            Expression::ArrayAccess { operand, argument } => {
                self.layout(operand, 1, outer);
                self.put("[");
                self.layout(argument, suffix + 1, outer);
                self.put("]");
            }
            Expression::MemberAccess { operand, target } => {
                let target = format!(".{}", target.string);
                self.layout(operand, target.len() + suffix, outer);
                self.put(&target);
            }
            Expression::DefaultAccess {
                operand: Some(operand),
                target,
            } => {
                let target = format!(".default.{}", target.string);
                self.layout(operand, target.len() + suffix, outer);
                self.put(&target);
            }
            Expression::StaticAccess {
                operand: Some(operand),
                target,
            } => {
                let target = format!(".static.{}", target.string);
                self.layout(operand, target.len() + suffix, outer);
                self.put(&target);
            }
            Expression::Cast { type_, operand } => {
                self.put(&format!("{}(", self.type_text(type_).unwrap_or_default()));
                self.layout(operand, suffix + 1, outer);
                self.put(")");
            }
            Expression::ParentheticalExpression(inner) => {
                self.put("(");
                // Lines inside the parentheses are indented one more level.
                let inner_outer = self.line_level.max(outer) + 1;
                self.layout(inner, suffix + 1, inner_outer);
                let end = self.code_end(inner.span.end);
                self.gap_comments(end, e.span.end - 1, false);
                self.put(")");
            }
            // Nothing else can break.
            _ => {
                let flat = self.expression(e);
                self.put(&flat);
            }
        }
    }

    /// Prints a chain of operators of the loosest precedence in `e`: on one
    /// line if it fits, otherwise broken after each operator with one operand
    /// per line (L3, L4), or for `$` and `@`, with as many operands per line
    /// as fit (L5).
    fn layout_operators(
        &mut self,
        lhs: &AstNode<Expression>,
        verb: &DyadicVerb,
        rhs: &AstNode<Expression>,
        suffix: usize,
        outer: usize,
    ) {
        let mut operands = Vec::new();
        let mut verbs = Vec::new();
        self.flatten_operators(lhs, verb, rhs, &mut operands, &mut verbs);
        let fill = verbs
            .iter()
            .all(|(v, _)| matches!(v, DyadicVerb::Concatenate | DyadicVerb::ConcatenateSpace));
        let level = self.line_level.max(outer.saturating_sub(1)) + 1;
        let mut must_break = false;
        for (i, operand) in operands.iter().enumerate() {
            let last = i == operands.len() - 1;
            let after = if last { suffix } else { verbs[i].1.len() + 1 };
            if i > 0 {
                let width = self.expression(operand).chars().count() + 1 + after;
                let room = fill
                    && self
                        .comments_in(operand.span.start..self.code_end(operand.span.end))
                        .is_none();
                if !must_break && (self.flat || (room && self.fits(width))) {
                    self.put(" ");
                } else {
                    self.break_line(level, " ");
                }
            }
            self.layout(operand, after, level);
            if !last {
                self.put(" ");
                self.put(&verbs[i].1);
                let end = self.code_end(operand.span.end);
                must_break = self.gap_comments(end, operands[i + 1].span.start, true);
            }
        }
    }

    /// Collects the operands and operators of the chain of operators at the
    /// precedence of `verb`, the operator between `lhs` and `rhs`.
    fn flatten_operators<'e>(
        &self,
        lhs: &'e AstNode<Expression>,
        verb: &'e DyadicVerb,
        rhs: &'e AstNode<Expression>,
        operands: &mut Vec<&'e AstNode<Expression>>,
        verbs: &mut Vec<(&'e DyadicVerb, String)>,
    ) {
        let precedence = crate::parser::precedence(verb);
        match &lhs.data {
            Expression::DyadicExpression {
                lhs: inner_lhs,
                verb: inner,
                rhs: inner_rhs,
            } if crate::parser::precedence(inner) == precedence => {
                self.flatten_operators(inner_lhs, inner, inner_rhs, operands, verbs)
            }
            _ => operands.push(lhs),
        }
        verbs.push((verb, self.verb_text(lhs, verb, rhs)));
        operands.push(rhs);
    }

    /// Prints a chain of calls, `base.A(...).B(...)`, on one line if it fits,
    /// otherwise with each call on a line of its own, indented one level.
    fn layout_chain(
        &mut self,
        base: &AstNode<Expression>,
        calls: &[Call],
        suffix: usize,
        outer: usize,
    ) {
        let level = self.line_level.max(outer.saturating_sub(1)) + 1;
        self.layout(base, 0, level);
        let mut end = self.code_end(base.span.end);
        for (i, call) in calls.iter().enumerate() {
            let dot = self.next_token(end);
            self.gap_comments(end, dot, true);
            self.break_line(level, "");
            self.put(&format!(".{}(", call.name.string));
            let open = self.next_token(call.name.span.end) + 1;
            let after = if i == calls.len() - 1 { suffix } else { 0 };
            self.layout_arguments(open, call.end - 1, call.arguments, after + 1, level + 1);
            self.put(")");
            end = call.end;
        }
    }

    /// Prints a call's arguments, between the `(` before `open` and the `)`
    /// at `close` (L2): on the call's line if they fit there with `suffix`
    /// more columns, and otherwise one per line, each laid out on its own,
    /// followed by a line for the `)`, all one level in from the call's line.
    fn layout_arguments(
        &mut self,
        open: usize,
        close: usize,
        arguments: &AstNode<ExpressionList>,
        suffix: usize,
        outer: usize,
    ) {
        let items: Vec<Item> = arguments
            .expressions
            .iter()
            .map(|slot| match slot {
                Some(e) => Item::Expression(e),
                None => Item::Empty,
            })
            .collect();
        let level = self.line_level.max(outer.saturating_sub(1)) + 1;
        let width: usize = arguments
            .expressions
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .map_or(0, |e| self.expression(e).chars().count())
                    + 2
            })
            .sum();
        let fits = match self.comments_in(open..close) {
            Some(usize::MAX) => false,
            comments => self.fits(width + comments.unwrap_or(0) + suffix),
        };
        if items.is_empty() || fits {
            self.layout_list(open, close, &items, suffix, level);
            return;
        }
        let mut cursor = open;
        for (i, item) in items.iter().enumerate() {
            let last = i + 1 == items.len();
            // An empty slot's comma stays on the line before.
            if let Item::Expression(e) = item {
                self.list_comments(cursor, e.span.start, level);
                self.break_line(level, "");
                self.layout(e, usize::from(!last), level);
                cursor = self.code_end(e.span.end);
            }
            if !last {
                self.put(",");
            }
        }
        self.list_comments(cursor, close, level);
        self.break_line(level, "");
    }

    /// Prints a function's parameters, from its `(` at `open` to its `)` at
    /// `close` in the source (R33): on the header's line if they fit there
    /// with `suffix` more columns, and otherwise one per line at `level`,
    /// followed by a line at `level` for the `)`.
    fn layout_parameters(
        &mut self,
        open: usize,
        close: usize,
        parameters: &[(Range<usize>, String)],
        suffix: usize,
        level: usize,
    ) {
        let width: usize = parameters.iter().map(|(_, t)| t.chars().count() + 2).sum();
        let fits = match self.comments_in(open..close) {
            Some(usize::MAX) => false,
            comments => self.fits(width + comments.unwrap_or(0) + suffix),
        };
        if parameters.is_empty() || fits {
            let items: Vec<Item> = parameters
                .iter()
                .map(|(range, text)| Item::Text(range.clone(), text.clone()))
                .collect();
            self.layout_list(open, close, &items, suffix, level);
            return;
        }
        let mut cursor = open;
        for (i, (range, text)) in parameters.iter().enumerate() {
            self.list_comments(cursor, range.start, level);
            self.break_line(level, "");
            self.put(text);
            if i + 1 < parameters.len() {
                self.put(",");
            }
            cursor = self.code_end(range.end);
        }
        self.list_comments(cursor, close, level);
        self.break_line(level, "");
    }

    /// Prints the comments from `from` to `to` between the items of a list
    /// laid out one per line: a comment on its own line in the source on its own line
    /// at `level`, and any other at the end of the line.
    fn list_comments(&mut self, from: usize, to: usize, level: usize) {
        while let Some(c) = self.comments.get(self.next_comment).cloned() {
            if c.start >= to {
                break;
            }
            self.next_comment += 1;
            if c.start < from {
                self.fail(c.start, "a comment here can't be placed");
                continue;
            }
            if self.src[self.prev_token_end(c.start)..c.start].contains('\n') {
                self.break_line(level, "");
            } else {
                self.put(" ");
            }
            self.put(&comment_text(self.src, &c));
        }
    }

    /// Prints a comma-separated list from `open` to `close` in the source,
    /// with as many items on each line as fit, and the lines after the first
    /// at `level` (L2b). Also prints arguments and parameters that fit on
    /// one line.
    fn layout_list(
        &mut self,
        open: usize,
        close: usize,
        items: &[Item],
        suffix: usize,
        level: usize,
    ) {
        let starts: Vec<Option<usize>> = items
            .iter()
            .map(|item| match item {
                Item::Expression(e) => Some(e.span.start),
                Item::Text(range, _) => Some(range.start),
                Item::Empty => None,
            })
            .collect();
        let ends: Vec<Option<usize>> = items
            .iter()
            .map(|item| match item {
                Item::Expression(e) => Some(self.code_end(e.span.end)),
                Item::Text(range, _) => Some(self.code_end(range.end)),
                Item::Empty => None,
            })
            .collect();
        let first = starts.iter().find_map(|s| *s).unwrap_or(close);
        self.opening_comments(open, first, first < close);
        let mut must_break = false;
        let mut cursor = open;
        for (i, item) in items.iter().enumerate() {
            let last = i == items.len() - 1;
            let after = if last { suffix } else { 1 };
            let flat = match item {
                Item::Expression(e) => Some((
                    self.expression(e),
                    self.comments_in(e.span.start..self.code_end(e.span.end)),
                )),
                Item::Text(_, text) => Some((text.clone(), None)),
                Item::Empty => None,
            };
            if i > 0
                && let Some((text, comments)) = flat
            {
                let width = text.chars().count();
                let fits_here = comments.is_none() && self.fits(width + 1 + after);
                // An item that won't fit on a line of its own (with its
                // comma) either stays here, and breaks inside.
                let comma = usize::from(!last);
                let fits_alone =
                    comments.is_none() && level * self.indent_columns + width + comma <= self.width;
                if !must_break && (self.flat || fits_here || !fits_alone) {
                    self.put(" ");
                } else {
                    self.break_line(level, " ");
                }
            }
            match item {
                Item::Expression(e) => self.layout(e, after, level),
                Item::Text(_, text) => self.put(text),
                Item::Empty => {}
            }
            if !last {
                self.put(",");
            }
            cursor = ends[i].unwrap_or(cursor);
            let next = starts[i + 1..].iter().find_map(|s| *s).unwrap_or(close);
            if ends[i].is_some() || last {
                must_break = self.gap_comments(cursor, next, !last);
            }
        }
    }

    // ---------------------------------------------------------------------
    // Expressions

    fn expression(&self, e: &AstNode<Expression>) -> String {
        match &e.data {
            Expression::Identifier(i) => keyword_identifier(&i.string),
            Expression::Literal(l) => self.literal(l),
            Expression::New { arguments, type_ } => {
                let mut text = "new".to_string();
                if let Some(arguments) = arguments {
                    text.push_str(&format!("({})", self.expression_list(arguments)));
                }
                text.push(' ');
                text.push_str(&self.expression(type_));
                text
            }
            Expression::MonadicPreExpression { operand, verb } => {
                format!("{}{}", self.prefix(verb, operand), self.expression(operand))
            }
            Expression::MonadicPostExpression { operand, verb } => {
                format!("{}{}", self.expression(operand), monadic(verb))
            }
            Expression::DyadicExpression { lhs, verb, rhs } => format!(
                "{} {} {}",
                self.expression(lhs),
                self.verb_text(lhs, verb, rhs),
                self.expression(rhs)
            ),
            Expression::Call { operand, arguments } => format!(
                "{}({})",
                self.expression(operand),
                self.expression_list(arguments)
            ),
            Expression::GlobalCall { name, arguments } => format!(
                "global.{}({})",
                name.string,
                self.expression_list(arguments)
            ),
            Expression::ArrayAccess { operand, argument } => format!(
                "{}[{}]",
                self.expression(operand),
                self.expression(argument)
            ),
            Expression::DefaultAccess { operand, target } => match operand {
                Some(o) => format!("{}.default.{}", self.expression(o), target.string),
                None => format!("default.{}", target.string),
            },
            Expression::StaticAccess { operand, target } => match operand {
                Some(o) => format!("{}.static.{}", self.expression(o), target.string),
                None => format!("static.{}", target.string),
            },
            Expression::MemberAccess { operand, target } => {
                format!("{}.{}", self.expression(operand), target.string)
            }
            Expression::Cast { type_, operand } => format!(
                "{}({})",
                self.type_text(type_).unwrap_or_default(),
                self.expression(operand)
            ),
            Expression::ParentheticalExpression(inner) => {
                format!("({})", self.expression(inner))
            }
        }
    }

    /// A prefix operator, with a space after it if it's needed so that
    /// `- -1` doesn't become `--1`.
    fn prefix(&self, verb: &MonadicVerb, operand: &AstNode<Expression>) -> String {
        let verb = monadic(verb);
        let last = verb.chars().last();
        let operand = self.expression(operand);
        if last.is_some_and(|c| (c == '-' || c == '+') && operand.starts_with(c)) {
            format!("{verb} ")
        } else {
            verb
        }
    }

    /// An operator's text: named operators (`Dot`, `Cross`,
    /// `ClockwiseFrom`) as written, the others as symbols.
    fn verb_text(
        &self,
        lhs: &AstNode<Expression>,
        verb: &DyadicVerb,
        rhs: &AstNode<Expression>,
    ) -> String {
        match verb {
            DyadicVerb::Dot | DyadicVerb::Cross | DyadicVerb::ClockwiseFrom => {
                let start = self.next_token(lhs.span.end);
                let end = self.prev_token_end(rhs.span.start);
                self.text(start..end).to_string()
            }
            verb => verb.to_string(),
        }
    }

    fn expression_list(&self, list: &AstNode<ExpressionList>) -> String {
        let mut text = String::new();
        for (i, slot) in list.expressions.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            if let Some(e) = slot {
                if i > 0 {
                    text.push(' ');
                }
                text.push_str(&self.expression(e));
            }
        }
        text
    }

    fn literal(&self, l: &AstNode<Literal>) -> String {
        match &l.data {
            Literal::None => "none".to_string(),
            Literal::Boolean(b) => b.to_string(),
            Literal::Numeric(n) => self.number(n),
            Literal::String(_) | Literal::Name(_) => {
                self.text(l.span.start..l.span.end).to_string()
            }
            Literal::Vector(v) | Literal::Rotator(v) => {
                let keyword = if matches!(l.data, Literal::Vector(_)) {
                    "vect"
                } else {
                    "rot"
                };
                let numbers: Vec<String> = v.iter().map(|n| self.number(n)).collect();
                format!("{keyword}({})", numbers.join(", "))
            }
            Literal::Object { type_, .. } => {
                let text = self.text(l.span.start..l.span.end);
                let quote = text.find('\'').expect("an object literal has a quote");
                let type_ = if type_.string.eq_ignore_ascii_case("class") {
                    "Class".to_string()
                } else if type_.string.eq_ignore_ascii_case("enum") {
                    "Enum".to_string()
                } else {
                    type_.string.clone()
                };
                format!("{type_}{}", &text[quote..])
            }
        }
    }

    fn number(&self, n: &AstNode<NumericLiteral>) -> String {
        number_text(self.text(n.span.start..n.span.end))
    }
}

/// An item of a list.
enum Item<'e> {
    Expression(&'e AstNode<Expression>),
    /// An empty slot in an argument list, as in `F(A,,B)`.
    Empty,
    /// Text printed as it is, for the source in the range.
    Text(Range<usize>, String),
}

/// A call in a chain of calls: `.Name(arguments)`, ending at `end`.
struct Call<'e> {
    name: &'e AstNode<crate::ast::Identifier>,
    arguments: &'e AstNode<ExpressionList>,
    end: usize,
}

/// If `e` is a chain of two or more calls, `base.A(...).B(...)`, its base
/// and calls, in order.
fn chain(e: &AstNode<Expression>) -> Option<(&AstNode<Expression>, Vec<Call<'_>>)> {
    let mut calls = Vec::new();
    let mut current = e;
    while let Expression::Call { operand, arguments } = &current.data
        && let Expression::MemberAccess {
            operand: inner,
            target,
        } = &operand.data
    {
        calls.push(Call {
            name: target,
            arguments,
            end: current.span.end,
        });
        current = inner;
    }
    calls.reverse();
    (calls.len() >= 2).then_some((current, calls))
}

/// A comment's text, without trailing whitespace on any of its lines.
fn comment_text(src: &str, c: &Range<usize>) -> String {
    src[c.clone()]
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Identifiers that are keywords are lower case (R45).
fn keyword_identifier(name: &str) -> String {
    if name.eq_ignore_ascii_case("self") || name.eq_ignore_ascii_case("super") {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

fn monadic(verb: &MonadicVerb) -> String {
    verb.to_string()
}

/// Sorts modifiers: the access modifier first, then the others
/// alphabetically (R32).
fn sort_modifiers<T>(modifiers: &mut [T], name: impl Fn(&T) -> &str) {
    modifiers.sort_by_key(|m| {
        let n = name(m).to_ascii_lowercase();
        let n = n.split('(').next().unwrap_or_default().to_string();
        (!matches!(n.as_str(), "private" | "protected" | "public"), n)
    });
}

/// A number as written, except that a float has digits on both sides of its
/// point (R49), a hex number's prefix is `0x` (R50), and a malformed float is
/// cut to the part that's read (`0.0.5` is `0.0`).
pub(super) fn number_text(text: &str) -> String {
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'-' | b'+') => text.split_at(1),
        _ => ("", text),
    };
    let rest = match rest.match_indices('.').nth(1) {
        Some((i, _)) => &rest[..i],
        None => rest,
    };
    if rest.len() > 1 && rest[..2].eq_ignore_ascii_case("0x") {
        return format!("{sign}0x{}", &rest[2..]);
    }
    let mut out = String::from(sign);
    if rest.starts_with('.') {
        out.push('0');
    }
    match rest.find('.') {
        Some(i) => {
            out.push_str(&rest[..=i]);
            let after = &rest[i + 1..];
            if !after.starts_with(|c: char| c.is_ascii_digit()) {
                out.push('0');
            }
            out.push_str(after);
        }
        None => out.push_str(rest),
    }
    out
}
