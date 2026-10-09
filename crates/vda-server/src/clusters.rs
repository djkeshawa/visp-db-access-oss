//! Scoped cluster CRUD, policy, connection tests and schema caching.
use crate::{
    app::AppState,
    auth::User,
    db::{self, ClusterRecord},
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
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
use vda_guard::AccessLevel;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Filters {
    project_id: Option<Uuid>,
    environment: Option<String>,
    engine: Option<String>,
    q: Option<String>,
}
/// Escape LIKE metacharacters so a search term matches literally.
fn escape_like(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for ch in term.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
pub(crate) async fn accessible(
    state: &AppState,
    user: &User,
    filters: &Filters,
) -> Result<Vec<Value>, ApiError> {
    let sql=format!("SELECT to_jsonb(c)-'password_enc' || jsonb_build_object('health',{},'my_access',CASE WHEN
         u.org_role='admin' THEN 'admin' ELSE (SELECT g.level FROM grants g WHERE g.user_id=u.id AND
         (g.expires_at IS NULL OR g.expires_at>now()) AND ((g.scope='cluster' AND g.scope_id=c.id) OR
         (g.scope='project' AND g.scope_id=c.project_id)) ORDER BY CASE g.level WHEN 'admin' THEN 3 WHEN
         'write' THEN 2 ELSE 1 END DESC LIMIT 1) END) FROM clusters c JOIN users u ON u.id=$1 WHERE {} AND
         ($2::uuid IS NULL OR c.project_id=$2) AND ($3::text IS NULL OR c.environment=$3) AND ($4::text IS
         NULL OR c.engine=$4) AND ($5::text IS NULL OR c.name ILIKE '%'||$5||'%' OR c.host ILIKE '%'||$5||'%')
         ORDER BY c.created_at DESC,c.id DESC",crate::health::CURRENT_HEALTH,rbac::ACCESS_SQL);
    Ok(sqlx::query_scalar(&sql)
        .bind(user.id)
        .bind(filters.project_id)
        .bind(&filters.environment)
        .bind(&filters.engine)
        .bind(filters.q.as_deref().map(escape_like))
        .fetch_all(&state.db)
        .await?)
}
pub(crate) async fn view(
    state: &AppState,
    user: &User,
    c: ClusterRecord,
) -> Result<Value, ApiError> {
    let (access, health) = futures::try_join!(
        rbac::effective(&state.db, user, &c),
        crate::health::current(&state.db, c.id),
    )?;
    let mut value = json!(c);
    let object = value.as_object_mut().ok_or_else(ApiError::internal)?;
    object.insert("health".into(), json!(health));
    object.insert("my_access".into(), json!(access));
    Ok(value)
}
pub(crate) async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Query(filters): Query<Filters>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"items":accessible(&state,&user,&filters).await?}),
    ))
}
pub(crate) async fn get_cluster(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let c = rbac::require_level(&state.db, &user, db::id(&id)?, AccessLevel::Read).await?;
    Ok(Json(view(&state, &user, c).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClusterInput {
    project_id: Uuid,
    name: String,
    engine: String,
    provider: String,
    region: String,
    environment: String,
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
    tls_mode: String,
    #[serde(default)]
    replica_host: Option<String>,
    #[serde(default)]
    replica_port: Option<u16>,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}
impl ClusterInput {
    pub(crate) async fn validate(&self, state: &AppState) -> Result<(), ApiError> {
        if !matches!(self.engine.as_str(), "postgres" | "mysql")
            || !matches!(
                self.provider.as_str(),
                "aws" | "gcp" | "azure" | "onprem" | "other"
            )
            || !matches!(
                self.environment.as_str(),
                "production" | "staging" | "development"
            )
            || !matches!(
                self.tls_mode.as_str(),
                "disable" | "prefer" | "require" | "verify_full"
            )
        {
            return Err(ApiError::validation(
                "Invalid cluster engine, provider, environment or TLS mode",
            ));
        }
        for field in [&self.name, &self.database, &self.username] {
            if field.trim().is_empty() || field.len() > 256 {
                return Err(ApiError::validation(
                    "Name, database and username must contain 1..=256 bytes",
                ));
            }
        }
        if self.password.len() > 4096
            || self.region.len() > 256
            || self.tags.len() > 64
            || self
                .tags
                .iter()
                .any(|(k, v)| k.len() > 256 || v.len() > 1024)
        {
            return Err(ApiError::validation("Cluster fields too large"));
        }
        crate::network::validate_host(&self.host, self.port, state.config.allow_private_targets)
            .await?;
        if let Some(host) = &self.replica_host {
            crate::network::validate_host(
                host,
                self.replica_port.unwrap_or(self.port),
                state.config.allow_private_targets,
            )
            .await?;
        } else if self.replica_port.is_some() {
            return Err(ApiError::validation("Replica port requires a replica host"));
        }
        Ok(())
    }
}
pub(crate) async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<ClusterInput>,
) -> Result<Json<Value>, ApiError> {
    rbac::require_admin(&user)?;
    body.validate(&state).await?;
    let mut tx = state.db.begin().await?;
    let id = insert(&state, &mut tx, body).await?;
    state
        .audit_change(&mut tx, &user, "cluster.create", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(
        view(&state, &user, db::cluster(&state.db, id).await?).await?,
    ))
}
/// Insert a validated cluster and default policy in the caller's transaction.
pub(crate) async fn insert(
    state: &AppState,
    connection: &mut sqlx::PgConnection,
    body: ClusterInput,
) -> Result<Uuid, ApiError> {
    let id = Uuid::new_v4();
    let encrypted = state
        .crypto
        .encrypt(id, &body.password)
        .map_err(|_| ApiError::internal())?;
    let policy = Policy::for_environment(&body.environment);
    sqlx::query(
"INSERT INTO
         clusters(id,project_id,name,engine,provider,region,environment,host,port,database,username,password_enc,tls_mode,replica_host,replica_port,tags)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
)
        .bind(id)
.bind(body.project_id)
.bind(body.name)
.bind(body.engine)
.bind(body.provider)
.bind(body.region)
.bind(body.environment)
.bind(body.host)
.bind(i32::from(body.port))
.bind(body.database)
.bind(body.username)
.bind(encrypted)
.bind(body.tls_mode)
.bind(body.replica_host)
.bind(body.replica_port.map(i32::from))
.bind(json!(body.tags))
.execute(&mut *connection).await?;
    sqlx::query("INSERT INTO cluster_policies(cluster_id,policy) VALUES ($1,$2)")
        .bind(id)
        .bind(json!(policy))
        .execute(&mut *connection)
        .await?;
    Ok(id)
}
pub(crate) async fn update(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(patch): Input<Value>,
) -> Result<Json<Value>, ApiError> {
    let id = db::id(&id)?;
    let current = rbac::require_level(&state.db, &user, id, AccessLevel::Admin).await?;
    let mut tx = state.db.begin().await?;
    apply_patch(&state, &mut tx, &user, ip.0, current, patch).await?;
    tx.commit().await?;
    state.pools.invalidate(id).await;
    state.schema_cache.invalidate(&id).await;
    Ok(Json(
        view(&state, &user, db::cluster(&state.db, id).await?).await?,
    ))
}
/// Apply an authorized cluster snapshot in the caller's transaction.
/// Optimistic concurrency rejects a snapshot changed since authorization.
pub(crate) async fn apply_patch(
    state: &AppState,
    connection: &mut sqlx::PgConnection,
    user: &User,
    ip: std::net::IpAddr,
    current: ClusterRecord,
    patch: Value,
) -> Result<(), ApiError> {
    let id = current.id;
    let object = patch
        .as_object()
        .ok_or_else(|| ApiError::validation("Expected an object"))?;
    let fields = [
        "project_id",
        "name",
        "engine",
        "provider",
        "region",
        "environment",
        "host",
        "port",
        "database",
        "username",
        "password",
        "tls_mode",
        "replica_host",
        "replica_port",
        "tags",
    ];
    if object.keys().any(|k| !fields.contains(&k.as_str())) {
        return Err(ApiError::validation("Unknown cluster field"));
    }
    let mut merged = json!(current);
    if endpoint_changed(&merged, &patch) && object.get("password").is_none_or(|p| !p.is_string()) {
        return Err(ApiError::validation(
            "Changing the connection endpoint requires re-entering the password",
        ));
    }
    // A supplied password replaces the stored one; skipping the decrypt lets an
    // admin recover a cluster whose ciphertext predates a master-key change.
    let password = match object.get("password") {
        Some(Value::String(password)) => json!(password),
        _ => json!(state
            .crypto
            .decrypt(id, &current.password_enc)
            .map_err(|_| ApiError::credentials_unreadable(id))?),
    };
    let map = merged.as_object_mut().ok_or_else(ApiError::internal)?;
    map.insert("password".into(), password);
    for field in ["id", "created_at", "updated_at"] {
        map.remove(field);
    }
    for (key, value) in object {
        map.insert(key.clone(), value.clone());
    }
    let body: ClusterInput = serde_json::from_value(merged)
        .map_err(|_| ApiError::validation("Invalid cluster fields"))?;
    if body.project_id != current.project_id {
        rbac::require_admin(user)?;
    }
    body.validate(state).await?;
    let encrypted = state
        .crypto
        .encrypt(id, &body.password)
        .map_err(|_| ApiError::internal())?;
    // Optimistic concurrency avoids losing a concurrently rotated credential/configuration.
    let changed = sqlx::query(
        "UPDATE clusters SET
         project_id=$2,name=$3,engine=$4,provider=$5,region=$6,environment=$7,
         host=$8,port=$9,database=$10,username=$11,password_enc=$12,tls_mode=$13,
         replica_host=$14,replica_port=$15,tags=$16,updated_at=clock_timestamp()
         WHERE id=$1 AND updated_at=$17",
    )
    .bind(id)
    .bind(body.project_id)
    .bind(body.name)
    .bind(body.engine)
    .bind(body.provider)
    .bind(body.region)
    .bind(body.environment)
    .bind(body.host)
    .bind(i32::from(body.port))
    .bind(body.database)
    .bind(body.username)
    .bind(encrypted)
    .bind(body.tls_mode)
    .bind(body.replica_host)
    .bind(body.replica_port.map(i32::from))
    .bind(json!(body.tags))
    .bind(current.updated_at)
    .execute(&mut *connection)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(ApiError::conflict(
            "Cluster changed concurrently; reload and retry",
        ));
    }
    state
        .audit_change(connection, user, "cluster.update", Some(id), ip)
        .await?;
    Ok(())
}
pub(crate) async fn remove(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    rbac::require_admin(&user)?;
    let id = db::id(&id)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT id FROM clusters WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM grants WHERE scope='cluster' AND scope_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM clusters WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "cluster.delete", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    state.pools.invalidate(id).await;
    state.schema_cache.invalidate(&id).await;
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn get_policy(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> Result<Json<Policy>, ApiError> {
    let id = db::id(&id)?;
    rbac::require_level(&state.db, &user, id, AccessLevel::Read).await?;
    Ok(Json(db::policy(&state.db, id).await?))
}
pub(crate) async fn put_policy(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<Policy>,
) -> Result<Json<Policy>, ApiError> {
    let id = db::id(&id)?;
    rbac::require_level(&state.db, &user, id, AccessLevel::Admin).await?;
    body.validate()?;
    let mut tx = state.db.begin().await?;
    let touched = sqlx::query("UPDATE clusters SET updated_at=clock_timestamp() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let stored = sqlx::query("UPDATE cluster_policies SET policy=$2 WHERE cluster_id=$1")
        .bind(id)
        .bind(json!(body))
        .execute(&mut *tx)
        .await?;
    // The cluster may have been deleted after authorization; do not report success.
    if touched.rows_affected() == 0 || stored.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    state
        .audit_change(&mut tx, &user, "policy.update", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    state.pools.invalidate(id).await;
    Ok(Json(body))
}
#[derive(Deserialize)]
pub(crate) struct SchemaQuery {
    #[serde(default)]
    refresh: bool,
}
pub(crate) async fn schema(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Query(query): Query<SchemaQuery>,
) -> Result<Json<vda_connectors::SchemaTree>, ApiError> {
    let c = rbac::require_level(&state.db, &user, db::id(&id)?, AccessLevel::Read).await?;
    crate::network::require_cluster(&state, c.id, ip.0).await?;
    if !query.refresh {
        if let Some((version, schema)) = state.schema_cache.get(&c.id).await {
            if version == c.updated_at {
                return Ok(Json(schema));
            }
        }
    }
    let policy = db::policy(&state.db, c.id).await?;
    let pools = state.pools.get(&state, &c, &policy).await?;
    let _permit =
        tokio::time::timeout(std::time::Duration::from_secs(2), pools.semaphore.acquire())
            .await
            .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "busy", "Cluster is busy"))?
            .map_err(|_| ApiError::internal())?;
    let schema = pools.primary.schema().await?;
    state
        .schema_cache
        .insert(c.id, (c.updated_at, schema.clone()))
        .await;
    Ok(Json(schema))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionTest {
    engine: vda_connectors::Engine,
    host: String,
    port: u16,
    database: String,
    username: String,
    password: Option<String>,
    tls_mode: vda_connectors::TlsMode,
    cluster_id: Option<Uuid>,
}
pub(crate) async fn test_connection(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Input(body): Input<ConnectionTest>,
) -> Result<Json<crate::health::Health>, ApiError> {
    rbac::require_admin(&user)?;
    let password = match body.password {
        Some(p) => p,
        None => {
            let id = body
                .cluster_id
                .ok_or_else(|| ApiError::validation("Password or cluster_id is required"))?;
            let c = db::cluster(&state.db, id).await?;
            // A stored password may only be tested against its configured endpoint.
            if body.host != c.host
                || i32::from(body.port) != c.port
                || body.username != c.username
                || body.database != c.database
                || json!(body.engine).as_str() != Some(c.engine.as_str())
                || json!(body.tls_mode).as_str() != Some(c.tls_mode.as_str())
            {
                return Err(ApiError::validation(
                    "Changing the connection endpoint requires re-entering the password",
                ));
            }
            state
                .crypto
                .decrypt(id, &c.password_enc)
                .map_err(|_| ApiError::credentials_unreadable(id))?
        }
    };
    crate::network::validate_host(&body.host, body.port, state.config.allow_private_targets)
        .await?;
    let spec = vda_connectors::ConnectionSpec {
        engine: body.engine,
        host: body.host,
        port: body.port,
        database: body.database,
        username: body.username,
        password: password.into(),
        tls: body.tls_mode,
        ca_cert_pem: None,
        application_name: "visp-db-access:connection-test".into(),
    };
    Ok(Json(crate::health::Health::from_report(
        vda_connectors::test_connection(&spec).await,
    )))
}

fn endpoint_changed(current: &Value, patch: &Value) -> bool {
    [
        "engine",
        "host",
        "port",
        "database",
        "username",
        "tls_mode",
        "replica_host",
        "replica_port",
    ]
    .iter()
    .any(|field| {
        patch
            .get(field)
            .is_some_and(|value| Some(value) != current.get(field))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn like_search_terms_match_literally() {
        assert_eq!(escape_like("plain"), "plain");
        assert_eq!(escape_like("50%_off"), "50\\%\\_off");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
        assert_eq!(escape_like(""), "");
    }
    #[test]
    fn every_connection_field_requires_a_fresh_password_when_changed() {
        let current = json!({"engine":"postgres","host":"db","port":5432,"database":"shop",
            "username":"reader","tls_mode":"require","replica_host":null,"replica_port":null});
        for field in [
            "engine",
            "host",
            "port",
            "database",
            "username",
            "tls_mode",
            "replica_host",
            "replica_port",
        ] {
            assert!(
                endpoint_changed(&current, &json!({field:"changed"})),
                "{field}"
            );
            assert!(
                !endpoint_changed(&current, &json!({field:current.get(field)})),
                "{field}"
            );
        }
        assert!(!endpoint_changed(
            &current,
            &json!({"name":"Renamed","tags":{"x":"y"}})
        ));
    }
}
