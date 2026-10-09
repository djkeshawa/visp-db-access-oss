//! Administrative users, projects, grants, audit and network settings.
use crate::{
    app::AppState,
    auth::User,
    db,
    error::{ApiError, Input},
    network::{ClientIp, NetworkSettings},
    rbac,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub(crate) fn validate_email(email: &str) -> Result<(), ApiError> {
    crate::sanitation::text(email)?;
    let email = email.trim();
    if email.len() > 254
        || email.chars().any(char::is_whitespace)
        || email.split_once('@').is_none_or(|(local, domain)| {
            local.is_empty() || domain.is_empty() || domain.contains('@')
        })
    {
        return Err(ApiError::validation("Invalid email"));
    }
    Ok(())
}
fn name(value: &str) -> Result<(), ApiError> {
    crate::sanitation::text(value)?;
    if value.trim().is_empty() || value.len() > 256 {
        Err(ApiError::validation("Name must contain 1..=256 bytes"))
    } else {
        Ok(())
    }
}
fn role(value: &str) -> Result<(), ApiError> {
    if matches!(value, "admin" | "member") {
        Ok(())
    } else {
        Err(ApiError::validation("Invalid organization role"))
    }
}

pub(crate) async fn users(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<Json<Value>, ApiError> {
    rbac::require_admin(&user)?;
    let users:Vec<User>=sqlx::query_as(
"SELECT id,email,name,org_role,disabled,created_at,last_login_at FROM users ORDER BY created_at DESC,id DESC",
)
.fetch_all(&state.db).await?;
    Ok(Json(json!({"items":users})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateUser {
    email: String,
    name: String,
    password: String,
    org_role: String,
}
pub(crate) async fn create_user(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<CreateUser>,
) -> Result<Json<User>, ApiError> {
    rbac::require_admin(&user)?;
    validate_email(&body.email)?;
    name(&body.name)?;
    role(&body.org_role)?;
    let hash = crate::auth::hash_password(body.password).await?;
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO users(id,email,name,password_hash,org_role) VALUES ($1,$2,$3,$4,$5)")
        .bind(id)
        .bind(body.email.trim().to_lowercase())
        .bind(body.name)
        .bind(hash)
        .bind(body.org_role)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "user.create", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(db::user(&state.db, id).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUser {
    name: Option<String>,
    org_role: Option<String>,
    disabled: Option<bool>,
    password: Option<String>,
}
pub(crate) async fn update_user(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<UpdateUser>,
) -> Result<Json<User>, ApiError> {
    rbac::require_admin(&user)?;
    let id = db::id(&id)?;
    if let Some(v) = &body.name {
        name(v)?;
    }
    if let Some(v) = &body.org_role {
        role(v)?;
    }
    if id == user.id && (body.disabled == Some(true) || body.org_role.as_deref() == Some("member"))
    {
        return Err(ApiError::validation(
            "Cannot disable or demote your own account",
        ));
    }
    let password = match body.password {
        Some(p) => Some(crate::auth::hash_password(p).await?),
        None => None,
    };
    let revoke = body.disabled == Some(true) || password.is_some() || body.org_role.is_some();
    let mut tx = state.db.begin().await?;
    let result=sqlx::query(
"UPDATE users SET
         name=COALESCE($2,name),org_role=COALESCE($3,org_role),disabled=COALESCE($4,disabled),password_hash=COALESCE($5,password_hash)
         WHERE id=$1",
)
.bind(id)
.bind(body.name)
.bind(body.org_role)
.bind(body.disabled)
.bind(password)
.execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    if revoke {
        sqlx::query("DELETE FROM sessions WHERE user_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    state
        .audit_change(&mut tx, &user, "user.update", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(db::user(&state.db, id).await?))
}
pub(crate) async fn delete_user(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    rbac::require_admin(&user)?;
    let id = db::id(&id)?;
    if user.id == id {
        return Err(ApiError::validation("Cannot disable your own account"));
    }
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("UPDATE users SET disabled=true WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "user.disable", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

const PROJECT_SQL:&str="SELECT jsonb_build_object('id',p.id,'name',p.name,'description',p.description,'cluster_count',(SELECT
         count(*) FROM clusters c WHERE c.project_id=p.id),'created_at',p.created_at) FROM projects p";
async fn project(db: &sqlx::PgPool, id: Uuid) -> Result<Value, ApiError> {
    Ok(sqlx::query_scalar(&format!("{PROJECT_SQL} WHERE p.id=$1"))
        .bind(id)
        .fetch_one(db)
        .await?)
}
pub(crate) async fn projects(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<Json<Value>, ApiError> {
    let sql=format!(
"{PROJECT_SQL} WHERE $2 OR EXISTS (SELECT 1 FROM clusters c JOIN users u ON u.id=$1 WHERE c.project_id=p.id AND {}) ORDER BY p.created_at DESC,p.id DESC",
rbac::ACCESS_SQL,
);
    let items: Vec<Value> = sqlx::query_scalar(&sql)
        .bind(user.id)
        .bind(user.org_role == "admin")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateProject {
    name: String,
    description: String,
}
pub(crate) async fn create_project(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<CreateProject>,
) -> Result<Json<Value>, ApiError> {
    rbac::require_admin(&user)?;
    name(&body.name)?;
    if body.description.len() > 10000 {
        return Err(ApiError::validation("Description too long"));
    }
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO projects(id,name,description) VALUES ($1,$2,$3)")
        .bind(id)
        .bind(body.name)
        .bind(body.description)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "project.create", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(project(&state.db, id).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateProject {
    name: Option<String>,
    description: Option<String>,
}
pub(crate) async fn update_project(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<UpdateProject>,
) -> Result<Json<Value>, ApiError> {
    rbac::require_admin(&user)?;
    let id = db::id(&id)?;
    if let Some(v) = &body.name {
        name(v)?;
    }
    if body.description.as_ref().is_some_and(|s| s.len() > 10000) {
        return Err(ApiError::validation("Description too long"));
    }
    let mut tx = state.db.begin().await?;
    let result=sqlx::query(
"UPDATE projects SET name=COALESCE($2,name),description=COALESCE($3,description) WHERE id=$1",
)
.bind(id)
.bind(body.name)
.bind(body.description)
.execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    state
        .audit_change(&mut tx, &user, "project.update", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(project(&state.db, id).await?))
}
pub(crate) async fn delete_project(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    rbac::require_admin(&user)?;
    let id = db::id(&id)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let populated: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM clusters WHERE project_id=$1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if populated {
        return Err(ApiError::conflict("Project still contains clusters"));
    }
    sqlx::query("DELETE FROM grants WHERE scope='project' AND scope_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM projects WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "project.delete", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) const GRANT_SQL:&str="SELECT
         jsonb_build_object('id',g.id,'user',jsonb_build_object('id',u.id,'email',u.email,'name',u.name),'scope',g.scope,'scope_id',g.scope_id,'scope_name',CASE
         WHEN g.scope='project' THEN (SELECT name FROM projects WHERE id=g.scope_id) ELSE (SELECT name FROM
         clusters WHERE id=g.scope_id)
         END,'level',g.level,'expires_at',g.expires_at,'created_at',g.created_at,'created_by',g.created_by)
         FROM grants g JOIN users u ON u.id=g.user_id";
pub(crate) async fn cluster_grants(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = rbac::require_level(
        &state.db,
        &user,
        db::id(&id)?,
        vda_guard::AccessLevel::Admin,
    )
    .await?;
    let items: Vec<Value> = sqlx::query_scalar(&format!(
        "{GRANT_SQL} WHERE (g.scope='cluster' AND g.scope_id=$1)
         OR (g.scope='project' AND g.scope_id=$2) ORDER BY g.created_at DESC,g.id DESC",
    ))
    .bind(c.id)
    .bind(c.project_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateGrant {
    user_id: Uuid,
    scope: String,
    scope_id: Uuid,
    level: String,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
async fn scope_admin(state: &AppState, user: &User, scope: &str, id: Uuid) -> Result<(), ApiError> {
    match scope {
        "cluster" => {
            rbac::require_level(&state.db, user, id, vda_guard::AccessLevel::Admin).await?;
        }
        "project" => {
            sqlx::query("SELECT id FROM projects WHERE id=$1")
                .bind(id)
                .fetch_one(&state.db)
                .await?;
            if user.org_role != "admin" {
                let permitted:bool=sqlx::query_scalar(
"SELECT EXISTS(SELECT 1 FROM grants WHERE user_id=$1 AND scope='project' AND scope_id=$2 AND
         level='admin' AND (expires_at IS NULL OR expires_at>now()))",
)
.bind(user.id)
.bind(id)
.fetch_one(&state.db).await?;
                if !permitted {
                    return Err(ApiError::forbidden());
                }
            }
        }
        _ => return Err(ApiError::validation("Invalid grant scope")),
    }
    Ok(())
}
pub(crate) async fn create_grant(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<CreateGrant>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(body.level.as_str(), "read" | "write" | "admin") {
        return Err(ApiError::validation("Invalid access level"));
    }
    if body.expires_at.is_some_and(|v| v <= chrono::Utc::now()) {
        return Err(ApiError::validation("Grant expiry must be in the future"));
    }
    scope_admin(&state, &user, &body.scope, body.scope_id).await?;
    let mut tx = state.db.begin().await?;
    if user.org_role != "admin" {
        if body.user_id == user.id {
            return Err(ApiError::forbidden());
        }
        let project = if body.scope == "cluster" {
            Some(
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT project_id FROM clusters WHERE id=$1 FOR SHARE",
                )
                .bind(body.scope_id)
                .fetch_one(&mut *tx)
                .await?,
            )
        } else {
            None
        };
        let grants: Vec<Option<chrono::DateTime<chrono::Utc>>> = sqlx::query_scalar(
            "SELECT expires_at FROM grants WHERE user_id=$1 AND level='admin' AND (expires_at IS NULL OR
         expires_at>now()) AND ((scope=$2 AND scope_id=$3) OR (scope='project' AND scope_id=$4)) FOR SHARE",
        )
        .bind(user.id)
        .bind(&body.scope)
        .bind(body.scope_id)
        .bind(project)
        .fetch_all(&mut *tx)
        .await?;
        if grants.is_empty() {
            return Err(ApiError::forbidden());
        }
        let expiry = grants.into_iter().flatten().min();
        if expiry.is_some_and(|limit| body.expires_at.is_none_or(|requested| requested > limit)) {
            return Err(ApiError::forbidden());
        }
    }
    let id = Uuid::new_v4();
    sqlx::query(
"INSERT INTO grants(id,user_id,scope,scope_id,level,expires_at,created_by) VALUES ($1,$2,$3,$4,$5,$6,$7)",
)
.bind(id)
.bind(body.user_id)
.bind(body.scope)
.bind(body.scope_id)
.bind(body.level)
.bind(body.expires_at)
.bind(user.id)
.execute(&mut *tx).await?;
    state
        .audit_change(&mut tx, &user, "grant.create", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(
        sqlx::query_scalar(&format!("{GRANT_SQL} WHERE g.id=$1"))
            .bind(id)
            .fetch_one(&state.db)
            .await?,
    ))
}
pub(crate) async fn delete_grant(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = db::id(&id)?;
    let (scope, scope_id): (String, Uuid) =
        sqlx::query_as("SELECT scope,scope_id FROM grants WHERE id=$1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    scope_admin(&state, &user, &scope, scope_id).await?;
    let mut tx = state.db.begin().await?;
    let deleted = sqlx::query("DELETE FROM grants WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // A concurrent admin may have removed it after the scope lookup.
    if deleted.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    state
        .audit_change(&mut tx, &user, "grant.delete", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn audit(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Query(page): Query<db::Page>,
) -> Result<Json<Value>, ApiError> {
    rbac::require_admin(&user)?;
    let (limit, cursor) = page.bounds()?;
    let items:Vec<Value>=sqlx::query_scalar(
"SELECT jsonb_build_object('id',a.id,'actor',CASE WHEN u.id IS NULL THEN NULL ELSE
         jsonb_build_object('id',u.id,'email',u.email)
         END,'action',a.action,'target_type',a.target_type,'target_id',a.target_id,'ip',host(a.ip),'details',a.details,'created_at',a.created_at)
         FROM audit_log a LEFT JOIN users u ON u.id=a.actor_id WHERE ($1::text IS NULL OR a.action=$1) AND
         ($2::uuid IS NULL OR a.actor_id=$2) AND ($3::timestamptz IS NULL OR (a.created_at,a.id)<($3,$4))
         ORDER BY a.created_at DESC,a.id DESC LIMIT $5",
)
        .bind(page.action)
.bind(page.actor_id)
.bind(cursor.as_ref().map(|c|c.created_at))
.bind(cursor.as_ref().map(|c|c.id))
.bind(limit+1)
.fetch_all(&state.db).await?;
    Ok(Json(db::page(items, limit)?))
}
pub(crate) async fn network(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<Json<NetworkSettings>, ApiError> {
    rbac::require_admin(&user)?;
    Ok(Json(state.network_settings().await?))
}
pub(crate) async fn set_network(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Input(body): Input<NetworkSettings>,
) -> Result<Json<NetworkSettings>, ApiError> {
    rbac::require_admin(&user)?;
    let cidrs = crate::network::parse_cidrs(&body.allowed_cidrs)?;
    let trust = state.config.trust_proxy_headers || body.trust_proxy_headers;
    let chain = if trust {
        crate::network::forwarded_chain(&headers)?
    } else {
        None
    };
    let next_ip = crate::network::client_ip(
        peer.ip(),
        chain.as_deref(),
        trust,
        &state.config.trusted_proxy_cidrs,
    )?;
    if !crate::network::allowed(ip.0, &cidrs) || !crate::network::allowed(next_ip, &cidrs) {
        return Err(ApiError::validation(
            "Network change would lock out your current IP",
        ));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE settings SET value=$1 WHERE key='network'")
        .bind(json!(body))
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "settings.network", None, ip.0)
        .await?;
    tx.commit().await?;
    state.invalidate_network().await;
    Ok(Json(body))
}
pub(crate) async fn overview(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<Json<Value>, ApiError> {
    let filters = crate::clusters::Filters::default();
    let history_sql = format!(
"SELECT count(*),count(*) FILTER(WHERE h.status='blocked'),count(*) FILTER(WHERE h.status='error')
         FROM query_history h JOIN clusters c ON c.id=h.cluster_id JOIN users u ON u.id=$1 WHERE
         h.created_at>now()-interval '24 hours' AND {}",
rbac::ACCESS_SQL,
    );
    let pending_sql = format!(
"SELECT count(*) FROM approvals a JOIN clusters c ON c.id=a.cluster_id JOIN users u ON u.id=$1 WHERE a.status='pending' AND a.expires_at>now() AND {}",
rbac::ACCESS_SQL,
    );
    let page = db::Page {
        limit: Some(10),
        ..Default::default()
    };
    // The queries are independent, so run them concurrently.
    let (clusters, (queries, blocked, errors), pending, recent, discovery): (
        Vec<Value>,
        (i64, i64, i64),
        i64,
        Value,
        Value,
    ) = futures::try_join!(
        crate::clusters::accessible(&state, &user, &filters),
        async {
            Ok::<_, ApiError>(
                sqlx::query_as(&history_sql)
                    .bind(user.id)
                    .fetch_one(&state.db)
                    .await?,
            )
        },
        async {
            Ok::<_, ApiError>(
                sqlx::query_scalar(&pending_sql)
                    .bind(user.id)
                    .fetch_one(&state.db)
                    .await?,
            )
        },
        crate::query::history_items(&state, &user, &page),
        async {
            if user.org_role == "admin" {
                Ok::<_, ApiError>(sqlx::query_scalar("SELECT jsonb_build_object('new',count(*) FILTER(WHERE status='new'),'gone',count(*) FILTER(WHERE status='gone'),'drifted',count(*) FILTER(WHERE drift!='[]'::jsonb)) FROM discovered_resources").fetch_one(&state.db).await?)
            } else {
                Ok(Value::Null)
            }
        },
    )?;
    let clusters_total = clusters.len();
    let mut healthy = 0;
    let mut degraded = 0;
    let mut down = 0;
    let mut unknown = 0;
    let mut unhealthy = Vec::new();
    for c in clusters {
        match c
            .get("health")
            .and_then(|h| h.get("status"))
            .and_then(Value::as_str)
        {
            Some("healthy") => healthy += 1,
            Some("degraded") => {
                degraded += 1;
                unhealthy.push(c);
            }
            Some("down") => {
                down += 1;
                unhealthy.push(c);
            }
            _ => unknown += 1,
        }
    }
    Ok(Json(json!({
        "discovery": discovery,
        "clusters_total": clusters_total,
        "healthy": healthy,
        "degraded": degraded,
        "down": down,
        "unknown": unknown,
        "queries_24h": queries,
        "blocked_24h": blocked,
        "errors_24h": errors,
        "pending_approvals": pending,
        "recent_queries": recent.get("items"),
        "unhealthy_clusters": unhealthy,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_validation() {
        assert!(validate_email("a@b.example").is_ok());
        for bad in [
            "",
            "a",
            "@b",
            "a@",
            "a@b@c",
            "a b@c",
            &format!("{}@x", "a".repeat(260)),
        ] {
            assert!(validate_email(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn name_and_role_validation() {
        assert!(name("Ops").is_ok());
        assert!(name("   ").is_err());
        assert!(name(&"x".repeat(257)).is_err());
        assert!(role("admin").is_ok());
        assert!(role("member").is_ok());
        assert!(role("owner").is_err());
    }
}
