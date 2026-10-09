//! Advisory performance hints. They never affect the verdict or the executed SQL.
use crate::{rewrite, walk, Dialect, Fix, FixAction, GuardPolicy, Suggestion};
use sqlparser::ast::{
    visit_expressions, BinaryOperator, Expr, FromTable, LimitClause, OrderByKind, Query,
    SelectItem, SetExpr, Statement, TableWithJoins, Value,
};
use std::ops::ControlFlow;

/// Rows a sample query fetches when the user accepts the `add_limit` fix.
const SAMPLE_ROWS: u64 = 100;
/// OFFSET at which keyset pagination clearly beats skipping rows.
const LARGE_OFFSET: u64 = 10_000;
/// IN-list length at which parse and plan time become noticeable.
const LARGE_IN_LIST: usize = 500;

#[derive(Default)]
struct Hints(Vec<Suggestion>);

impl Hints {
    fn add(&mut self, code: &str, message: impl Into<String>, fix: Option<Fix>) {
        if !self.0.iter().any(|s| s.code == code) {
            self.0.push(Suggestion {
                code: code.to_owned(),
                message: message.into(),
                fix,
            });
        }
    }
}

/// `renderable` is false when re-rendering the AST would lose syntax the
/// parser cannot represent (e.g. restored lock clauses); fixes are then omitted.
pub(crate) fn collect(
    statement: &Statement,
    dialect: Dialect,
    policy: &GuardPolicy,
    renderable: bool,
) -> Vec<Suggestion> {
    let mut hints = Hints::default();
    let render = |sql: String| {
        if dialect == Dialect::MySql {
            crate::mysql::escape_rendered(&sql)
        } else {
            sql
        }
    };
    match statement {
        Statement::Query(query) => {
            query_hints(query, policy, &mut hints);
            if renderable && hints.0.iter().any(|s| s.code == "add_limit") {
                let limit = SAMPLE_ROWS.min(policy.max_rows);
                let mut sample = statement.clone();
                if let Statement::Query(query) = &mut sample {
                    query.limit_clause = Some(LimitClause::LimitOffset {
                        limit: Some(Expr::Value(Value::Number(limit.to_string(), false).into())),
                        offset: None,
                        limit_by: vec![],
                    });
                }
                if let Some(hint) = hints.0.iter_mut().find(|s| s.code == "add_limit") {
                    hint.fix = Some(Fix {
                        label: format!("Add LIMIT {limit}"),
                        sql: render(sample.to_string()),
                        action: FixAction::Replace,
                    });
                }
            }
        }
        Statement::Update(update) if update.from.is_none() => {
            if let Some(selection) = &update.selection {
                where_hints(selection, &mut hints);
                if renderable {
                    preview(&update.table, selection, &render, &mut hints);
                }
            }
        }
        Statement::Delete(delete) if delete.using.is_none() && delete.tables.is_empty() => {
            if let (
                FromTable::WithFromKeyword(from) | FromTable::WithoutKeyword(from),
                Some(selection),
            ) = (&delete.from, &delete.selection)
            {
                where_hints(selection, &mut hints);
                if let ([table], true) = (from.as_slice(), renderable) {
                    preview(table, selection, &render, &mut hints);
                }
            }
        }
        _ => {}
    }
    expression_hints(statement, &mut hints);
    hints.0
}

fn query_hints(query: &Query, policy: &GuardPolicy, hints: &mut Hints) {
    if let SetExpr::Select(select) = query.body.as_ref() {
        if select.projection.iter().any(|item| {
            matches!(
                item,
                SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..)
            )
        }) {
            hints.add("select_star", "SELECT * reads every column, including wide ones you may not need. List only the columns you use so less data is read, sent, and masked.", None);
        }
        if let Some(selection) = &select.selection {
            where_hints(selection, hints);
        }
        if !rewrite::has_limit(query)
            && select.selection.is_none()
            && !select.from.is_empty()
            && !walk::aggregate_read(&query.body)
            && policy.max_rows > SAMPLE_ROWS
        {
            hints.add("add_limit", format!("This reads the whole table without a filter. The gateway stops at {} rows, but a small LIMIT returns a sample faster and keeps the result easy to scan.", policy.max_rows), None);
        }
    }
    if let Some(order_by) = &query.order_by {
        if let OrderByKind::Expressions(exprs) = &order_by.kind {
            if exprs.iter().any(|o| is_random(&o.expr)) {
                hints.add("order_by_random", "ORDER BY random() sorts every matching row before returning any. On large tables, sample with TABLESAMPLE (Postgres) or filter by a random key range instead.", None);
            }
        }
    }
    let offset = match &query.limit_clause {
        Some(LimitClause::LimitOffset {
            offset: Some(offset),
            ..
        }) => rewrite::number(&offset.value),
        Some(LimitClause::OffsetCommaLimit { offset, .. }) => rewrite::number(offset),
        _ => None,
    };
    if offset.is_some_and(|n| n >= LARGE_OFFSET) {
        hints.add("large_offset", "A large OFFSET still reads and discards every skipped row. Page with a WHERE on the last seen key (keyset pagination) instead.", None);
    }
}

/// Non-sargable comparisons: a column wrapped in a function or cast.
fn where_hints(selection: &Expr, hints: &mut Hints) {
    let _ = visit_expressions(selection, |expr| {
        if let Expr::BinaryOp { left, op, right } = expr {
            if matches!(
                op,
                BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
            ) {
                for (side, other) in [(left, right), (right, left)] {
                    if let Some(wrapper) = wrapped_column(side) {
                        if !has_column(other) {
                            hints.add("function_on_column", format!("{wrapper} wraps a column in the WHERE clause, so an index on that column cannot be used. Compare the bare column (for dates, use a range) or add an expression index."), None);
                        }
                    }
                }
            }
        }
        ControlFlow::<()>::Continue(())
    });
}

fn expression_hints(statement: &Statement, hints: &mut Hints) {
    let _ = visit_expressions(statement, |expr| {
        match expr {
            Expr::Like { pattern, .. } | Expr::ILike { pattern, .. }
                if leading_wildcard(pattern) =>
            {
                hints.add("leading_wildcard", "A LIKE pattern that starts with % cannot use a normal index and scans every row. Anchor the pattern at the start, or use a trigram/full-text index.", None);
            }
            Expr::InSubquery { negated: true, .. } => {
                hints.add("not_in_subquery", "NOT IN (subquery) returns no rows if the subquery yields a NULL, and often plans poorly. Use NOT EXISTS (...) instead.", None);
            }
            Expr::InList { list, .. } if list.len() >= LARGE_IN_LIST => {
                hints.add("large_in_list", format!("This IN list has {} values, which is slow to parse and plan. Join against a temporary table or pass an array instead.", list.len()), None);
            }
            _ => {}
        }
        ControlFlow::<()>::Continue(())
    });
}

fn preview(
    table: &TableWithJoins,
    selection: &Expr,
    render: &impl Fn(String) -> String,
    hints: &mut Hints,
) {
    if !table.joins.is_empty() {
        return;
    }
    hints.add(
        "preview_write",
        "Check how many rows this write will touch before running it.",
        Some(Fix {
            label: "Preview affected rows".to_owned(),
            sql: render(format!(
                "SELECT COUNT(*) FROM {} WHERE {selection}",
                table.relation
            )),
            action: FixAction::NewTab,
        }),
    );
}

fn leading_wildcard(pattern: &Expr) -> bool {
    match pattern {
        Expr::Value(v) => matches!(
            &v.value,
            Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) if s.starts_with('%')
        ),
        Expr::Nested(inner) => leading_wildcard(inner),
        _ => false,
    }
}

fn is_random(expr: &Expr) -> bool {
    match expr {
        Expr::Function(f) => {
            let name = f.name.to_string().to_lowercase();
            matches!(name.as_str(), "random" | "rand")
        }
        Expr::Nested(inner) => is_random(inner),
        _ => false,
    }
}

/// The function or cast text when `expr` wraps at least one column.
fn wrapped_column(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Function(f) if has_column(expr) => Some(format!("{}()", f.name)),
        Expr::Cast { expr: inner, .. } if has_column(inner) => Some("A cast".to_owned()),
        Expr::Nested(inner) => wrapped_column(inner),
        _ => None,
    }
}

fn has_column(expr: &Expr) -> bool {
    visit_expressions(expr, |e| {
        if matches!(e, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}
