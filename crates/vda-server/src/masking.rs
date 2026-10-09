//! Column masking based on column names and referenced relations.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A result column, including whether its values were masked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub masked: bool,
}
/// Case-insensitive glob matching with linear memory and bounded pattern size.
pub fn glob(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let value = value.to_lowercase();
    let mut previous: Vec<bool> = std::iter::once(true)
        .chain(std::iter::repeat_n(false, value.len()))
        .collect();
    for p in pattern.bytes() {
        let mut current = Vec::with_capacity(value.len() + 1);
        current.push(p == b'*' && previous.first().copied().unwrap_or(false));
        for ((diagonal, above), v) in previous
            .iter()
            .zip(previous.iter().skip(1))
            .zip(value.bytes())
        {
            current.push(if p == b'*' {
                *above || current.last().copied().unwrap_or(false)
            } else {
                (p == b'?' || p == v) && *diagonal
            });
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(false)
}
/// Table-qualified patterns apply only if that table appears in the analysis.
pub fn matches_column(pattern: &str, column: &str, tables: &[String]) -> bool {
    match pattern.rsplit_once('.') {
        None => glob(pattern, column),
        Some((table, col)) => {
            glob(col, column)
                && tables.iter().any(|name| {
                    glob(table, name) || glob(table, name.rsplit('.').next().unwrap_or(name))
                })
        }
    }
}
/// Mask every matching cell and annotate the corresponding columns.
pub fn apply(
    result: &mut vda_connectors::QueryResult,
    patterns: &[String],
    tables: &[String],
) -> Vec<Column> {
    result
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let masked = patterns
                .iter()
                .any(|p| matches_column(p, &column.name, tables));
            if masked {
                for row in &mut result.rows {
                    if let Some(value) = row.get_mut(index) {
                        *value = Value::String("••••••".into());
                    }
                }
            }
            Column {
                name: column.name.clone(),
                type_name: column.type_name.clone(),
                masked,
            }
        })
        .collect()
}

/// Whether any configured pattern could cover a relation touched by this analysis.
pub fn touches_masked(analysis: &vda_guard::Analysis, patterns: &[String]) -> bool {
    analysis
        .statements
        .iter()
        .flat_map(|s| &s.tables)
        .any(|name| {
            patterns
                .iter()
                .any(|pattern| match pattern.rsplit_once('.') {
                    None => true,
                    Some((table, _)) => {
                        glob(table, name) || glob(table, name.rsplit('.').next().unwrap_or(name))
                    }
                })
        })
}

/// Reject row serialization that destroys the column names required for masking.
pub fn harden_analysis(
    analysis: &mut vda_guard::Analysis,
    patterns: &[String],
    sql: &str,
    dialect: vda_guard::Dialect,
) {
    if !touches_masked(analysis, patterns) {
        return;
    }
    let function = analysis
        .statements
        .iter()
        .flat_map(|s| &s.functions)
        .any(|name| {
            matches!(
                name.rsplit('.').next().unwrap_or(name),
                "row_to_json"
                    | "to_json"
                    | "to_jsonb"
                    | "json_agg"
                    | "jsonb_agg"
                    | "json_build_object"
                    | "jsonb_build_object"
                    | "row"
            )
        });
    let tables: Vec<String> = analysis
        .statements
        .iter()
        .flat_map(|s| s.tables.iter().cloned())
        .collect();
    let (code, message) = if function || whole_rows(sql, dialect) {
        (
            "masked_data_serialization",
            "Row serialization is prohibited when the query touches masked data",
        )
    } else if renames_masked(sql, dialect, patterns, &tables) {
        (
            "masked_data_transformation",
            "Masked columns must be selected by their own name, without aliases or expressions",
        )
    } else {
        return;
    };
    let issue = vda_guard::Issue {
        severity: vda_guard::Severity::Block,
        code: code.into(),
        message: message.into(),
    };
    analysis.issues.push(issue.clone());
    for statement in &mut analysis.statements {
        statement.issues.push(issue.clone());
    }
    analysis.verdict = vda_guard::Verdict::Deny;
    analysis.rewritten_sql = None;
}

/// Items a DML statement returns to the caller; empty for everything else.
fn returning(statement: &sqlparser::ast::Statement) -> &[sqlparser::ast::SelectItem] {
    use sqlparser::ast::Statement;
    match statement {
        Statement::Insert(insert) => insert.returning.as_deref(),
        Statement::Update(update) => update.returning.as_deref(),
        Statement::Delete(delete) => delete.returning.as_deref(),
        _ => None,
    }
    .unwrap_or_default()
}

fn is_masked_name(patterns: &[String], name: &str, tables: &[String]) -> bool {
    patterns.iter().any(|p| matches_column(p, name, tables))
}

/// Masking matches result column names, so a masked column that is aliased, wrapped in an
/// expression or renamed positionally would leave the driver-reported name unmasked.
fn renames_masked(
    sql: &str,
    dialect: vda_guard::Dialect,
    patterns: &[String],
    tables: &[String],
) -> bool {
    use sqlparser::{
        ast::{Expr, Query, Select, SelectItem, SetExpr, Statement, TableFactor, Visit, Visitor},
        parser::Parser,
    };
    use std::ops::ControlFlow;
    let postgres = dialect == vda_guard::Dialect::Postgres;
    let dialect: &dyn sqlparser::dialect::Dialect = match dialect {
        vda_guard::Dialect::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        vda_guard::Dialect::MySql => &sqlparser::dialect::MySqlDialect {},
    };
    let Ok(statements) = Parser::parse_sql(dialect, sql) else {
        return false;
    };
    /// Breaks when an expression mentions a masked column anywhere, including subqueries.
    struct Mentions<'a>(&'a [String], &'a [String]);
    impl Visitor for Mentions<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            let name = match expr {
                Expr::Identifier(i) => Some(i.value.as_str()),
                Expr::CompoundIdentifier(ids) => ids.last().map(|i| i.value.as_str()),
                _ => None,
            };
            if name.is_some_and(|n| is_masked_name(self.0, n, self.1)) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }
    /// The output name a result column carries, when the SQL fixes it. PostgreSQL keeps the
    /// column name through casts and parentheses; MySQL names such columns after the expression.
    fn output_name(item: &SelectItem, postgres: bool) -> Option<String> {
        fn column(expr: &Expr, postgres: bool) -> Option<String> {
            match expr {
                Expr::Identifier(i) => Some(i.value.clone()),
                Expr::CompoundIdentifier(ids) => ids.last().map(|i| i.value.clone()),
                Expr::Cast { expr, .. } | Expr::Nested(expr) if postgres => column(expr, postgres),
                _ => None,
            }
        }
        match item {
            SelectItem::UnnamedExpr(expr) => column(expr, postgres),
            SelectItem::ExprWithAlias { alias, .. } => Some(alias.value.clone()),
            _ => None,
        }
    }
    fn selects(body: &SetExpr) -> Vec<&Select> {
        match body {
            SetExpr::Select(select) => vec![select.as_ref()],
            SetExpr::SetOperation { left, right, .. } => {
                let mut all = selects(left);
                all.extend(selects(right));
                all
            }
            SetExpr::Query(query) => selects(&query.body),
            _ => Vec::new(),
        }
    }
    struct Renames<'a> {
        patterns: &'a [String],
        tables: &'a [String],
        postgres: bool,
    }
    impl Visitor for Renames<'_> {
        type Break = ();
        /// RETURNING is a projection too: a masked column must keep its own name there.
        fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
            for item in returning(statement) {
                let expr = match item {
                    SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => e,
                    _ => continue,
                };
                if expr
                    .visit(&mut Mentions(self.patterns, self.tables))
                    .is_break()
                    && !output_name(item, self.postgres)
                        .is_some_and(|n| is_masked_name(self.patterns, &n, self.tables))
                {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        }
        fn pre_visit_table_factor(&mut self, table: &TableFactor) -> ControlFlow<()> {
            let columns = match table {
                TableFactor::Table { alias, .. } | TableFactor::Derived { alias, .. } => {
                    alias.as_ref().map(|a| a.columns.len())
                }
                _ => None,
            };
            if columns.unwrap_or(0) > 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
        fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
            if query
                .with
                .as_ref()
                .is_some_and(|with| with.cte_tables.iter().any(|c| !c.alias.columns.is_empty()))
            {
                return ControlFlow::Break(());
            }
            let all = selects(&query.body);
            let first: Vec<Option<String>> = all
                .first()
                .map(|s| {
                    s.projection
                        .iter()
                        .map(|i| output_name(i, self.postgres))
                        .collect()
                })
                .unwrap_or_default();
            let first_has_wildcard = all.first().is_some_and(|s| {
                s.projection.iter().any(|i| {
                    matches!(
                        i,
                        SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..)
                    )
                })
            });
            for (position, select) in all.iter().enumerate() {
                for (index, item) in select.projection.iter().enumerate() {
                    let expr = match item {
                        SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => e,
                        _ => continue,
                    };
                    if expr
                        .visit(&mut Mentions(self.patterns, self.tables))
                        .is_continue()
                    {
                        continue;
                    }
                    // Later set-operation branches take their names from the first branch.
                    let out = if position == 0 {
                        output_name(item, self.postgres)
                    } else if first_has_wildcard {
                        None
                    } else {
                        first.get(index).cloned().flatten()
                    };
                    if !out.is_some_and(|n| is_masked_name(self.patterns, &n, self.tables)) {
                        return ControlFlow::Break(());
                    }
                }
            }
            ControlFlow::Continue(())
        }
    }
    statements
        .visit(&mut Renames {
            patterns,
            tables,
            postgres,
        })
        .is_break()
}

fn whole_rows(sql: &str, dialect: vda_guard::Dialect) -> bool {
    use sqlparser::{
        ast::{
            Expr, Select, SelectItem, SetExpr, Statement, TableFactor, TableObject, Visit, Visitor,
        },
        parser::Parser,
    };
    use std::{collections::BTreeSet, ops::ControlFlow};
    let dialect: &dyn sqlparser::dialect::Dialect = match dialect {
        vda_guard::Dialect::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        vda_guard::Dialect::MySql => &sqlparser::dialect::MySqlDialect {},
    };
    let Ok(statements) = Parser::parse_sql(dialect, sql) else {
        return true;
    };
    #[derive(Default)]
    struct Relations(BTreeSet<String>);
    impl Relations {
        fn add(&mut self, name: &sqlparser::ast::ObjectName) {
            let name = name.to_string().replace(['"', '`'], "").to_lowercase();
            self.0
                .insert(name.rsplit('.').next().unwrap_or(&name).into());
            self.0.insert(name);
        }
    }
    impl Visitor for Relations {
        type Break = ();
        // INSERT targets are not table factors, but RETURNING can still name them.
        fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
            if let Statement::Insert(insert) = statement {
                if let TableObject::TableName(name) = &insert.table {
                    self.add(name);
                }
                if let Some(alias) = &insert.table_alias {
                    self.0.insert(alias.alias.value.to_lowercase());
                }
            }
            ControlFlow::Continue(())
        }
        fn pre_visit_table_factor(&mut self, table: &TableFactor) -> ControlFlow<()> {
            match table {
                TableFactor::Table { name, alias, .. } => {
                    self.add(name);
                    if let Some(alias) = alias {
                        self.0.insert(alias.name.value.to_lowercase());
                    }
                }
                TableFactor::Derived {
                    alias: Some(alias), ..
                } => {
                    self.0.insert(alias.name.value.to_lowercase());
                }
                _ => {}
            }
            ControlFlow::Continue(())
        }
    }
    /// Flags row constructors and bare relation references inside one projection.
    /// Names that are projection aliases (e.g. `count(*) AS orders`) are columns, not rows.
    struct Rows<'a> {
        relations: &'a BTreeSet<String>,
        aliases: &'a BTreeSet<String>,
    }
    impl Visitor for Rows<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            let is_relation =
                |name: String| !self.aliases.contains(&name) && self.relations.contains(&name);
            let whole = match expr {
                Expr::Tuple(_) | Expr::Struct { .. } => true,
                Expr::Identifier(i) => is_relation(i.value.to_lowercase()),
                Expr::CompoundIdentifier(ids) => is_relation(
                    ids.iter()
                        .map(|i| i.value.to_lowercase())
                        .collect::<Vec<_>>()
                        .join("."),
                ),
                _ => false,
            };
            if whole {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }
    /// Only projections return data, so only they can serialize masked rows.
    struct Projections(BTreeSet<String>);
    impl Projections {
        fn check(&self, projection: &[SelectItem]) -> ControlFlow<()> {
            let aliases = projection
                .iter()
                .filter_map(|item| match item {
                    SelectItem::ExprWithAlias { alias, .. } => Some(alias.value.to_lowercase()),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let mut rows = Rows {
                relations: &self.0,
                aliases: &aliases,
            };
            for item in projection {
                item.visit(&mut rows)?;
            }
            ControlFlow::Continue(())
        }
    }
    impl Visitor for Projections {
        type Break = ();
        fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> ControlFlow<()> {
            for select in selects(&query.body) {
                self.check(&select.projection)?;
            }
            ControlFlow::Continue(())
        }
        fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
            self.check(returning(statement))
        }
    }
    fn selects(body: &SetExpr) -> Vec<&Select> {
        match body {
            SetExpr::Select(select) => vec![select.as_ref()],
            SetExpr::SetOperation { left, right, .. } => {
                let mut all = selects(left);
                all.extend(selects(right));
                all
            }
            _ => Vec::new(),
        }
    }
    let mut relations = Relations::default();
    let _ = statements.visit(&mut relations);
    statements.visit(&mut Projections(relations.0)).is_break()
}
