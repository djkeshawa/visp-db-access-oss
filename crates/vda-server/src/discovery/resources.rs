use super::{drift, gate};
use crate::{
    app::AppState,
    auth::User,
    db,
    error::{ApiError, Input},
    network::ClientIp,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

const VIEW:&str="SELECT r.payload || jsonb_build_object('id',r.id,'source_id',r.source_id,'source_name',s.name,'provider','aws','tags',r.tags,'status',r.status,'cluster_id',r.cluster_id,'drift',r.drift,'first_seen_at',r.first_seen_at,'last_seen_at',r.last_seen_at) FROM discovered_resources r JOIN discovery_sources s ON s.id=r.source_id";
pub(super) async fn view(state: &AppState, id: Uuid) -> Result<Value, ApiError> {
    Ok(sqlx::query_scalar(&format!("{VIEW} WHERE r.id=$1"))
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}
pub(super) fn page(mut rows: Vec<Value>, limit: i64, time: &str) -> Result<Value, ApiError> {
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if more {
        rows.last()
            .map(|r| -> Result<String, ApiError> {
                Ok(db::cursor::Cursor {
                    created_at: serde_json::from_value(
                        r.get(time).cloned().ok_or_else(ApiError::internal)?,
                    )
                    .map_err(|_| ApiError::internal())?,
                    id: serde_json::from_value(
                        r.get("id").cloned().ok_or_else(ApiError::internal)?,
                    )
                    .map_err(|_| ApiError::internal())?,
                }
                .encode())
            })
            .transpose()?
    } else {
        None
    };
    Ok(json!({"items":rows,"next_cursor":next}))
}
#[derive(Debug, Deserialize)]
pub(super) struct Filters {
    source_id: Option<Uuid>,
    status: Option<String>,
    engine: Option<String>,
    region: Option<String>,
    q: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}
pub(super) async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Query(filters): Query<Filters>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    for text in [
        &filters.status,
        &filters.engine,
        &filters.region,
        &filters.q,
    ]
    .into_iter()
    .flatten()
    {
        crate::sanitation::text(text)?;
        if text.len() > 256 {
            return Err(ApiError::validation("Filter too long"));
        }
    }
    if filters
        .status
        .as_ref()
        .is_some_and(|s| !["new", "imported", "ignored", "gone"].contains(&s.as_str()))
        || filters
            .engine
            .as_ref()
            .is_some_and(|s| !["postgres", "mysql"].contains(&s.as_str()))
    {
        return Err(ApiError::validation("Invalid discovery status or engine"));
    }
    let (limit, cursor) = db::Page {
        limit: filters.limit,
        cursor: filters.cursor,
        ..Default::default()
    }
    .bounds()?;
    let predicate=" WHERE ($1::uuid IS NULL OR r.source_id=$1) AND ($2::text IS NULL OR r.engine=$2) AND ($3::text IS NULL OR r.region=$3) AND ($4::text IS NULL OR strpos(lower(r.payload->>'identifier'),lower($4))>0 OR strpos(lower(r.payload->>'host'),lower($4))>0 OR strpos(lower(r.arn),lower($4))>0)";
    let items:Vec<Value>=sqlx::query_scalar(&format!("{VIEW}{predicate} AND ($5::text IS NULL OR r.status=$5) AND ($6::timestamptz IS NULL OR (r.first_seen_at,r.id)<($6,$7)) ORDER BY r.first_seen_at DESC,r.id DESC LIMIT $8")).bind(filters.source_id).bind(&filters.engine).bind(&filters.region).bind(&filters.q).bind(&filters.status).bind(cursor.as_ref().map(|c|c.created_at)).bind(cursor.as_ref().map(|c|c.id)).bind(limit+1).fetch_all(&state.db).await?;
    let counts:Value=sqlx::query_scalar(&format!("SELECT jsonb_build_object('new',count(*) FILTER(WHERE status='new'),'imported',count(*) FILTER(WHERE status='imported'),'ignored',count(*) FILTER(WHERE status='ignored'),'gone',count(*) FILTER(WHERE status='gone')) FROM discovered_resources r{predicate}")).bind(filters.source_id).bind(&filters.engine).bind(&filters.region).bind(&filters.q).fetch_one(&state.db).await?;
    let mut result = page(items, limit, "first_seen_at")?;
    result
        .as_object_mut()
        .ok_or_else(ApiError::internal)?
        .insert("counts".into(), counts);
    Ok(Json(result))
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportInput {
    project_id: Uuid,
    name: String,
    environment: String,
    database: String,
    username: String,
    password: String,
    tls_mode: String,
}
pub(super) async fn import(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<ImportInput>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let id = db::id(&id)?;
    let mut tx = state.db.begin().await?;
    let (payload, status, cluster_id): (Value, String, Option<Uuid>) = sqlx::query_as(
        "SELECT payload,status,cluster_id FROM discovered_resources WHERE id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if cluster_id.is_some() || status == "imported" {
        return Err(ApiError::conflict("Resource already imported"));
    }
    if status == "gone" {
        return Err(ApiError::conflict("Resource is gone"));
    }
    let mut tags = payload
        .get("tags")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    tags.insert(
        "aws:arn".into(),
        payload.get("arn").cloned().unwrap_or(Value::Null),
    );
    tags.insert(
        "aws:account".into(),
        payload.get("account_id").cloned().unwrap_or(Value::Null),
    );
    let fields = json!({"project_id":body.project_id,"name":body.name,"environment":body.environment,"database":body.database,"username":body.username,"password":body.password,"tls_mode":body.tls_mode,"host":payload.get("host"),"port":payload.get("port"),"engine":payload.get("engine"),"provider":"aws","region":payload.get("region"),"replica_host":payload.get("replica_host"),"replica_port":payload.get("replica_port"),"tags":tags});
    crate::sanitation::json(&fields)?;
    let input: crate::clusters::ClusterInput = serde_json::from_value(fields)
        .map_err(|_| ApiError::validation("Invalid discovered cluster fields"))?;
    input.validate(&state).await?;
    let cluster = crate::clusters::insert(&state, &mut tx, input).await?;
    sqlx::query(
        "UPDATE discovered_resources SET status='imported',cluster_id=$2,drift='[]' WHERE id=$1",
    )
    .bind(id)
    .bind(cluster)
    .execute(&mut *tx)
    .await?;
    state
        .audit_change(&mut tx, &user, "cluster.create", Some(cluster), ip.0)
        .await?;
    state
        .audit_change(&mut tx, &user, "discovery.import", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    refresh_metrics(&state).await;
    Ok(Json(
        crate::clusters::view(&state, &user, db::cluster(&state.db, cluster).await?).await?,
    ))
}
pub(super) async fn ignore(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    toggle(&state, &user, ip, db::id(&id)?, true).await
}
pub(super) async fn unignore(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    toggle(&state, &user, ip, db::id(&id)?, false).await
}
async fn toggle(
    state: &AppState,
    user: &User,
    ip: ClientIp,
    id: Uuid,
    ignore: bool,
) -> Result<Json<Value>, ApiError> {
    gate(state, user, ip).await?;
    let mut tx = state.db.begin().await?;
    let (status, cluster): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status,cluster_id FROM discovered_resources WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if !["new", "ignored"].contains(&status.as_str()) || cluster.is_some() {
        return Err(ApiError::conflict(
            "Only unimported resources can be ignored",
        ));
    }
    sqlx::query("UPDATE discovered_resources SET status=$2 WHERE id=$1")
        .bind(id)
        .bind(if ignore { "ignored" } else { "new" })
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(
            &mut tx,
            user,
            if ignore {
                "discovery.ignore"
            } else {
                "discovery.unignore"
            },
            Some(id),
            ip.0,
        )
        .await?;
    tx.commit().await?;
    refresh_metrics(state).await;
    Ok(Json(view(state, id).await?))
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SyncInput {
    password: Option<String>,
}
pub(super) async fn sync(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(body): Input<SyncInput>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let id = db::id(&id)?;
    let mut tx = state.db.begin().await?;
    let (payload, status, cluster): (Value, String, Option<Uuid>) = sqlx::query_as(
        "SELECT payload,status,cluster_id FROM discovered_resources WHERE id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let cluster = cluster
        .filter(|_| status == "imported")
        .ok_or_else(|| ApiError::conflict("Only imported resources can be synced"))?;
    // Read the cluster only after locking the inventory row so the patch is applied
    // to the current record rather than to a copy loaded before a concurrent edit.
    let current: db::ClusterRecord = sqlx::query_as("SELECT * FROM clusters WHERE id=$1")
        .bind(cluster)
        .fetch_one(&mut *tx)
        .await?;
    let mut patch = json!({"host":payload.get("host"),"port":payload.get("port"),"replica_host":payload.get("replica_host"),"replica_port":payload.get("replica_port")});
    if let Some(password) = body.password {
        patch
            .as_object_mut()
            .ok_or_else(ApiError::internal)?
            .insert("password".into(), json!(password));
    }
    crate::clusters::apply_patch(&state, &mut tx, &user, ip.0, current, patch).await?;
    let updated: db::ClusterRecord = sqlx::query_as("SELECT * FROM clusters WHERE id=$1")
        .bind(cluster)
        .fetch_one(&mut *tx)
        .await?;
    let remaining = drift::compare(&payload, Some(&json!(updated)));
    sqlx::query("UPDATE discovered_resources SET drift=$2 WHERE id=$1")
        .bind(id)
        .bind(json!(remaining))
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "discovery.sync", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    state.pools.invalidate(cluster).await;
    state.schema_cache.invalidate(&cluster).await;
    Ok(Json(
        crate::clusters::view(&state, &user, db::cluster(&state.db, cluster).await?).await?,
    ))
}

/// Refresh bounded status gauges after inventory transitions; metrics are best effort.
pub(super) async fn refresh_metrics(state: &AppState) {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT status,count(*) FROM discovered_resources GROUP BY status",
    )
    .fetch_all(&state.db)
    .await;
    match rows {
        Ok(rows) => {
            for status in ["new", "imported", "ignored", "gone"] {
                let count = rows
                    .iter()
                    .find(|(name, _)| name == status)
                    .map(|(_, count)| *count)
                    .unwrap_or(0);
                metrics::gauge!("vda_discovery_resources", "status" => status).set(count as f64);
            }
        }
        Err(error) => tracing::warn!(%error, "discovery metrics refresh failed"),
    }
}
