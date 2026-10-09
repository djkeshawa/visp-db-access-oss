//! Four-eyes review and atomic single-use approved execution.
use crate::{
    app::AppState,
    auth::User,
    db,
    error::{ApiError, Input},
    network::ClientIp,
    query::QueryInput,
    rbac,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::Instrument;
use uuid::Uuid;
use vda_guard::{AccessLevel, Verdict};

const APPROVAL_SQL: &str = "SELECT jsonb_build_object( 'id',a.id,'cluster_id',a.cluster_id,'cluster_name',c.name,
         'requester',jsonb_build_object('id',u.id,'email',u.email,'name',u.name),
         'sql',a.sql,'sql_truncated',false,'reason',a.reason,'analysis',a.analysis,
         'status',a.status,'error',a.error, 'reviewer',CASE WHEN r.id IS NULL THEN NULL ELSE
         jsonb_build_object('id',r.id,'email',r.email,'name',r.name) END,
         'review_note',a.review_note,'result',a.result,'created_at',a.created_at,
         'reviewed_at',a.reviewed_at,'executed_at',a.executed_at,'expires_at',a.expires_at) FROM approvals a
         JOIN clusters c ON c.id=a.cluster_id JOIN users u ON u.id=a.requester_id LEFT JOIN users r ON
         r.id=a.reviewer_id";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalInput {
    cluster_id: Uuid,
    sql: String,
    reason: String,
}
/// Re-analyze requested SQL and create a 24-hour review request.
pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<ApprovalInput>,
) -> Result<Json<Value>, ApiError> {
    let (c, level) =
        rbac::require_level_effective(&state.db, &user, body.cluster_id, AccessLevel::Write)
            .await?;
    if body.reason.trim().is_empty()
        || body.reason.len() > 10000
        || body.sql.trim().is_empty()
        || body.sql.len() > 1024 * 1024
    {
        return Err(ApiError::validation("Invalid SQL or approval reason"));
    }
    let policy = db::policy(&state.db, c.id).await?;
    if !crate::network::allowed(ip.0, &crate::network::parse_cidrs(&policy.allowed_cidrs)?) {
        return Err(ApiError::forbidden());
    }
    let dialect = if c.engine == "mysql" {
        vda_guard::Dialect::MySql
    } else {
        vda_guard::Dialect::Postgres
    };
    let mut analysis = vda_guard::analyze(&body.sql, dialect, level, &policy.guard());
    crate::masking::harden_analysis(&mut analysis, &policy.masked_columns, &body.sql, dialect);
    if analysis.verdict == Verdict::Deny {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "query_denied",
            "Query denied by policy",
        )
        .details(analysis));
    }
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query(
"INSERT INTO approvals(id,cluster_id,requester_id,sql,reason,analysis,status) VALUES ($1,$2,$3,$4,$5,$6,'pending')",
)
.bind(id)
.bind(c.id)
.bind(user.id)
.bind(body.sql)
.bind(body.reason)
.bind(json!(analysis))
.execute(&mut *tx).await?;
    state
        .audit_change(&mut tx, &user, "approval.create", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(view(&state, id).await?))
}
async fn view(state: &AppState, id: Uuid) -> Result<Value, ApiError> {
    Ok(sqlx::query_scalar(&format!("{APPROVAL_SQL} WHERE a.id=$1"))
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}
#[derive(Debug, Deserialize, Default)]
pub struct ApprovalFilters {
    status: Option<String>,
    cluster_id: Option<Uuid>,
    #[serde(default)]
    mine: bool,
    limit: Option<i64>,
    cursor: Option<String>,
}
/// List accessible approval requests; expiration is also enforced without a scheduler.
pub async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Query(filters): Query<ApprovalFilters>,
) -> Result<Json<Value>, ApiError> {
    if filters.status.as_ref().is_some_and(|s| {
        !matches!(
            s.as_str(),
            "pending" | "approved" | "rejected" | "executing" | "executed" | "failed" | "expired"
        )
    }) {
        return Err(ApiError::validation("Invalid approval status"));
    }
    sqlx::query(
"UPDATE approvals SET status='expired' WHERE status IN ('pending','approved') AND expires_at<=now()",
)
.execute(&state.db).await?;
    let limit = filters.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(ApiError::validation("Limit must be 1..=100"));
    }
    let cursor = filters
        .cursor
        .as_deref()
        .map(db::cursor::Cursor::decode)
        .transpose()?;
    let projection = APPROVAL_SQL
        .replace(
            "'sql',a.sql,'sql_truncated',false",
            "'sql',encode(substring(convert_to(a.sql,'UTF8') from 1 for 8000),'base64'),'sql_truncated',octet_length(a.sql)>8000",
        )
        .replace("'result',a.result", "'result',NULL");
    let sql = format!("{projection} JOIN users viewer ON viewer.id=$1
        WHERE (viewer.org_role='admin' OR a.requester_id=$1 OR EXISTS(
            SELECT 1 FROM grants g WHERE g.user_id=$1 AND g.level='admin'
            AND (g.expires_at IS NULL OR g.expires_at>now())
            AND ((g.scope='cluster' AND g.scope_id=c.id) OR (g.scope='project' AND g.scope_id=c.project_id))))
        AND ($2::text IS NULL OR a.status=$2) AND ($3::uuid IS NULL OR a.cluster_id=$3)
        AND (NOT $4 OR a.requester_id=$1)
        AND ($5::timestamptz IS NULL OR (a.created_at,a.id)<($5,$6))
        ORDER BY a.created_at DESC,a.id DESC LIMIT $7");
    let mut items: Vec<Value> = sqlx::query_scalar(&sql)
        .bind(user.id)
        .bind(filters.status)
        .bind(filters.cluster_id)
        .bind(filters.mine)
        .bind(cursor.as_ref().map(|c| c.created_at))
        .bind(cursor.as_ref().map(|c| c.id))
        .bind(limit + 1)
        .fetch_all(&state.db)
        .await?;
    // Bound SQL transfer to 8 KiB (2,000 maximum-width UTF-8 characters),
    // then truncate by Unicode characters independent of metadata DB encoding.
    use base64::Engine;
    for item in &mut items {
        let encoded = item["sql"].as_str().ok_or_else(ApiError::internal)?;
        let encoded = encoded.split_whitespace().collect::<String>();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| ApiError::internal())?;
        let sql = String::from_utf8_lossy(&bytes);
        let truncated = item["sql_truncated"] == true || sql.chars().count() > 2000;
        item["sql"] = Value::String(sql.chars().take(2000).collect());
        item["sql_truncated"] = Value::Bool(truncated);
    }
    Ok(Json(db::page(items, limit)?))
}
async fn visible(state: &AppState, user: &User, id: Uuid) -> Result<(Uuid, Uuid), ApiError> {
    let (cluster_id, requester): (Uuid, Uuid) =
        sqlx::query_as("SELECT cluster_id,requester_id FROM approvals WHERE id=$1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    if user.id != requester {
        rbac::require_level(&state.db, user, cluster_id, AccessLevel::Admin).await?;
    } else {
        rbac::require_level(&state.db, user, cluster_id, AccessLevel::Read).await?;
    }
    Ok((cluster_id, requester))
}
/// Return a visible approval request.
pub async fn get(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = db::id(&id)?;
    visible(&state, &user, id).await?;
    sqlx::query(
"UPDATE approvals SET status='expired' WHERE id=$1 AND status IN ('pending','approved') AND expires_at<=now()",
)
.bind(id)
.execute(&state.db).await?;
    Ok(Json(view(&state, id).await?))
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    note: Option<String>,
}
async fn review(
    state: &AppState,
    user: &User,
    id: Uuid,
    note: Option<String>,
    approved: bool,
    ip: std::net::IpAddr,
) -> Result<Value, ApiError> {
    // `visible` already requires cluster admin for every non-requester; requesters never self-review.
    let (cluster_id, requester) = visible(state, user, id).await?;
    if requester == user.id {
        return Err(ApiError::forbidden());
    }
    crate::network::require_cluster(state, cluster_id, ip).await?;
    if note.as_ref().is_some_and(|n| n.len() > 10000)
        || (!approved && note.as_ref().is_none_or(|n| n.trim().is_empty()))
    {
        return Err(ApiError::validation(
            "Rejection requires a note; notes must be at most 10000 bytes",
        ));
    }
    let mut tx = state.db.begin().await?;
    let result=sqlx::query(
"UPDATE approvals SET status=$2,reviewer_id=$3,review_note=$4,reviewed_at=now() WHERE id=$1 AND
         status='pending' AND expires_at>now() AND requester_id<>$3",
)
        .bind(id)
.bind(if approved {"approved"} else {"rejected"})
.bind(user.id)
.bind(note)
.execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Approval is expired or already reviewed",
        ));
    }
    state
        .audit_change(
            &mut tx,
            user,
            if approved {
                "approval.approve"
            } else {
                "approval.reject"
            },
            Some(id),
            ip,
        )
        .await?;
    tx.commit().await?;
    view(state, id).await
}
/// Approve a pending request as a distinct cluster administrator.
pub async fn approve(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<Review>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        review(&state, &user, db::id(&id)?, body.note, true, ip.0).await?,
    ))
}
/// Reject a pending request, requiring a review note.
pub async fn reject(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<Review>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        review(&state, &user, db::id(&id)?, body.note, false, ip.0).await?,
    ))
}
/// Atomically claim an approved request before executing; failed claims are never replayed.
pub async fn execute(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = db::id(&id)?;
    let (cluster, requester, reviewer): (Uuid, Uuid, Option<Uuid>) =
        sqlx::query_as("SELECT cluster_id,requester_id,reviewer_id FROM approvals WHERE id=$1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    if user.id != requester && Some(user.id) != reviewer {
        return Err(ApiError::forbidden());
    }
    let reviewer_id = reviewer.ok_or_else(ApiError::forbidden)?;
    let (original, reviewer, policy) = tokio::try_join!(
        db::user(&state.db, requester),
        db::user(&state.db, reviewer_id),
        db::policy(&state.db, cluster),
    )?;
    if original.disabled || reviewer.disabled {
        return Err(ApiError::forbidden());
    }
    // The caller, the requester and the reviewer must all retain their required rights at execution.
    tokio::try_join!(
        rbac::require_level(&state.db, &user, cluster, AccessLevel::Write),
        rbac::require_level(&state.db, &original, cluster, AccessLevel::Write),
        rbac::require_level(&state.db, &reviewer, cluster, AccessLevel::Admin),
    )?;
    if !crate::network::allowed(ip.0, &crate::network::parse_cidrs(&policy.allowed_cidrs)?) {
        return Err(ApiError::forbidden());
    }
    let timeout_ms = policy.statement_timeout_ms;
    let mut tx = state.db.begin().await?;
    let sql: Option<String> = sqlx::query_scalar(
        "UPDATE approvals SET execution_timeout_ms=$2,execution_started_at=now(),status='executing'
         WHERE id=$1 AND status='approved' AND expires_at>now() RETURNING sql",
    )
    .bind(id)
    .bind(timeout_ms as i64)
    .fetch_optional(&mut *tx)
    .await?;
    let sql = sql.ok_or_else(|| {
        ApiError::conflict("Approval is expired, not approved or already consumed")
    })?;
    state
        .audit_change(&mut tx, &user, "approval.execute", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    let tracker = state.tasks.clone();
    let task = tracker.spawn(
        finish_execution(state, original, cluster, sql, id, ip.0, timeout_ms)
            .instrument(tracing::Span::current()),
    );
    task.await.map_err(|_| ApiError::internal())?
}

async fn finish_execution(
    state: AppState,
    requester: User,
    cluster: Uuid,
    sql: String,
    id: Uuid,
    ip: std::net::IpAddr,
    timeout_ms: u64,
) -> Result<Json<Value>, ApiError> {
    let result = crate::query::run_approved(
        &state,
        &requester,
        cluster,
        QueryInput {
            sql,
            query_id: None,
        },
        ip,
        timeout_ms,
    )
    .await;
    let mut tx = state.db.begin().await?;
    match result {
        Ok(result) => {
            sqlx::query(
"UPDATE approvals SET status='executed',result=$2,executed_at=now() WHERE id=$1 AND status='executing'",
)
                .bind(id)
.bind(json!(result))
.execute(&mut *tx).await?;
            state
                .audit_change(&mut tx, &requester, "approval.executed", Some(id), ip)
                .await?;
            tx.commit().await?;
            Ok(Json(view(&state, id).await?))
        }
        Err(error) => {
            sqlx::query(
"UPDATE approvals SET status='failed',error=$2,executed_at=now() WHERE id=$1 AND status='executing'",
)
                .bind(id)
.bind(&error.message)
.execute(&mut *tx).await?;
            state
                .audit_change(&mut tx, &requester, "approval.failed", Some(id), ip)
                .await?;
            tx.commit().await?;
            Err(error)
        }
    }
}
