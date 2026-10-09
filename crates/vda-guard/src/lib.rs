//! SQL safety engine. Pure: no I/O, no async.
//!
//! `analyze` parses SQL for a dialect, classifies every statement by walking the
//! full AST, applies the caller's access level and the cluster policy, and
//! returns a verdict plus a rewritten SQL with an enforced row limit.
//!
//! INTERFACE CONTRACT: the public types and `analyze` signature below are
//! consumed by `vda-server`. Keep them stable; add fields only.

mod classify;
mod engine;
mod functions;
mod hints;
mod mysql;
mod parse;
mod policy;
mod predicates;
mod rewrite;
mod syntax;
mod tables;
mod walk;

use serde::{Deserialize, Serialize};

/// SQL dialect used to parse the submitted statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    Postgres,
    MySql,
}

/// Caller access level, ordered from read access to administration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read,
    Write,
    Admin,
}

/// The effective statement category, accounting for nested writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementKind {
    Select,
    Explain,
    Show,
    Insert,
    Update,
    Delete,
    Merge,
    Ddl,
    Dcl,
    Transaction,
    Utility,
    Other,
}

/// Issue severity, ordered from informational advice to a mandatory block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Block,
}

/// Estimated production impact, ordered from low to critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

/// Execution decision after applying structural checks and policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    RequiresApproval,
    Deny,
}

/// A stable machine-readable issue and a human-readable explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// How this issue affects execution.
    pub severity: Severity,
    /// Stable machine code, e.g. `write_without_where`, `dangerous_function`.
    pub code: String,
    /// Explanation and suggested next step.
    pub message: String,
}

/// The subset of the cluster policy the guard needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardPolicy {
    /// Maximum requested rows; rewrites reserve one extra row for truncation detection.
    pub max_rows: u64,
    /// Whether the cluster policy permits data writes.
    pub allow_writes: bool,
    /// Whether permitted data writes require approval.
    pub require_approval_for_writes: bool,
    /// Whether Admin callers may request schema changes.
    pub allow_ddl: bool,
    /// Patterns: `table`, `schema.table`, `schema.*`, `*.table`; case-insensitive.
    pub blocked_tables: Vec<String>,
}

/// Safety facts for one parsed statement before limit rewriting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementAnalysis {
    /// Effective kind, including writes hidden inside queries or EXPLAIN ANALYZE.
    pub kind: StatementKind,
    /// Normalized SQL for the original parsed statement.
    pub sql: String,
    /// Referenced relations, normalized lower-case, `schema.table` when qualified.
    pub tables: Vec<String>,
    /// Called functions, normalized lower-case.
    pub functions: Vec<String>,
    /// Estimated impact on the production database.
    pub risk: Risk,
    /// Issues found while analyzing this statement.
    pub issues: Vec<Issue>,
    /// Whether the original AST contains a WHERE clause.
    pub has_where: bool,
    /// Whether the original outer query or write has LIMIT or FETCH.
    pub has_limit: bool,
}

/// Overall verdict and the original statements’ safety analyses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Analysis {
    /// Overall execution decision; the most restrictive statement wins.
    pub verdict: Verdict,
    /// Estimated impact on the production database.
    pub risk: Risk,
    /// Per-statement analyses in their original order.
    pub statements: Vec<StatementAnalysis>,
    /// SQL to execute (limit enforced). `None` when verdict is `Deny`.
    pub rewritten_sql: Option<String>,
    /// All issues, flattened, including top-level ones (e.g. parse errors).
    pub issues: Vec<Issue>,
    /// Advisory performance hints; never affect the verdict. Empty when denied.
    #[serde(default)]
    pub suggestions: Vec<Suggestion>,
}

/// An optional performance improvement, independent of the safety verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// Stable machine code, e.g. `select_star`, `leading_wildcard`.
    pub code: String,
    /// What is slow and how to improve it.
    pub message: String,
    /// SQL the console can apply for the user; never executed by the guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

/// A ready-to-apply SQL edit attached to a suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// Short button label.
    pub label: String,
    /// The SQL to apply.
    pub sql: String,
    /// Whether to replace the current query or open the SQL alongside it.
    pub action: FixAction,
}

/// How the console applies a [`Fix`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixAction {
    Replace,
    NewTab,
}

impl Analysis {
    /// True when every statement is read-only (select/explain/show).
    pub fn is_read_only(&self) -> bool {
        self.statements.iter().all(|s| {
            matches!(
                s.kind,
                StatementKind::Select | StatementKind::Explain | StatementKind::Show
            )
        })
    }
}

/// Analyze `sql`. Never panics; unparsable SQL yields `Verdict::Deny` with a
/// `parse_error` issue rather than an `Err`.
pub fn analyze(sql: &str, dialect: Dialect, level: AccessLevel, policy: &GuardPolicy) -> Analysis {
    std::panic::catch_unwind(|| engine::analyze(sql, dialect, level, policy)).unwrap_or_else(|_| {
        engine::denied(
            "parse_error",
            "SQL analysis could not safely complete. Simplify the query and try again.",
        )
    })
}

impl Issue {
    pub(crate) fn new(severity: Severity, code: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.to_owned(),
            message: message.into(),
        }
    }
}
