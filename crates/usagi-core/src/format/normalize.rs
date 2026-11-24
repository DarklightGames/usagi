//! Undoing the changes the formatter is allowed to make, so that a program and
//! its formatted version compare equal.

use crate::ast::{
    AstNode, AstSpan, CodeBlock, CodeStatement, CodeStatementOrBlock, FunctionBodyStatement,
    FunctionDeclaration, Program, ProgramStatement, StateStatement, StructDeclaration, Type,
};
use std::rc::Rc;

/// Rewrites `program` into a canonical form that ignores the differences the
/// formatter may introduce:
///
/// - Modifiers are sorted (R32), since their order means nothing.
/// - The body of an `if`, `else`, `for`, `foreach`, `while` or `do` is always a
///   block (R14b): a single statement becomes a block holding it, and an `if`
///   with no body gets an empty one.
/// - Empty statements and declarations (stray `;`) are removed (R17b).
///
/// Spans aren't changed, and are ignored when comparing anyway.
///
/// # Panics
///
/// If a `var` declaration is shared, i.e. a clone of its `Rc` is still alive.
pub fn normalize(program: &mut Program) {
    program
        .statements
        .retain(|s| !matches!(s.data, ProgramStatement::Empty));
    for statement in &mut program.statements {
        match &mut statement.data {
            ProgramStatement::ClassDeclaration(class) => {
                class
                    .data
                    .modifiers
                    .sort_by_key(|m| m.data.type_.data.to_string());
            }
            ProgramStatement::VarDeclaration(var) => {
                let var = Rc::get_mut(var).expect("var declarations aren't shared");
                var.data.modifiers.sort_by_key(|m| m.data);
                normalize_type(&mut var.data.type_);
            }
            ProgramStatement::StructDeclaration(s) => normalize_struct(&mut s.data),
            ProgramStatement::FunctionDeclaration(function) => {
                normalize_function(&mut function.data)
            }
            ProgramStatement::StateDeclaration(state) => {
                state.modifiers.sort_by_key(|m| m.to_string());
                for statement in &mut state.statements {
                    if let StateStatement::FunctionDeclaration(function) = statement {
                        normalize_function(&mut function.data);
                    }
                }
                for label in &mut state.labels {
                    normalize_statements(&mut label.statements);
                }
            }
            _ => {}
        }
    }
}

fn normalize_type(type_: &mut Type) {
    match type_ {
        Type::Struct(s) => normalize_struct(&mut s.data),
        Type::Array(inner) => normalize_type(inner),
        _ => {}
    }
}

fn normalize_struct(s: &mut StructDeclaration) {
    s.modifiers.sort_by_key(|m| m.data.to_string());
    for member in &mut s.members {
        member.data.modifiers.sort_by_key(|m| m.to_string());
        normalize_type(&mut member.data.type_);
    }
}

fn normalize_function(function: &mut FunctionDeclaration) {
    function
        .modifiers
        .sort_by(|a, b| a.data.type_.cmp(&b.data.type_));
    for argument in &mut function.arguments {
        argument.data.modifiers.sort_by_key(|m| m.to_string());
    }
    if let Some(body) = &mut function.body {
        body.data.statements.retain(|s| {
            !matches!(s, FunctionBodyStatement::CodeStatement(c) if matches!(c.data, CodeStatement::Empty))
        });
        for statement in &mut body.data.statements {
            if let FunctionBodyStatement::CodeStatement(c) = statement {
                normalize_statement(&mut c.data);
            }
        }
    }
}

fn normalize_statements(statements: &mut Vec<AstNode<CodeStatement>>) {
    statements.retain(|s| !matches!(s.data, CodeStatement::Empty));
    for statement in statements {
        normalize_statement(&mut statement.data);
    }
}

fn normalize_statement(statement: &mut CodeStatement) {
    match statement {
        CodeStatement::ForEach(f) => normalize_body(&mut f.body),
        CodeStatement::For(f) => normalize_body(&mut f.body),
        CodeStatement::While(w) => normalize_body(&mut w.body),
        CodeStatement::DoUntil(d) => normalize_body(&mut d.body),
        CodeStatement::Switch(s) => {
            for case in &mut s.cases {
                normalize_statements(&mut case.statements);
            }
        }
        CodeStatement::Conditional(c) => {
            let if_ = &mut c.if_statement;
            let span = if_.predicate.span;
            normalize_body(if_.body.get_or_insert_with(|| block(span, Vec::new())));
            for elif in &mut c.elif_statements {
                normalize_body(&mut elif.body);
            }
            if let Some(else_) = &mut c.else_statement {
                normalize_body(&mut else_.body);
            }
        }
        _ => {}
    }
}

/// Makes `body` a block, and normalizes its statements.
fn normalize_body(body: &mut AstNode<CodeStatementOrBlock>) {
    let empty = CodeStatementOrBlock::CodeBlock(AstNode {
        span: body.span,
        data: CodeBlock {
            statements: Vec::new(),
        },
    });
    let mut block = match std::mem::replace(&mut body.data, empty) {
        CodeStatementOrBlock::CodeBlock(block) => block,
        CodeStatementOrBlock::CodeStatement(statement) => AstNode {
            span: statement.span,
            data: CodeBlock {
                statements: vec![statement],
            },
        },
    };
    normalize_statements(&mut block.data.statements);
    body.data = CodeStatementOrBlock::CodeBlock(block);
}

fn block(span: AstSpan, statements: Vec<AstNode<CodeStatement>>) -> AstNode<CodeStatementOrBlock> {
    AstNode {
        span,
        data: CodeStatementOrBlock::CodeBlock(AstNode {
            span,
            data: CodeBlock { statements },
        }),
    }
}
