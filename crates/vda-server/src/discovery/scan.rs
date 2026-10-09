use super::{drift, gate, sources};
use crate::{app::AppState, auth::User, db, error::ApiError, network::ClientIp};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::{stream, StreamExt};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sqlx::Connection;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(super) async fn run_route(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let source = db::id(&id)?;
    let worker_state = state.clone();
    // A disconnected HTTP caller must not abandon the durable scan claim.
    let worker = state
        .tasks
        .spawn(async move { run(&worker_state, source, Some(user.id), Some(ip.0)).await });
    let result = tokio::time::timeout(Duration::from_secs(60), worker)
        .await
        .map_err(|_| {
            ApiError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                "upstream",
                "Scan is still finishing; check scan history",
            )
        })?
        .map_err(|_| ApiError::internal())??;
    Ok(Json(result))
}
pub(super) async fn runs(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Query(page): Query<db::Page>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let source = db::id(&id)?;
    sources::view(&state, source).await?;
    let (limit, cursor) = page.bounds()?;
    let rows:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(r) FROM discovery_runs r WHERE source_id=$1 AND ($2::timestamptz IS NULL OR (started_at,id)<($2,$3)) ORDER BY started_at DESC,id DESC LIMIT $4").bind(source).bind(cursor.as_ref().map(|c|c.created_at)).bind(cursor.as_ref().map(|c|c.id)).bind(limit+1).fetch_all(&state.db).await?;
    Ok(Json(super::resources::page(rows, limit, "started_at")?))
}
async fn recover_stale(connection: &mut sqlx::PgConnection, source: Uuid) -> Result<(), ApiError> {
    let reports: Vec<Value> = sqlx::query_scalar("UPDATE discovery_runs SET status='failed',finished_at=now(),errors='[{\"region\":null,\"message\":\"Scan abandoned by previous worker\"}]' WHERE source_id=$1 AND status='running' AND started_at<now()-interval '10 minutes' RETURNING to_jsonb(discovery_runs)").bind(source).fetch_all(&mut *connection).await?;
    for report in reports {
        scan_audit(connection, source, None, None, report).await?;
    }
    sqlx::query("UPDATE discovery_sources SET status='idle' WHERE id=$1 AND NOT EXISTS(SELECT 1 FROM discovery_runs WHERE source_id=$1 AND status='running')").bind(source).execute(connection).await?;
    Ok(())
}
async fn scan_audit(
    connection: &mut sqlx::PgConnection,
    source: Uuid,
    actor: Option<Uuid>,
    ip: Option<std::net::IpAddr>,
    report: Value,
) -> Result<(), ApiError> {
    crate::audit::persist_on(
        connection,
        &crate::audit::Event {
            id: Uuid::new_v4(),
            actor_id: actor,
            action: "discovery.scan".into(),
            target_type: Some("discovery_source".into()),
            target_id: Some(source),
            ip,
            details: report,
            created_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}
async fn recover_abandoned(state: &AppState) -> Result<(), ApiError> {
    let sources: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT source_id FROM discovery_runs WHERE status='running' AND started_at<now()-interval '10 minutes'").fetch_all(&state.db).await?;
    for source in sources {
        let mut tx = state.db.begin().await?;
        sqlx::query("SELECT id FROM discovery_sources WHERE id=$1 FOR UPDATE")
            .bind(source)
            .fetch_optional(&mut *tx)
            .await?;
        recover_stale(&mut tx, source).await?;
        tx.commit().await?;
    }
    Ok(())
}
async fn claim(state: &AppState, source: Uuid, run: Uuid) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT id FROM discovery_sources WHERE id=$1 FOR UPDATE")
        .bind(source)
        .fetch_one(&mut *tx)
        .await?;
    recover_stale(&mut tx, source).await?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM discovery_runs WHERE source_id=$1 AND status='running')",
    )
    .bind(source)
    .fetch_one(&mut *tx)
    .await?;
    if active {
        return Err(ApiError::conflict("Source scan is already running"));
    }
    let changed =
        sqlx::query("UPDATE discovery_sources SET status='running' WHERE id=$1 AND status='idle'")
            .bind(source)
            .execute(&mut *tx)
            .await?;
    if changed.rows_affected() != 1 {
        return Err(ApiError::conflict("Source scan is already running"));
    }
    sqlx::query("INSERT INTO discovery_runs(id,source_id,status) VALUES($1,$2,'running')")
        .bind(run)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub(crate) async fn run(
    state: &AppState,
    source: Uuid,
    actor: Option<Uuid>,
    ip: Option<std::net::IpAddr>,
) -> Result<Value, ApiError> {
    let run_id = Uuid::new_v4();
    claim(state, source, run_id).await?;
    let start = Instant::now();
    let result = async {
        let config = sources::config(state, source).await?;
        let outcome = tokio::time::timeout(Duration::from_secs(55), state.discovery.scan(&config))
            .await
            .unwrap_or_else(|_| vda_discovery::ScanOutcome {
                resources: Vec::new(),
                errors: config
                    .regions
                    .iter()
                    .map(|r| vda_discovery::RegionError {
                        region: Some(r.clone()),
                        message: "Scan timed out".into(),
                    })
                    .collect(),
                skipped: 0,
            });
        finish(state, source, run_id, outcome, &config.regions, actor, ip).await
    }
    .await;
    metrics::histogram!("vda_discovery_scan_duration_seconds")
        .record(start.elapsed().as_secs_f64());
    if result.is_err() {
        let mut tx = state.db.begin().await?;
        sqlx::query("SELECT id FROM discovery_sources WHERE id=$1 FOR UPDATE")
            .bind(source)
            .fetch_optional(&mut *tx)
            .await?;
        let report: Option<Value> = sqlx::query_scalar("UPDATE discovery_runs SET status='failed',finished_at=now(),errors='[{\"region\":null,\"message\":\"Scan persistence failed\"}]' WHERE id=$1 AND status='running' RETURNING to_jsonb(discovery_runs)").bind(run_id).fetch_optional(&mut *tx).await?;
        if let Some(report) = report {
            sqlx::query("UPDATE discovery_sources SET status='idle' WHERE id=$1")
                .bind(source)
                .execute(&mut *tx)
                .await?;
            scan_audit(&mut tx, source, actor, ip, report).await?;
        }
        tx.commit().await?;
    }
    result
}
async fn finish(
    state: &AppState,
    source: Uuid,
    run: Uuid,
    outcome: vda_discovery::ScanOutcome,
    regions: &[String],
    actor: Option<Uuid>,
    ip: Option<std::net::IpAddr>,
) -> Result<Value, ApiError> {
    let mut tx = state.db.begin().await?;
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT environment_tag_keys FROM discovery_sources WHERE id=$1 FOR UPDATE",
    )
    .bind(source)
    .fetch_one(&mut *tx)
    .await?;
    let status: String =
        sqlx::query_scalar("SELECT status FROM discovery_runs WHERE id=$1 FOR UPDATE")
            .bind(run)
            .fetch_one(&mut *tx)
            .await?;
    if status != "running" {
        return Err(ApiError::conflict("Scan claim expired"));
    }
    let mut new = 0_i32;
    let mut changed = 0_i32;
    let mut seen = std::collections::HashSet::new();
    let mut inventory = Vec::new();
    for mut resource in outcome.resources {
        if !regions.contains(&resource.region) || !seen.insert(resource.arn.clone()) {
            continue;
        }
        resource.suggested_environment =
            vda_discovery::suggested_environment(&resource.tags, &resource.identifier, &keys);
        let payload = json!(resource);
        crate::sanitation::json(&payload)?;
        inventory.push((resource, payload));
    }
    // One locked read replaces a lookup per resource; only the cluster fields that
    // drift detection compares are loaded, never the stored credentials.
    let arns: Vec<&str> = inventory.iter().map(|(r, _)| r.arn.as_str()).collect();
    let prior_rows:Vec<(String,Value,Option<Value>)>=sqlx::query_as("SELECT r.arn,r.payload,CASE WHEN c.id IS NULL THEN NULL ELSE jsonb_build_object('host',c.host,'port',c.port,'replica_host',c.replica_host,'replica_port',c.replica_port,'engine',c.engine) END FROM discovered_resources r LEFT JOIN clusters c ON c.id=r.cluster_id WHERE r.source_id=$1 AND r.arn=ANY($2) ORDER BY r.arn FOR UPDATE OF r").bind(source).bind(&arns).fetch_all(&mut *tx).await?;
    let prior: std::collections::HashMap<String, (Value, Option<Value>)> = prior_rows
        .into_iter()
        .map(|(arn, payload, cluster)| (arn, (payload, cluster)))
        .collect();
    for (resource, payload) in inventory {
        let prior = prior.get(&resource.arn);
        if prior.is_none() {
            new += 1;
        }
        if prior.is_some_and(|(p, _)| *p != payload) {
            changed += 1;
        }
        let drift = json!(drift::compare(
            &payload,
            prior.and_then(|(_, c)| c.as_ref())
        ));
        sqlx::query("INSERT INTO discovered_resources(id,source_id,arn,region,engine,payload,tags,drift,last_run_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(source_id,arn) DO UPDATE SET region=EXCLUDED.region,engine=EXCLUDED.engine,payload=EXCLUDED.payload,tags=EXCLUDED.tags,drift=EXCLUDED.drift,last_run_id=EXCLUDED.last_run_id,last_seen_at=clock_timestamp(),status=CASE WHEN discovered_resources.status='gone' THEN CASE WHEN discovered_resources.cluster_id IS NULL THEN 'new' ELSE 'imported' END ELSE discovered_resources.status END").bind(Uuid::new_v4()).bind(source).bind(&resource.arn).bind(&resource.region).bind(&resource.engine).bind(payload).bind(json!(resource.tags)).bind(drift).bind(run).execute(&mut *tx).await?;
    }
    let successful: Vec<String> = regions
        .iter()
        .filter(|r| {
            !outcome
                .errors
                .iter()
                .any(|e| e.region.as_ref().is_none_or(|er| er == *r))
        })
        .cloned()
        .collect();
    let gone=sqlx::query("UPDATE discovered_resources SET status='gone',drift=CASE WHEN cluster_id IS NOT NULL THEN '[\"deleted\"]'::jsonb ELSE '[]'::jsonb END WHERE source_id=$1 AND region=ANY($2) AND last_run_id IS DISTINCT FROM $3 AND status!='gone'").bind(source).bind(&successful).bind(run).execute(&mut *tx).await?.rows_affected();
    let status = if outcome.errors.is_empty() {
        "succeeded"
    } else if successful.is_empty() {
        "failed"
    } else {
        "partial"
    };
    let errors = json!(outcome.errors);
    sqlx::query("UPDATE discovery_runs SET status=$2,finished_at=clock_timestamp(),found=$3,new=$4,gone=$5,changed=$6,errors=$7 WHERE id=$1").bind(run).bind(status).bind(i32::try_from(seen.len()).map_err(|_|ApiError::internal())?).bind(new).bind(i32::try_from(gone).map_err(|_|ApiError::internal())?).bind(changed).bind(errors).execute(&mut *tx).await?;
    sqlx::query("UPDATE discovery_sources SET status='idle' WHERE id=$1")
        .bind(source)
        .execute(&mut *tx)
        .await?;
    let report: Value = sqlx::query_scalar("SELECT to_jsonb(r) FROM discovery_runs r WHERE id=$1")
        .bind(run)
        .fetch_one(&mut *tx)
        .await?;
    scan_audit(&mut tx, source, actor, ip, report.clone()).await?;
    tx.commit().await?;
    super::resources::refresh_metrics(state).await;
    Ok(report)
}
/// Leader-elected cloud scans; the dedicated connection owns the advisory lock.
pub async fn scheduler(state: AppState) {
    let Some(url) = state.config.database_url.as_ref() else {
        return;
    };
    let mut connection: Option<sqlx::PgConnection> = None;
    loop {
        let jitter = rand::random::<u64>() % 10;
        tokio::select! {_=state.stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(25+jitter))=>{}}
        if connection.is_none() {
            connection = sqlx::PgConnection::connect(url.expose_secret()).await.ok();
        }
        let Some(conn) = connection.as_mut() else {
            continue;
        };
        match sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock(762104222)")
            .fetch_one(&mut *conn)
            .await
        {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => {
                connection = None;
                continue;
            }
        }
        if let Err(error) = recover_abandoned(&state).await {
            tracing::warn!(%error, "abandoned discovery recovery failed");
        }
        let ids=sqlx::query_scalar::<_,Uuid>("SELECT s.id FROM discovery_sources s WHERE enabled AND (NOT EXISTS(SELECT 1 FROM discovery_runs r WHERE r.source_id=s.id) OR (SELECT max(started_at) FROM discovery_runs r WHERE r.source_id=s.id)<now()-s.scan_interval_minutes*interval '1 minute') ORDER BY s.created_at LIMIT 100").fetch_all(&state.db).await;
        if let Ok(ids) = ids {
            let scans = stream::iter(ids)
                .map(|id| {
                    let state = &state;
                    async move {
                        if let Err(e) = run(state, id, None, None).await {
                            tracing::warn!(source_id=%id,error=%e,"scheduled discovery failed");
                        }
                    }
                })
                .buffer_unordered(2)
                .collect::<()>();
            tokio::select! {_=state.stop.cancelled()=>{},_=scans=>{}}
        }
        if sqlx::query("SELECT pg_advisory_unlock(762104222)")
            .execute(&mut *conn)
            .await
            .is_err()
        {
            connection = None;
        }
    }
    if let Some(conn) = connection {
        let _ = conn.close().await;
    }
}
