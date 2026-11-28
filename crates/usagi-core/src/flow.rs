//! Which locals may have been assigned at each point of a function body: a
//! small dataflow pass over the AST, run while the symbol table is built, that
//! finds the reads of a local that no assignment can reach (C019
//! `unassigned-object-local`).
//!
//! The pass knows the file and nothing else, so a call that a local is given
//! to may assign it (an `out` argument) or not. It records such calls with the
//! read, and the project lint looks their declarations up: the read is a
//! problem only if none of them can assign the local.
//!
//! The state at each point is, for each local, whether an assignment may have
//! reached it, and the calls that may have assigned it. Paths join by union,
//! and loops are run until their state stops growing. Code after a `return`,
//! `break` or `continue` is unreachable, and nothing there is recorded. A
//! function with `goto` or a label isn't looked at.

use crate::ast::*;
use crate::symbols::{Namespace, Operand, ScopeId, SymbolId, SymbolKind, SymbolTable, TypeRef};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A read of a local that no assignment reaches, unless one of `calls`
/// assigns it.
#[derive(Debug, Clone)]
pub struct UnassignedRead {
    pub local: SymbolId,
    pub span: AstSpan,
    /// The scope of the function, to look the callees' names up in.
    pub scope: ScopeId,
    /// The calls the local was given to before, that may have assigned it:
    /// the function called (a name or a member, `Obj.F`) and the argument's
    /// position, from 0. A read that is itself an argument includes its call.
    pub calls: Vec<(Operand, usize)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct State {
    reachable: bool,
    /// The locals an assignment may have reached.
    assigned: BTreeSet<SymbolId>,
    /// The calls that may have assigned each local.
    pending: BTreeMap<SymbolId, BTreeSet<usize>>,
}

impl State {
    fn entry() -> State {
        State {
            reachable: true,
            ..State::default()
        }
    }

    /// Code that control can't reach.
    fn unreachable() -> State {
        State::default()
    }

    fn join(&mut self, other: &State) {
        if !other.reachable {
            return;
        }
        if !self.reachable {
            *self = other.clone();
            return;
        }
        self.assigned.extend(other.assigned.iter().copied());
        for (local, calls) in &other.pending {
            self.pending
                .entry(*local)
                .or_default()
                .extend(calls.iter().copied());
        }
    }

    fn joined(mut self, other: &State) -> State {
        self.join(other);
        self
    }
}

/// Where `break` and `continue` go, from a loop or a `switch`.
#[derive(Default)]
struct Jumps {
    breaks: State,
    continues: State,
    /// Whether it's a loop: `continue` in a `switch` goes to the loop around
    /// it.
    is_loop: bool,
}

struct Flow<'t> {
    table: &'t SymbolTable,
    scope: ScopeId,
    /// The locals that are static arrays.
    arrays: BTreeSet<SymbolId>,
    state: State,
    jumps: Vec<Jumps>,
    /// Every call a local was given to, as `(callee, argument)`.
    calls: Vec<(Operand, usize)>,
    /// The id of each call in `calls`, by the start of the argument.
    call_ids: HashMap<usize, usize>,
    /// The latest finding at each read, by its start: the last visit is the
    /// one with the most complete state.
    reads: BTreeMap<usize, Option<UnassignedRead>>,
}

/// The reads of a local in `body`, the body of the function whose scope is
/// `scope`, that no assignment reaches. Only locals that may hold an object
/// are tracked: those of a named type or `class<...>`, and not arrays.
pub(crate) fn unassigned_reads(
    table: &SymbolTable,
    scope: ScopeId,
    body: &FunctionBody,
) -> Vec<UnassignedRead> {
    let statements: Vec<&AstNode<CodeStatement>> = body
        .statements
        .iter()
        .filter_map(|s| match s {
            FunctionBodyStatement::CodeStatement(s) => Some(s),
            _ => None,
        })
        .collect();
    if statements.iter().any(|s| has_jump(s)) {
        return Vec::new();
    }
    // A static array's size may be a const the file doesn't declare, which
    // leaves it unknown in the table.
    let arrays = body
        .statements
        .iter()
        .filter_map(|s| match s {
            FunctionBodyStatement::LocalDeclaration(l) => Some(&l.names),
            _ => None,
        })
        .flatten()
        .filter(|name| name.size.is_some())
        .filter_map(|name| table.lookup(scope, Namespace::Value, &name.identifier))
        .collect();
    let mut flow = Flow {
        table,
        scope,
        arrays,
        state: State::entry(),
        jumps: Vec::new(),
        calls: Vec::new(),
        call_ids: HashMap::new(),
        reads: BTreeMap::new(),
    };
    for statement in statements {
        flow.statement(statement);
    }
    flow.reads.into_values().flatten().collect()
}

/// Whether there's a `goto` or a label in `statement`.
fn has_jump(statement: &AstNode<CodeStatement>) -> bool {
    match &statement.data {
        CodeStatement::Goto { .. } | CodeStatement::JumpLabel(_) => true,
        CodeStatement::Conditional(c) => c
            .if_statement
            .body
            .iter()
            .chain(c.elif_statements.iter().map(|e| &e.body))
            .chain(c.else_statement.iter().map(|e| &e.body))
            .any(body_has_jump),
        CodeStatement::ForEach(f) => body_has_jump(&f.body),
        CodeStatement::For(f) => body_has_jump(&f.body),
        CodeStatement::While(w) => body_has_jump(&w.body),
        CodeStatement::DoUntil(d) => body_has_jump(&d.body),
        CodeStatement::Switch(s) => s.cases.iter().flat_map(|c| &c.statements).any(has_jump),
        _ => false,
    }
}

fn body_has_jump(body: &AstNode<CodeStatementOrBlock>) -> bool {
    match &body.data {
        CodeStatementOrBlock::CodeStatement(s) => has_jump(s),
        CodeStatementOrBlock::CodeBlock(b) => b.statements.iter().any(has_jump),
    }
}

fn unparenthesize(mut expression: &AstNode<Expression>) -> &AstNode<Expression> {
    while let Expression::ParentheticalExpression(inner) = &expression.data {
        expression = inner;
    }
    expression
}

impl Flow<'_> {
    /// The local that `expression` names, if it's a bare name of a tracked
    /// local.
    fn local(&self, expression: &AstNode<Expression>) -> Option<SymbolId> {
        let Expression::Identifier(name) = &unparenthesize(expression).data else {
            return None;
        };
        let symbol = self.table.lookup(self.scope, Namespace::Value, name)?;
        match &self.table.symbol(symbol).kind {
            SymbolKind::Local {
                type_: TypeRef::Named(_) | TypeRef::Class(_),
                ..
            } if !self.arrays.contains(&symbol) => Some(symbol),
            _ => None,
        }
    }

    fn read(&mut self, local: SymbolId, span: AstSpan, extra: Option<usize>) {
        if !self.state.reachable {
            return;
        }
        let finding = (!self.state.assigned.contains(&local)).then(|| {
            let ids = self.state.pending.get(&local).into_iter().flatten();
            UnassignedRead {
                local,
                span,
                scope: self.scope,
                calls: ids
                    .chain(extra.as_ref())
                    .map(|&id| self.calls[id].clone())
                    .collect(),
            }
        });
        self.reads.insert(span.start, finding);
    }

    fn assign(&mut self, local: SymbolId) {
        self.state.assigned.insert(local);
    }

    /// The id of the call giving the argument that starts at `start`, its
    /// `argument`th.
    fn call_id(&mut self, start: usize, callee: &Operand, argument: usize) -> usize {
        *self.call_ids.entry(start).or_insert_with(|| {
            self.calls.push((callee.clone(), argument));
            self.calls.len() - 1
        })
    }

    fn body(&mut self, body: &AstNode<CodeStatementOrBlock>) {
        match &body.data {
            CodeStatementOrBlock::CodeStatement(s) => self.statement(s),
            CodeStatementOrBlock::CodeBlock(b) => {
                for s in &b.statements {
                    self.statement(s);
                }
            }
        }
    }

    /// Runs a loop: `step` runs one pass from the state at the loop's head,
    /// leaving the state that goes back to the head. Returns the state that
    /// leaves the loop by its condition (`step` sets it in `exit`), joined
    /// with the `break`s.
    fn run_loop(&mut self, mut step: impl FnMut(&mut Self, &mut State)) -> State {
        let entry = std::mem::replace(&mut self.state, State::unreachable());
        let mut head = entry.clone();
        loop {
            self.jumps.push(Jumps {
                is_loop: true,
                ..Jumps::default()
            });
            self.state = head.clone();
            let mut exit = State::unreachable();
            step(self, &mut exit);
            let jumps = self.jumps.pop().expect("pushed above");
            let next = entry.clone().joined(&self.state).joined(&jumps.continues);
            if next == head {
                return exit.joined(&jumps.breaks);
            }
            head = head.joined(&next);
        }
    }

    fn statement(&mut self, statement: &AstNode<CodeStatement>) {
        match &statement.data {
            CodeStatement::Empty
            | CodeStatement::CompilerDirective(_)
            | CodeStatement::ConstDeclaration(_)
            | CodeStatement::Goto { .. }
            | CodeStatement::JumpLabel(_) => {}
            CodeStatement::Expression(e) => self.expression(e),
            CodeStatement::Return(e) => {
                if let Some(e) = e {
                    self.expression(e);
                }
                self.state = State::unreachable();
            }
            CodeStatement::Break => {
                let state = std::mem::replace(&mut self.state, State::unreachable());
                if let Some(jumps) = self.jumps.last_mut() {
                    jumps.breaks.join(&state);
                }
            }
            CodeStatement::Continue => {
                let state = std::mem::replace(&mut self.state, State::unreachable());
                if let Some(jumps) = self.jumps.iter_mut().rev().find(|j| j.is_loop) {
                    jumps.continues.join(&state);
                }
            }
            CodeStatement::ForEach(f) => {
                self.expression(&f.predicate);
                // An iterator fills in the variables it's given.
                if let Expression::Call { arguments, .. } = &unparenthesize(&f.predicate).data {
                    for argument in arguments.expressions.iter().flatten() {
                        if let Some(local) = self.local(argument) {
                            self.assign(local);
                        }
                    }
                }
                self.state = self.run_loop(|flow, exit| {
                    exit.join(&flow.state);
                    flow.body(&f.body);
                });
            }
            CodeStatement::While(w) => {
                self.state = self.run_loop(|flow, exit| {
                    flow.expression(&w.predicate);
                    exit.join(&flow.state);
                    flow.body(&w.body);
                });
            }
            CodeStatement::For(f) => {
                if let Some(init) = &f.init {
                    self.expression(init);
                }
                self.state = self.run_loop(|flow, exit| {
                    if let Some(predicate) = &f.predicate {
                        flow.expression(predicate);
                    }
                    exit.join(&flow.state);
                    flow.body(&f.body);
                    let continues = flow.jumps.last().expect("in a loop").continues.clone();
                    flow.state.join(&continues);
                    if let Some(post) = &f.post {
                        flow.expression(post);
                    }
                });
            }
            CodeStatement::DoUntil(d) => {
                self.state = self.run_loop(|flow, exit| {
                    flow.body(&d.body);
                    let continues = flow.jumps.last().expect("in a loop").continues.clone();
                    flow.state.join(&continues);
                    if let Some(predicate) = &d.predicate {
                        flow.expression(predicate);
                    }
                    exit.join(&flow.state);
                });
            }
            CodeStatement::Conditional(c) => {
                self.expression(&c.if_statement.predicate);
                let mut after = State::unreachable();
                let mut otherwise = self.state.clone();
                if let Some(body) = &c.if_statement.body {
                    self.body(body);
                }
                after.join(&self.state);
                for elif in &c.elif_statements {
                    self.state = otherwise;
                    self.expression(&elif.predicate);
                    otherwise = self.state.clone();
                    self.body(&elif.body);
                    after.join(&self.state);
                }
                self.state = otherwise;
                if let Some(e) = &c.else_statement {
                    self.body(&e.body);
                }
                after.join(&self.state);
                self.state = after;
            }
            CodeStatement::Switch(s) => {
                self.expression(&s.predicate);
                let subject = self.state.clone();
                self.jumps.push(Jumps::default());
                // Each case is entered from the subject, or by falling through.
                self.state = State::unreachable();
                for case in &s.cases {
                    self.state.join(&subject);
                    if let SwitchCaseType::Expression(label) = &case.type_ {
                        self.expression(label);
                    }
                    for statement in &case.statements {
                        self.statement(statement);
                    }
                }
                let jumps = self.jumps.pop().expect("pushed above");
                self.state.join(&jumps.breaks);
                if !s
                    .cases
                    .iter()
                    .any(|c| matches!(c.type_, SwitchCaseType::Default))
                {
                    self.state.join(&subject);
                }
            }
        }
    }

    fn expression(&mut self, expression: &AstNode<Expression>) {
        match &expression.data {
            Expression::Identifier(_) => {
                if let Some(local) = self.local(expression) {
                    self.read(local, expression.span, None);
                }
            }
            Expression::Literal(_) => {}
            Expression::New { arguments, type_ } => {
                if let Some(arguments) = arguments {
                    for argument in arguments.expressions.iter().flatten() {
                        self.expression(argument);
                    }
                }
                self.expression(type_);
            }
            Expression::MonadicPreExpression { operand, .. }
            | Expression::MonadicPostExpression { operand, .. } => self.expression(operand),
            Expression::DyadicExpression { lhs, verb, rhs } => {
                match self.local(lhs).filter(|_| verb.is_assignment()) {
                    // The value is worked out before it's assigned.
                    Some(local) => {
                        self.expression(rhs);
                        self.assign(local);
                    }
                    None => {
                        self.expression(lhs);
                        self.expression(rhs);
                    }
                }
            }
            Expression::Call { operand, arguments } => {
                // The callee is a name, or an object's member (which reads the
                // object).
                let callee = unparenthesize(operand);
                let named = match &callee.data {
                    Expression::Identifier(_) => Operand::of(&callee.data),
                    // `static.F()` is the class's own `F`.
                    Expression::StaticAccess {
                        operand: None,
                        target,
                    } => Operand::Name(target.data.clone()),
                    Expression::MemberAccess { operand, .. } => {
                        self.expression(operand);
                        Operand::of(&callee.data)
                    }
                    _ => {
                        self.expression(callee);
                        Operand::of(&callee.data)
                    }
                };
                self.call_arguments(&named, arguments);
            }
            Expression::GlobalCall { name, arguments } => {
                self.call_arguments(&Operand::Name(name.data.clone()), arguments);
            }
            Expression::ArrayAccess { operand, argument } => {
                self.expression(operand);
                self.expression(argument);
            }
            Expression::DefaultAccess { operand, .. }
            | Expression::StaticAccess { operand, .. } => {
                if let Some(operand) = operand {
                    self.expression(operand);
                }
            }
            Expression::MemberAccess { operand, .. } => self.expression(operand),
            Expression::Cast { operand, .. } => self.expression(operand),
            Expression::ParentheticalExpression(inner) => self.expression(inner),
        }
    }

    /// A call's arguments. A local given as an argument is read unless the
    /// argument is `out`, and is assigned afterwards if it is.
    fn call_arguments(&mut self, callee: &Operand, arguments: &AstNode<ExpressionList>) {
        let mut given = Vec::new();
        for (i, argument) in arguments.expressions.iter().enumerate() {
            let Some(argument) = argument else { continue };
            match self.local(argument) {
                Some(local) => {
                    let span = unparenthesize(argument).span;
                    let id = self.call_id(span.start, callee, i);
                    self.read(local, span, Some(id));
                    given.push((local, id));
                }
                None => self.expression(argument),
            }
        }
        for (local, id) in given {
            self.state.pending.entry(local).or_default().insert(id);
        }
    }
}
