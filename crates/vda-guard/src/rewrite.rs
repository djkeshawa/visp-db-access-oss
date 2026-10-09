//! Enforce the outer result cap while retaining the original offset.
use sqlparser::ast::{Expr, LimitClause, Query, Statement, Value};

pub(crate) fn has_limit(query: &Query) -> bool {
    query.fetch.is_some()
        || match &query.limit_clause {
            Some(LimitClause::OffsetCommaLimit { .. }) => true,
            Some(LimitClause::LimitOffset { limit, .. }) => limit.is_some(),
            None => false,
        }
}

pub(crate) fn number(expr: &Expr) -> Option<u64> {
    match expr {
        Expr::Value(v) => match &v.value {
            Value::Number(n, _) => n.parse().ok(),
            _ => None,
        },
        Expr::Nested(inner) => number(inner),
        _ => None,
    }
}

pub(crate) fn cap(statement: &Statement, max_rows: u64) -> String {
    let mut statement = statement.clone();
    if let Statement::Query(query) = &mut statement {
        let sentinel =
            Expr::Value(Value::Number((u128::from(max_rows) + 1).to_string(), false).into());
        if let Some(fetch) = &mut query.fetch {
            let quantity = fetch.quantity.as_ref().map_or(Some(1), number);
            if fetch.percent || fetch.with_ties || quantity.is_none_or(|n| n > max_rows) {
                fetch.quantity = Some(sentinel.clone());
                fetch.percent = false;
                fetch.with_ties = false;
            }
        }
        match &mut query.limit_clause {
            Some(LimitClause::OffsetCommaLimit { limit, .. }) => {
                if number(limit).is_none_or(|n| n > max_rows) {
                    *limit = sentinel;
                }
            }
            Some(LimitClause::LimitOffset { limit, .. }) => {
                if (limit.is_some() || query.fetch.is_none())
                    && limit.as_ref().and_then(number).is_none_or(|n| n > max_rows)
                {
                    *limit = Some(sentinel);
                }
            }
            None if query.fetch.is_none() => {
                query.limit_clause = Some(LimitClause::LimitOffset {
                    limit: Some(sentinel),
                    offset: None,
                    limit_by: vec![],
                });
            }
            None => {}
        }
    }
    statement.to_string()
}
