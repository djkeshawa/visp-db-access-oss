//! Guard, cost gate, concurrency, routing, cancellation, masking and query history.
use crate::{
    app::AppState,
    audit::{Event, History},
    auth::User,
    db,
    error::{ApiError, Input},
    network::ClientIp,
    policy::Policy,
    rbac,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use uuid::Uuid;
use vda_guard::{AccessLevel, Analysis, Verdict};

/// SQL input; the optional client-generated query ID enables early cancellation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryInput {
    pub sql: String,
    pub query_id: Option<Uuid>,
}
/// Complete guarded query result wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub query_id: Uuid,
    pub columns: Vec<crate::masking::Column>,
    pub rows: Vec<Vec<Value>>,
    pub row_count: usize,
    pub truncated: bool,
    pub affected_rows: Option<u64>,
    pub elapsed_ms: u64,
    pub executed_sql: String,
    pub routed_to: String,
    pub analysis: Analysis,
}
fn validate_sql(sql: &str) -> Result<(), ApiError> {
    crate::sanitation::text(sql)?;
    if sql.trim().is_empty() || sql.len() > 1024 * 1024 {
        Err(ApiError::validation("SQL must contain 1..=1048576 bytes"))
    } else {
        Ok(())
    }
}
fn dialect(c: &db::ClusterRecord) -> vda_guard::Dialect {
    if c.engine == "mysql" {
        vda_guard::Dialect::MySql
    } else {
        vda_guard::Dialect::Postgres
    }
}
/// Analyze without target I/O, under the caller's current access and policy.
pub async fn analyze(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<QueryInput>,
) -> Result<Json<Analysis>, ApiError> {
    validate_sql(&body.sql)?;
    let cluster_id = db::id(&id)?;
    let (access, policy) = tokio::join!(
        rbac::require_level_effective(&state.db, &user, cluster_id, AccessLevel::Read),
        db::policy(&state.db, cluster_id)
    );
    let (c, level) = access?;
    let policy = policy?;
    if !crate::network::allowed(ip.0, &crate::network::parse_cidrs(&policy.allowed_cidrs)?) {
        return Err(ApiError::forbidden());
    }
    let mut analysis = vda_guard::analyze(&body.sql, dialect(&c), level, &policy.guard());
    crate::masking::harden_analysis(
        &mut analysis,
        &policy.masked_columns,
        &body.sql,
        dialect(&c),
    );
    Ok(Json(analysis))
}
/// Execute a guarded query for the caller.
pub async fn execute(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<QueryInput>,
) -> Result<Json<QueryResult>, ApiError> {
    Ok(Json(
        run(&state, &user, db::id(&id)?, body, ip.0, false).await?,
    ))
}
struct Registration {
    state: AppState,
    id: Uuid,
    token: CancellationToken,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.token.cancel();
        self.state.active.remove(&self.id);
    }
}
/// Run the pipeline; approved executions still re-check current grants and policy.
pub async fn run(
    state: &AppState,
    user: &User,
    cluster_id: Uuid,
    body: QueryInput,
    ip: std::net::IpAddr,
    approved: bool,
) -> Result<QueryResult, ApiError> {
    run_inner(state, user, cluster_id, body, ip, (approved, None)).await
}

/// Use the claim's timeout ceiling even if policy changes while approval work starts.
pub(crate) async fn run_approved(
    state: &AppState,
    user: &User,
    cluster_id: Uuid,
    body: QueryInput,
    ip: std::net::IpAddr,
    timeout_ms: u64,
) -> Result<QueryResult, ApiError> {
    run_inner(state, user, cluster_id, body, ip, (true, Some(timeout_ms))).await
}

async fn run_inner(
    state: &AppState,
    user: &User,
    cluster_id: Uuid,
    body: QueryInput,
    ip: std::net::IpAddr,
    approval: (bool, Option<u64>),
) -> Result<QueryResult, ApiError> {
    let (approved, timeout_ms) = approval;
    validate_sql(&body.sql)?;
    let (access, policy) = tokio::join!(
        rbac::require_level_effective(&state.db, user, cluster_id, AccessLevel::Read),
        db::policy(&state.db, cluster_id)
    );
    let (c, level) = access?;
    let mut policy = policy?;
    if !crate::network::allowed(ip, &crate::network::parse_cidrs(&policy.allowed_cidrs)?) {
        return Err(ApiError::forbidden());
    }
    if let Some(timeout_ms) = timeout_ms {
        policy.statement_timeout_ms = policy.statement_timeout_ms.min(timeout_ms);
        policy.lock_timeout_ms = policy.lock_timeout_ms.min(policy.statement_timeout_ms);
    }
    if approved {
        policy.require_approval_for_writes = false;
    }
    let mut analysis = vda_guard::analyze(&body.sql, dialect(&c), level, &policy.guard());
    crate::masking::harden_analysis(
        &mut analysis,
        &policy.masked_columns,
        &body.sql,
        dialect(&c),
    );
    let query_id = body.query_id.unwrap_or_else(Uuid::new_v4);
    let token = CancellationToken::new();
    match state.active.entry(query_id) {
        dashmap::mapref::entry::Entry::Occupied(_) => {
            return Err(ApiError::conflict("Query ID is already active"))
        }
        dashmap::mapref::entry::Entry::Vacant(v) => {
            v.insert((user.id, token.clone()));
        }
    }
    let registration = Registration {
        state: state.clone(),
        id: query_id,
        token: token.clone(),
    };
    // A client UUID is only a cancellation handle; every attempt has a fresh audit identity.
    let history_id = Uuid::new_v4();
    let verdict = serde_json::to_value(analysis.verdict)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "deny".into());
    let blocked = analysis.verdict == Verdict::Deny
        || (analysis.verdict == Verdict::RequiresApproval && !approved);
    let history = History {
        id: history_id,
        cluster_id: c.id,
        cluster_name: c.name.clone(),
        user_id: user.id,
        user_email: user.email.clone(),
        sql: body.sql,
        verdict: verdict.clone(),
        status: if blocked { "blocked" } else { "running" }.into(),
        row_count: None,
        elapsed_ms: blocked.then_some(0),
        error: if analysis.verdict == Verdict::Deny {
            Some("Query denied by policy".into())
        } else if blocked {
            Some("Query requires approval".into())
        } else {
            None
        },
    };
    let intent = Event {
        id: Uuid::new_v4(),
        actor_id: Some(user.id),
        action: "query.execute".into(),
        target_type: Some("cluster".into()),
        target_id: Some(c.id),
        ip: Some(ip),
        details: json!({"query_id":query_id,"history_id":history_id,"status":history.status}),
        created_at: chrono::Utc::now(),
    };
    crate::audit::begin_query(&state.db, &history, &intent)
        .await
        .map_err(|error| {
            tracing::error!(%error, "query intent persistence failed; refusing target I/O");
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "internal",
                "Audit unavailable",
            )
        })?;
    let cancel_on_drop = token.clone().drop_guard();
    let state = state.clone();
    let tracker = state.tasks.clone();
    let task = tracker.spawn(
        async move {
            let _registration = registration;
            let start = Instant::now();
            // EXPLAIN and execution share an absolute lifetime. Keep that lifetime
            // below the crash-recovery threshold, with bounded pool/DNS overhead.
            let deadline =
                Duration::from_millis(policy.statement_timeout_ms) + Duration::from_secs(30);
            let mut result = tokio::time::timeout(
                deadline,
                pipeline(
                    &state,
                    &c,
                    &policy,
                    analysis.clone(),
                    query_id,
                    token,
                    approved,
                ),
            )
            .await
            .unwrap_or_else(|_| Err(vda_connectors::ConnectorError::Timeout.into()));
            if crate::masking::touches_masked(&analysis, &policy.masked_columns) {
                if let Err(error) = &mut result {
                    if error.kind == crate::error::ErrorKind::Database {
                        error.message =
                            "Query failed (details hidden because the query touches masked data)"
                                .into();
                        error.details = Value::Null;
                    }
                }
            }
            let status = match &result {
                Ok(_) => "ok",
                Err(e)
                    if matches!(
                        e.code,
                        "query_denied" | "approval_required" | "cost_exceeded"
                    ) =>
                {
                    "blocked"
                }
                Err(e) if e.kind == crate::error::ErrorKind::Cancelled => "cancelled",
                Err(_) => "error",
            };
            metrics::histogram!(
                "vda_query_duration_seconds",
                "cluster" => c.id.to_string(),
                "verdict" => verdict.clone()
            )
            .record(start.elapsed().as_secs_f64());
            if status == "blocked" {
                metrics::counter!(
                    "vda_blocked_queries_total",
                    "cluster" => c.id.to_string()
                )
                .increment(1);
            }
            // Finalization is synchronous and contains no caller-controlled identity.
            // On failure/crash the committed intent survives for maintenance recovery.
            if !blocked {
                sqlx::query(
                    "UPDATE query_history SET status=$2,row_count=$3,elapsed_ms=$4,error=$5
             WHERE id=$1 AND status IN ('running','blocked')",
                )
                .bind(history_id)
                .bind(status)
                .bind(result.as_ref().ok().map(|r| r.row_count as i64))
                .bind(start.elapsed().as_millis() as i64)
                .bind(result.as_ref().err().map(|e| &e.message))
                .execute(&state.db)
                .await?;
            }
            result
        }
        .instrument(tracing::Span::current()),
    );
    let result = task.await.map_err(|_| ApiError::internal())?;
    drop(cancel_on_drop);
    result
}

async fn pipeline(
    state: &AppState,
    c: &db::ClusterRecord,
    policy: &Policy,
    analysis: Analysis,
    query_id: Uuid,
    token: CancellationToken,
    approved: bool,
) -> Result<QueryResult, ApiError> {
    match analysis.verdict {
        Verdict::Deny => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "query_denied",
                "Query denied by policy",
            )
            .details(&analysis))
        }
        Verdict::RequiresApproval if !approved => {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "approval_required",
                "Query requires approval",
            )
            .details(&analysis))
        }
        _ => {}
    }
    let sql = analysis
        .rewritten_sql
        .clone()
        .ok_or_else(ApiError::internal)?;
    let pools = state.pools.get(state, c, policy).await?;
    let limits = policy.limits();
    let read = analysis.is_read_only();
    let replica = if read && policy.route_reads_to_replica && pools.replica.is_some() {
        state.replica_health.try_get_with(c.id, async {
            sqlx::query_scalar::<_,bool>(
                "SELECT COALESCE((SELECT status IN ('healthy','degraded') AND checked_at>now()-interval '2 minutes'
         FROM health_checks WHERE cluster_id=$1 AND endpoint='replica' ORDER BY checked_at DESC,id DESC LIMIT
         1),false)",
            )
.bind(c.id)
.fetch_one(&state.db).await.map_err(ApiError::from)
        }).await.map_err(|e| (*e).clone())?
    } else {
        false
    };
    let pool = if replica {
        pools.replica.as_ref().unwrap_or(&pools.primary)
    } else {
        &pools.primary
    };
    // EXPLAIN also consumes production resources and must share the concurrency cap.
    let permit = tokio::select! {
        _=token.cancelled()=>return Err(vda_connectors::ConnectorError::Cancelled.into()),
        value = tokio::time::timeout(Duration::from_secs(2), pools.semaphore.clone().acquire_owned()) => {
            value.map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "busy", "Cluster is busy"))?
                .map_err(|_| ApiError::internal())?
        }
    };
    if read {
        if let Some(max_cost) = policy.max_cost {
            let cost = tokio::select! {
                _ = token.cancelled() => return Err(vda_connectors::ConnectorError::Cancelled.into()),
                value = pool.explain_cost(&sql, &limits) => value?,
            };
            let cost = cost.filter(|v| v.is_finite()).ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "upstream",
                    "Query cost could not be determined",
                )
            })?;
            if cost > max_cost {
                return Err(ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "cost_exceeded",
                    "Estimated query cost exceeds policy",
                )
                .details(json!({"cost":cost,"max_cost":max_cost})));
            }
        }
    }
    let mut result = if read {
        pool.execute_read(&sql, &limits, token).await?
    } else {
        pool.execute_write(&sql, &limits, policy.max_affected_rows, token)
            .await?
    };
    drop(permit);
    let tables = analysis
        .statements
        .iter()
        .flat_map(|s| s.tables.iter().cloned())
        .collect::<Vec<_>>();
    let columns = crate::masking::apply(&mut result, &policy.masked_columns, &tables);
    Ok(QueryResult {
        query_id,
        columns,
        rows: result.rows,
        row_count: result.row_count,
        truncated: result.truncated,
        affected_rows: result.affected_rows,
        elapsed_ms: result.elapsed_ms,
        executed_sql: sql,
        routed_to: if replica { "replica" } else { "primary" }.into(),
        analysis,
    })
}
/// Cancel an active local query, with ownership checks.
pub async fn cancel(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = db::id(&id)?;
    if let Some(query) = state.active.get(&id) {
        if query.0 != user.id && user.org_role != "admin" {
            return Err(ApiError::forbidden());
        }
        query.1.cancel();
        return Ok(StatusCode::NO_CONTENT);
    }
    Err(ApiError::not_found())
}
/// Fetch scoped history with descending keyset pagination.
pub async fn history_items(
    state: &AppState,
    user: &User,
    page: &db::Page,
) -> Result<Value, ApiError> {
    let (limit, cursor) = page.bounds()?;
    if page.status.as_ref().is_some_and(|s| {
        !matches!(
            s.as_str(),
            "running" | "unknown" | "ok" | "blocked" | "error" | "cancelled"
        )
    }) {
        return Err(ApiError::validation("Invalid history status"));
    }
    let rows:Vec<Value>=sqlx::query_scalar(
"SELECT to_jsonb(h)-'cluster_ref' FROM query_history h WHERE ($1 OR h.user_id=$2) AND ($3::uuid IS
         NULL OR h.cluster_id=$3) AND ($4::uuid IS NULL OR h.user_id=$4) AND ($5::text IS NULL OR h.status=$5)
         AND ($6::timestamptz IS NULL OR (h.created_at,h.id)<($6,$7)) ORDER BY h.created_at DESC,h.id DESC
         LIMIT $8",
)
        .bind(user.org_role=="admin")
.bind(user.id)
.bind(page.cluster_id)
.bind(page.user_id)
.bind(&page.status)
.bind(cursor.as_ref().map(|c|c.created_at))
.bind(cursor.as_ref().map(|c|c.id))
.bind(limit+1)
.fetch_all(&state.db).await?;
    db::page(rows, limit)
}
/// History list endpoint.
pub async fn history(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Query(page): Query<db::Page>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(history_items(&state, &user, &page).await?))
}
