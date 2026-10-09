//! Assemble parsing, structural analysis, policy, and executable SQL.
use crate::{
    classify, hints, parse, policy, rewrite, syntax, walk, AccessLevel, Analysis, Dialect,
    GuardPolicy, Issue, Risk, Severity, StatementAnalysis, StatementKind, Verdict,
};
use sqlparser::ast::Statement;

const MAX_QUERY_BYTES: usize = 1024 * 1024;

pub(crate) fn analyze(
    sql: &str,
    dialect: Dialect,
    level: AccessLevel,
    policy: &GuardPolicy,
) -> Analysis {
    if sql.len() > MAX_QUERY_BYTES {
        return denied(
            "query_too_large",
            "SQL exceeds the 1 MiB input limit. Submit a smaller query.",
        );
    }
    let parsed = match parse::parse(sql, dialect) {
        Ok(parsed) => parsed,
        Err(parse::ParseFailure::ExecutableComment) => {
            return denied(
                "executable_comment",
                "Executable MySQL/MariaDB comments are blocked. Write SQL explicitly.",
            )
        }
        Err(parse::ParseFailure::OptimizerHint) => {
            return denied(
                "optimizer_hint",
                "Optimizer hints can override execution limits and are blocked.",
            )
        }
        Err(parse::ParseFailure::FileWrite(message)) => {
            let mut analysis = denied("parse_error", &format!("SQL could not be parsed: {message}. File destinations are not supported by the gateway."));
            analysis.risk = Risk::Critical;
            analysis.issues.push(Issue::new(Severity::Block, "file_write", "SELECT INTO OUTFILE/DUMPFILE writes files on the database server and is blocked at every access level. Remove the file destination."));
            return analysis;
        }
        Err(error) => {
            return denied(
                "parse_error",
                &format!("SQL could not be parsed: {error}. Correct the syntax before executing."),
            )
        }
    };
    if parsed.statements.is_empty() {
        return denied(
            "empty_query",
            "No SQL statement was provided. Enter a query to analyze.",
        );
    }
    let mut analysis = Analysis {
        verdict: Verdict::Allow,
        risk: Risk::Low,
        statements: vec![],
        rewritten_sql: None,
        issues: vec![],
        suggestions: vec![],
    };
    let mut locks = parsed.lock_syntax.iter().copied();
    for statement in &parsed.statements {
        let (mut detail, verdict) = statement_analysis(statement, dialect, level, policy);
        detail.sql = syntax::restore(&detail.sql, dialect, &mut locks);
        analysis.verdict = policy::worst(analysis.verdict, verdict);
        analysis.risk = analysis.risk.max(detail.risk);
        analysis.issues.extend(detail.issues.iter().cloned());
        analysis.statements.push(detail);
    }
    if parsed.statements.len() > 1 {
        analysis.verdict = Verdict::Deny;
        analysis.issues.push(Issue::new(Severity::Block, "multiple_statements", "The gateway executes exactly one statement at a time. Submit each statement separately."));
    }
    if analysis.verdict != Verdict::Deny {
        if let Some(statement) = parsed.statements.first() {
            let select = analysis
                .statements
                .first()
                .is_some_and(|s| s.kind == StatementKind::Select);
            analysis.rewritten_sql = Some(if select && matches!(statement, Statement::Query(_)) {
                syntax::restore(
                    &rewrite::cap(statement, policy.max_rows),
                    dialect,
                    &mut parsed.lock_syntax.iter().copied(),
                )
            } else {
                sql.trim().trim_end_matches(';').trim_end().to_owned()
            });
        }
    }
    if analysis.verdict != Verdict::Deny {
        if let [statement] = parsed.statements.as_slice() {
            analysis.suggestions =
                hints::collect(statement, dialect, policy, parsed.lock_syntax.is_empty());
        }
    }
    if dialect == Dialect::MySql {
        if let Some(sql) = &mut analysis.rewritten_sql {
            if analysis
                .statements
                .first()
                .is_some_and(|s| s.kind == StatementKind::Select)
            {
                *sql = crate::mysql::escape_rendered(sql);
            }
        }
    }
    analysis
}

pub(crate) fn denied(code: &str, message: &str) -> Analysis {
    Analysis {
        verdict: Verdict::Deny,
        risk: Risk::Low,
        statements: vec![],
        rewritten_sql: None,
        issues: vec![Issue::new(Severity::Block, code, message)],
        suggestions: vec![],
    }
}

fn statement_analysis(
    statement: &Statement,
    dialect: Dialect,
    level: AccessLevel,
    policy: &GuardPolicy,
) -> (StatementAnalysis, Verdict) {
    let facts = walk::collect(statement, dialect);
    let mut issues = facts.issues.clone();
    let kind = effective_kind(statement, &facts, &mut issues);
    let has_limit = original_limit(statement);
    let mut detail = StatementAnalysis {
        kind,
        sql: statement.to_string(),
        tables: facts.tables.iter().cloned().collect(),
        functions: facts.functions.iter().cloned().collect(),
        risk: risk(statement, kind, &facts, has_limit),
        issues,
        has_where: facts.has_where,
        has_limit,
    };
    if facts.cross_join {
        detail.issues.push(Issue::new(Severity::Warning, "cross_join", "This query contains a cross join and may produce a Cartesian product. Add a join condition or tightly bound both inputs."));
    }
    // SELECT INTO may coexist with a modifying CTE; both write and DDL gates apply.
    if kind == StatementKind::Ddl && !facts.writes.is_empty() {
        if level == AccessLevel::Read {
            detail.issues.push(Issue::new(
                Severity::Block,
                "insufficient_access",
                "A nested write requires Write or Admin access. Request a write grant.",
            ));
        }
        if !policy.allow_writes {
            detail.issues.push(Issue::new(
                Severity::Block,
                "writes_disabled",
                "The query contains a nested write, but writes are disabled by cluster policy.",
            ));
        }
    }
    let verdict = policy::apply(&mut detail, level, policy, facts.locking);
    (detail, verdict)
}

fn effective_kind(
    statement: &Statement,
    facts: &walk::Facts,
    issues: &mut Vec<Issue>,
) -> StatementKind {
    let base = classify::base(statement);
    if let Statement::Explain {
        statement: inner, ..
    } = statement
    {
        if classify::explain_executes(statement) {
            let kind = effective_kind(inner, facts, issues);
            if classify::is_write(kind) || kind == StatementKind::Ddl {
                issues.push(Issue::new(Severity::Warning, "explain_analyze_write", "EXPLAIN ANALYZE executes the enclosed write. It requires the same access, safety checks, and approval as executing the write directly."));
                return kind;
            }
            issues.push(Issue::new(Severity::Warning, "explain_analyze_executes", "EXPLAIN ANALYZE executes the query and can consume production resources. Use EXPLAIN without ANALYZE when only a plan is needed."));
            if !matches!(
                kind,
                StatementKind::Select | StatementKind::Explain | StatementKind::Show
            ) {
                return kind;
            }
        }
        return base;
    }
    if matches!(statement, Statement::Query(_)) {
        if !facts.writes.is_empty() {
            issues.push(Issue::new(Severity::Warning, "data_modifying_cte", "This query contains a data-modifying CTE or nested write. Read-only access and SELECT row limits do not make the write safe."));
        }
        if facts.select_into {
            issues.push(Issue::new(Severity::Warning, "select_into", "SELECT INTO creates a destination table. It is a schema change requiring Admin access and approval."));
            return StatementKind::Ddl;
        }
        if let Some(kind) = facts.writes.first() {
            return *kind;
        }
    }
    base
}

fn original_limit(statement: &Statement) -> bool {
    match statement {
        Statement::Query(query) => rewrite::has_limit(query),
        Statement::Explain { statement, .. } => original_limit(statement),
        Statement::Update(update) => update.limit.is_some(),
        Statement::Delete(delete) => delete.limit.is_some(),
        _ => false,
    }
}

fn risk(statement: &Statement, kind: StatementKind, facts: &walk::Facts, has_limit: bool) -> Risk {
    if kind == StatementKind::Ddl {
        return Risk::Critical;
    }
    if classify::is_write(kind) || facts.locking {
        return Risk::High;
    }
    if !matches!(
        kind,
        StatementKind::Select | StatementKind::Explain | StatementKind::Show
    ) {
        return Risk::High;
    }
    let aggregate = match statement {
        Statement::Query(query) => walk::aggregate_read(&query.body),
        Statement::Explain { statement, .. } => match statement.as_ref() {
            Statement::Query(query) => walk::aggregate_read(&query.body),
            _ => false,
        },
        _ => false,
    };
    if kind == StatementKind::Show
        || (!facts.broad_read && !facts.cross_join && (has_limit || aggregate))
    {
        Risk::Low
    } else {
        Risk::Medium
    }
}
