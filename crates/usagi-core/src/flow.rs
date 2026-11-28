//! A small dataflow pass over each function body, run while the symbol table
//! is built. It follows, at each point of the function:
//!
//! - which locals may have been assigned, and the calls they were given to
//!   that may have assigned them (an `out` argument), for C019
//!   `unassigned-object-local`;
//! - which locals and arguments are known to be `None`, from a comparison
//!   with `None` or an assignment of it, for C021 `known-none-access`.
//!
//! The pass knows the file and nothing else, so a call that a local is given
//! to may assign it (an `out` argument) or not. For C019 it records
//! such calls with the read, and the project lint looks their declarations
//! up: the read is a problem only if none of them can assign the local. The
//! other lints treat such a call as assigning what it's given.
//!
//! A condition leaves one state for when it's true and one for when it's
//! false, so that `if (P == None)` knows `P` is `None` in its body, and
//! `&&` and `||` are followed as they short-circuit. Paths join: what may
//! have happened by union, what is known by intersection. Loops are run until
//! their state stops changing. Code after a `return`, `break` or `continue`
//! is unreachable, and nothing there is recorded. A function with `goto` or a
//! label isn't looked at.

use crate::ast::*;
use crate::lint::Lint;
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

/// A problem the pass finds by itself, without other classes.
#[derive(Debug, Clone)]
pub struct FlowProblem {
    pub lint: Lint,
    pub message: String,
    pub span: AstSpan,
}

/// What the pass finds in a function.
#[derive(Debug, Default)]
pub(crate) struct Findings {
    pub reads: Vec<UnassignedRead>,
    pub problems: Vec<FlowProblem>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct State {
    reachable: bool,
    /// The variables an assignment may have reached.
    assigned: BTreeSet<SymbolId>,
    /// The calls that may have assigned each of them.
    pending: BTreeMap<SymbolId, BTreeSet<usize>>,
    /// The locals and arguments known to be `None`.
    none: BTreeSet<SymbolId>,
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
        for (symbol, calls) in &other.pending {
            self.pending
                .entry(*symbol)
                .or_default()
                .extend(calls.iter().copied());
        }
        self.none.retain(|symbol| other.none.contains(symbol));
    }

    fn joined(mut self, other: &State) -> State {
        self.join(other);
        self
    }

    /// Forgets what's known of `symbol`'s value, when something may change it.
    fn forget(&mut self, symbol: SymbolId) {
        self.none.remove(&symbol);
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
    /// The latest problem at each use of a variable, by its start,
    /// likewise.
    problems: BTreeMap<usize, Option<FlowProblem>>,
}

/// What the pass finds in `body`, the body of the function whose scope is
/// `scope`.
pub(crate) fn analyze(table: &SymbolTable, scope: ScopeId, body: &FunctionBody) -> Findings {
    let statements: Vec<&AstNode<CodeStatement>> = body
        .statements
        .iter()
        .filter_map(|s| match s {
            FunctionBodyStatement::CodeStatement(s) => Some(s),
            _ => None,
        })
        .collect();
    if statements.iter().any(|s| has_jump(s)) {
        return Findings::default();
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
        problems: BTreeMap::new(),
    };
    for statement in statements {
        flow.statement(statement);
    }
    Findings {
        reads: flow.reads.into_values().flatten().collect(),
        problems: flow.problems.into_values().flatten().collect(),
    }
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

fn is_none_literal(expression: &AstNode<Expression>) -> bool {
    matches!(&unparenthesize(expression).data,
        Expression::Literal(literal) if matches!(literal.data, Literal::None))
}

impl Flow<'_> {
    /// The local, argument or class variable that `expression` names, if it's
    /// a bare name of one.
    fn variable(&self, expression: &AstNode<Expression>) -> Option<SymbolId> {
        let Expression::Identifier(name) = &unparenthesize(expression).data else {
            return None;
        };
        let symbol = self.table.lookup(self.scope, Namespace::Value, name)?;
        matches!(
            self.table.symbol(symbol).kind,
            SymbolKind::Local { .. } | SymbolKind::Argument { .. } | SymbolKind::Variable { .. }
        )
        .then_some(symbol)
    }

    fn kind(&self, symbol: SymbolId) -> &SymbolKind {
        &self.table.symbol(symbol).kind
    }

    /// Whether the reads of `symbol` that no assignment reaches are recorded:
    /// a local that may hold an object.
    fn is_tracked_local(&self, symbol: SymbolId) -> bool {
        !self.arrays.contains(&symbol)
            && matches!(
                self.kind(symbol),
                SymbolKind::Local {
                    type_: TypeRef::Named(_) | TypeRef::Class(_),
                    ..
                }
            )
    }

    /// Whether `None` facts are kept for `symbol`: a local or an argument,
    /// which nothing but the function itself changes.
    fn is_local_or_argument(&self, symbol: SymbolId) -> bool {
        matches!(
            self.kind(symbol),
            SymbolKind::Local { .. } | SymbolKind::Argument { .. }
        )
    }

    fn read(&mut self, symbol: SymbolId, span: AstSpan, extra: Option<usize>) {
        if !self.state.reachable || !self.is_tracked_local(symbol) {
            return;
        }
        let finding = (!self.state.assigned.contains(&symbol)).then(|| {
            let ids = self.state.pending.get(&symbol).into_iter().flatten();
            UnassignedRead {
                local: symbol,
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

    /// `symbol` is assigned something other than `None`.
    fn assign(&mut self, symbol: SymbolId) {
        self.state.assigned.insert(symbol);
        self.state.forget(symbol);
    }

    /// The object in `operand` is used (`operand.X`, `operand.default.X`):
    /// reports it if it's known to be `None`.
    fn dereference(&mut self, operand: &AstNode<Expression>) {
        self.expression(operand);
        let Some(symbol) = self.variable(operand) else {
            return;
        };
        if !self.state.reachable {
            return;
        }
        let name = self.table.symbol(symbol).name.as_str();
        let span = unparenthesize(operand).span;
        // A use doesn't change what's known to be `None`, so each use is
        // reported.
        let problem = self.state.none.contains(&symbol).then(|| FlowProblem {
            lint: Lint::KnownNoneAccess,
            message: format!("`{}` is `None` here, so using it is an Accessed None", name),
            span,
        });
        self.problems.insert(span.start, problem);
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
                        if let Some(symbol) = self.variable(argument) {
                            self.assign(symbol);
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
                    let (yes, no) = flow.condition(&w.predicate);
                    exit.join(&no);
                    flow.state = yes;
                    flow.body(&w.body);
                });
            }
            CodeStatement::For(f) => {
                if let Some(init) = &f.init {
                    self.expression(init);
                }
                self.state = self.run_loop(|flow, exit| {
                    if let Some(predicate) = &f.predicate {
                        let (yes, no) = flow.condition(predicate);
                        exit.join(&no);
                        flow.state = yes;
                    }
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
                    match &d.predicate {
                        // `until` leaves the loop when it's true.
                        Some(predicate) => {
                            let (yes, no) = flow.condition(predicate);
                            exit.join(&yes);
                            flow.state = no;
                        }
                        None => exit.join(&flow.state),
                    }
                });
            }
            CodeStatement::Conditional(c) => {
                let (yes, mut otherwise) = self.condition(&c.if_statement.predicate);
                self.state = yes;
                if let Some(body) = &c.if_statement.body {
                    self.body(body);
                }
                let mut after = std::mem::take(&mut self.state);
                for elif in &c.elif_statements {
                    self.state = otherwise;
                    let (yes, no) = self.condition(&elif.predicate);
                    otherwise = no;
                    self.state = yes;
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

    /// The variable `expression` compares with `None`, whether it's `==`
    /// (rather than `!=`), and the two sides.
    #[allow(clippy::type_complexity)]
    fn none_comparison<'e>(
        &self,
        expression: &'e AstNode<Expression>,
    ) -> Option<(
        SymbolId,
        bool,
        &'e AstNode<Expression>,
        &'e AstNode<Expression>,
    )> {
        let Expression::DyadicExpression { lhs, verb, rhs } = &unparenthesize(expression).data
        else {
            return None;
        };
        let equal = match verb {
            DyadicVerb::CompareEqual => true,
            DyadicVerb::CompareNotEqual => false,
            _ => return None,
        };
        let variable = if is_none_literal(rhs) {
            lhs
        } else if is_none_literal(lhs) {
            rhs
        } else {
            return None;
        };
        Some((self.variable(variable)?, equal, lhs, rhs))
    }

    /// Whether `expression` is followed by [`Self::condition`]: it's a
    /// logical operation, or compares something with `None`.
    fn is_logical(&self, expression: &AstNode<Expression>) -> bool {
        match &unparenthesize(expression).data {
            Expression::MonadicPreExpression {
                verb: MonadicVerb::Not,
                ..
            } => true,
            Expression::DyadicExpression {
                verb: DyadicVerb::LogicalAnd | DyadicVerb::LogicalOr,
                ..
            } => true,
            _ => self.none_comparison(expression).is_some(),
        }
    }

    /// Runs `expression` as a condition, from the current state: the states
    /// when it's true and when it's false.
    fn condition(&mut self, expression: &AstNode<Expression>) -> (State, State) {
        let expression = unparenthesize(expression);
        match &expression.data {
            Expression::MonadicPreExpression {
                verb: MonadicVerb::Not,
                operand,
            } => {
                let (yes, no) = self.condition(operand);
                (no, yes)
            }
            Expression::DyadicExpression {
                lhs,
                verb: DyadicVerb::LogicalAnd,
                rhs,
            } => {
                let (yes, no) = self.condition(lhs);
                self.state = yes;
                let (both, not_rhs) = self.condition(rhs);
                (both, no.joined(&not_rhs))
            }
            Expression::DyadicExpression {
                lhs,
                verb: DyadicVerb::LogicalOr,
                rhs,
            } => {
                let (yes, no) = self.condition(lhs);
                self.state = no;
                let (rhs_yes, neither) = self.condition(rhs);
                (yes.joined(&rhs_yes), neither)
            }
            _ => match self.none_comparison(expression) {
                Some((symbol, equal, lhs, rhs)) => {
                    self.expression(lhs);
                    self.expression(rhs);
                    let mut is_none = self.state.clone();
                    let mut is_not = self.state.clone();
                    if self.is_local_or_argument(symbol) && is_none.reachable {
                        is_none.none.insert(symbol);
                    }
                    is_not.none.remove(&symbol);
                    if equal {
                        (is_none, is_not)
                    } else {
                        (is_not, is_none)
                    }
                }
                None => {
                    self.evaluate(expression);
                    (self.state.clone(), self.state.clone())
                }
            },
        }
    }

    fn expression(&mut self, expression: &AstNode<Expression>) {
        if self.is_logical(expression) {
            let (yes, no) = self.condition(expression);
            self.state = yes.joined(&no);
        } else {
            self.evaluate(expression);
        }
    }

    fn evaluate(&mut self, expression: &AstNode<Expression>) {
        match &expression.data {
            Expression::Identifier(_) => {
                if let Some(symbol) = self.variable(expression) {
                    self.read(symbol, expression.span, None);
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
            Expression::MonadicPreExpression { operand, verb }
            | Expression::MonadicPostExpression { operand, verb } => {
                self.expression(operand);
                if matches!(verb, MonadicVerb::Increment | MonadicVerb::Decrement) {
                    if let Some(symbol) = self.variable(operand) {
                        self.assign(symbol);
                    }
                }
            }
            Expression::DyadicExpression { lhs, verb, rhs } => {
                if !verb.is_assignment() {
                    self.expression(lhs);
                    self.expression(rhs);
                    return;
                }
                match self.variable(lhs) {
                    // The value is worked out before it's assigned.
                    Some(symbol) => {
                        self.expression(rhs);
                        self.assign(symbol);
                        if *verb == DyadicVerb::Assign
                            && self.state.reachable
                            && is_none_literal(rhs)
                            && self.is_local_or_argument(symbol)
                        {
                            self.state.none.insert(symbol);
                        }
                    }
                    // `X.Y = ...` uses `X`.
                    None => {
                        self.expression(lhs);
                        self.expression(rhs);
                    }
                }
            }
            Expression::Call { operand, arguments } => {
                // The callee is a name, or an object's member (which uses the
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
                        self.dereference(operand);
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
                    self.dereference(operand);
                }
            }
            Expression::MemberAccess { operand, .. } => self.dereference(operand),
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
            match self.variable(argument) {
                Some(symbol) => {
                    let span = unparenthesize(argument).span;
                    let id = self.call_id(span.start, callee, i);
                    self.read(symbol, span, Some(id));
                    given.push((symbol, id));
                }
                None => self.expression(argument),
            }
        }
        for (symbol, id) in given {
            self.state.pending.entry(symbol).or_default().insert(id);
            self.state.forget(symbol);
        }
    }
}
