use super::gate;
use crate::{
    app::AppState,
    auth::User,
    db,
    error::{ApiError, Input},
    network::ClientIp,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use uuid::Uuid;

pub(super) const VIEW: &str = "SELECT to_jsonb(s)-'external_id_enc'-'status' || jsonb_build_object('external_id_set',s.external_id_enc IS NOT NULL,'last_scan',(SELECT to_jsonb(r) FROM discovery_runs r WHERE r.source_id=s.id ORDER BY started_at DESC,id DESC LIMIT 1)) FROM discovery_sources s";
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceInput {
    provider: String,
    name: String,
    role_arn: Option<String>,
    external_id: Option<String>,
    regions: Vec<String>,
    default_project_id: Option<Uuid>,
    #[serde(default = "tag_keys")]
    environment_tag_keys: Vec<String>,
    #[serde(default = "interval")]
    scan_interval_minutes: i32,
    #[serde(default = "enabled")]
    enabled: bool,
}
fn tag_keys() -> Vec<String> {
    vda_discovery::mapping::DEFAULT_ENVIRONMENT_TAG_KEYS
        .into_iter()
        .map(str::to_owned)
        .collect()
}
fn interval() -> i32 {
    60
}
fn enabled() -> bool {
    true
}
static ROLE: LazyLock<Result<regex::Regex, regex::Error>> = LazyLock::new(|| {
    regex::RegexBuilder::new(r"^arn:aws(-cn|-us-gov)?:iam::\d{12}:role/[\w+=,.@/-]{1,512}$")
        .unicode(false)
        .build()
});
static REGION: LazyLock<Result<regex::Regex, regex::Error>> = LazyLock::new(|| {
    regex::RegexBuilder::new(r"^[a-z]{2}(-gov)?-[a-z]+-\d$")
        .unicode(false)
        .build()
});
impl SourceInput {
    fn validate(&self) -> Result<(), ApiError> {
        let role = ROLE.as_ref().map_err(|_| ApiError::internal())?;
        let region = REGION.as_ref().map_err(|_| ApiError::internal())?;
        if self.provider != "aws" || self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(ApiError::validation("Invalid discovery provider or name"));
        }
        if self.role_arn.as_ref().is_some_and(|s| !role.is_match(s)) {
            return Err(ApiError::validation("Invalid IAM role ARN"));
        }
        if !(1..=20).contains(&self.regions.len())
            || self.regions.iter().any(|r| !region.is_match(r))
        {
            return Err(ApiError::validation("Supply 1..=20 valid AWS regions"));
        }
        if self
            .regions
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != self.regions.len()
        {
            return Err(ApiError::validation("Duplicate AWS regions"));
        }
        if !(5..=1440).contains(&self.scan_interval_minutes)
            || self.environment_tag_keys.len() > 20
            || self
                .environment_tag_keys
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 256)
            || self
                .external_id
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 1224)
        {
            return Err(ApiError::validation(
                "Invalid scan interval, environment tag keys or external ID",
            ));
        }
        Ok(())
    }
}
pub(super) async fn view(state: &AppState, id: Uuid) -> Result<Value, ApiError> {
    Ok(sqlx::query_scalar(&format!("{VIEW} WHERE s.id=$1"))
        .bind(id)
        .fetch_one(&state.db)
        .await?)
}
pub(super) async fn config(
    state: &AppState,
    id: Uuid,
) -> Result<vda_discovery::SourceConfig, ApiError> {
    Ok(config_snapshot(state, id).await?.0)
}
async fn config_snapshot(
    state: &AppState,
    id: Uuid,
) -> Result<(vda_discovery::SourceConfig, chrono::DateTime<chrono::Utc>), ApiError> {
    let (role_arn, encrypted, regions, updated_at): (
        Option<String>,
        Option<String>,
        Vec<String>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "SELECT role_arn,external_id_enc,regions,updated_at FROM discovery_sources WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let external_id = encrypted
        .map(|v| {
            state
                .crypto
                .decrypt(id, &v)
                .map(SecretString::from)
                .map_err(|_| ApiError::internal())
        })
        .transpose()?;
    Ok((
        vda_discovery::SourceConfig {
            role_arn,
            external_id,
            regions,
        },
        updated_at,
    ))
}
pub(super) async fn list(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let items: Vec<Value> =
        sqlx::query_scalar(&format!("{VIEW} ORDER BY s.created_at DESC,s.id DESC"))
            .fetch_all(&state.db)
            .await?;
    Ok(Json(json!({"items":items})))
}
pub(super) async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<SourceInput>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    body.validate()?;
    let id = Uuid::new_v4();
    let encrypted = body
        .external_id
        .as_ref()
        .map(|v| {
            state
                .crypto
                .encrypt(id, v)
                .map_err(|_| ApiError::internal())
        })
        .transpose()?;
    let last_test = state
        .discovery_tests
        .get(&(user.id, fingerprint(&input_config(&body))?))
        .await;
    let mut tx = state.db.begin().await?;
    sqlx::query("INSERT INTO discovery_sources(id,provider,name,role_arn,external_id_enc,regions,default_project_id,environment_tag_keys,scan_interval_minutes,enabled,last_test) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(id).bind(body.provider).bind(body.name).bind(body.role_arn).bind(encrypted).bind(body.regions).bind(body.default_project_id).bind(body.environment_tag_keys).bind(body.scan_interval_minutes).bind(body.enabled).bind(last_test).execute(&mut *tx).await?;
    state
        .audit_change(&mut tx, &user, "discovery.source.create", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(view(&state, id).await?))
}
pub(super) async fn update(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
    Input(patch): Input<Value>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let id = db::id(&id)?;
    let object = patch
        .as_object()
        .ok_or_else(|| ApiError::validation("Expected an object"))?;
    let mut tx = state.db.begin().await?;
    let (mut merged, old_enc, status):(Value,Option<String>,String)=sqlx::query_as("SELECT to_jsonb(s)-'id'-'created_at'-'updated_at'-'external_id_enc'-'status'-'last_test',external_id_enc,status FROM discovery_sources s WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
    if status == "running" {
        return Err(ApiError::conflict("Source scan is running"));
    }
    let mut old_input: SourceInput =
        serde_json::from_value(merged.clone()).map_err(|_| ApiError::internal())?;
    old_input.external_id = old_enc
        .as_ref()
        .map(|v| {
            state
                .crypto
                .decrypt(id, v)
                .map_err(|_| ApiError::internal())
        })
        .transpose()?;
    let old_fingerprint = fingerprint(&input_config(&old_input))?;
    for (k, v) in object {
        merged
            .as_object_mut()
            .ok_or_else(ApiError::internal)?
            .insert(k.clone(), v.clone());
    }
    let body: SourceInput = serde_json::from_value(merged)
        .map_err(|_| ApiError::validation("Invalid source fields"))?;
    body.validate()?;
    let enc = if object.contains_key("external_id") {
        body.external_id
            .as_ref()
            .map(|v| {
                state
                    .crypto
                    .encrypt(id, v)
                    .map_err(|_| ApiError::internal())
            })
            .transpose()?
    } else {
        old_enc
    };
    let mut new_config = input_config(&body);
    new_config.external_id = enc
        .as_ref()
        .map(|v| {
            state
                .crypto
                .decrypt(id, v)
                .map(SecretString::from)
                .map_err(|_| ApiError::internal())
        })
        .transpose()?;
    let new_fingerprint = fingerprint(&new_config)?;
    let tested = state.discovery_tests.get(&(user.id, new_fingerprint)).await;
    sqlx::query("UPDATE discovery_sources SET provider=$2,name=$3,role_arn=$4,external_id_enc=$5,regions=$6,default_project_id=$7,environment_tag_keys=$8,scan_interval_minutes=$9,enabled=$10,last_test=CASE WHEN $12 THEN $11::jsonb WHEN last_test IS NULL THEN $11 ELSE last_test END,updated_at=clock_timestamp() WHERE id=$1").bind(id).bind(body.provider).bind(body.name).bind(body.role_arn).bind(enc).bind(body.regions).bind(body.default_project_id).bind(body.environment_tag_keys).bind(body.scan_interval_minutes).bind(body.enabled).bind(tested).bind(old_fingerprint != new_fingerprint).execute(&mut *tx).await?;
    state
        .audit_change(&mut tx, &user, "discovery.source.update", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok(Json(view(&state, id).await?))
}
pub(super) async fn delete(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    gate(&state, &user, ip).await?;
    let id = db::id(&id)?;
    let mut tx = state.db.begin().await?;
    let status: String =
        sqlx::query_scalar("SELECT status FROM discovery_sources WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if status == "running" {
        return Err(ApiError::conflict("Source scan is running"));
    }
    sqlx::query("DELETE FROM discovery_sources WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "discovery.source.delete", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    super::resources::refresh_metrics(&state).await;
    Ok(StatusCode::NO_CONTENT)
}
fn input_config(body: &SourceInput) -> vda_discovery::SourceConfig {
    vda_discovery::SourceConfig {
        role_arn: body.role_arn.clone(),
        external_id: body.external_id.clone().map(SecretString::from),
        regions: body.regions.clone(),
    }
}
fn fingerprint(config: &vda_discovery::SourceConfig) -> Result<[u8; 32], ApiError> {
    // Length-delimited serialization avoids collisions from field boundaries;
    // the secret is hashed, never put in the cache key as plaintext or persisted.
    let bytes = serde_json::to_vec(&json!({"role":config.role_arn,"external":config.external_id.as_ref().map(|v|v.expose_secret()),"regions":config.regions})).map_err(|_| ApiError::internal())?;
    Ok(Sha256::digest(bytes).into())
}
async fn run_test(
    state: &AppState,
    user: &User,
    config: &vda_discovery::SourceConfig,
) -> Result<(Value, Value), ApiError> {
    let report = match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        state.discovery.test(config),
    )
    .await
    {
        Ok(report) => report,
        Err(_) => vda_discovery::TestReport {
            ok: false,
            account_id: None,
            identity_arn: None,
            regions: config
                .regions
                .iter()
                .map(|region| vda_discovery::RegionTest {
                    region: region.clone(),
                    ok: false,
                    error: Some("Discovery test timed out".into()),
                })
                .collect(),
        },
    };
    let summary = json!({"ok":report.ok,"account_id":report.account_id,"identity_arn":report.identity_arn,"tested_at":chrono::Utc::now()});
    state
        .discovery_tests
        .insert((user.id, fingerprint(config)?), summary.clone())
        .await;
    Ok((json!(report), summary))
}
pub(super) async fn test_draft(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Input(body): Input<SourceInput>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    body.validate()?;
    if let Some(project) = body.default_project_id {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=$1)")
            .bind(project)
            .fetch_one(&state.db)
            .await?;
        if !exists {
            return Err(ApiError::validation("Default project does not exist"));
        }
    }
    let (report, _) = run_test(&state, &user, &input_config(&body)).await?;
    Ok(Json(report))
}
pub(super) async fn test(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    gate(&state, &user, ip).await?;
    let id = db::id(&id)?;
    let (config, version) = config_snapshot(&state, id).await?;
    let (report, summary) = run_test(&state, &user, &config).await?;
    // A test in flight must not label an edited configuration as verified.
    sqlx::query("UPDATE discovery_sources SET last_test=$2 WHERE id=$1 AND updated_at=$3")
        .bind(id)
        .bind(summary)
        .bind(version)
        .execute(&state.db)
        .await?;
    Ok(Json(report))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn iam_role_validation_accepts_partitions_and_bounded_ascii_names() {
        let mut source: SourceInput = serde_json::from_value(json!({
            "provider":"aws", "name":"inventory", "regions":["us-east-1"]
        }))
        .unwrap();
        for partition in ["aws", "aws-cn", "aws-us-gov"] {
            source.role_arn = Some(format!(
                "arn:{partition}:iam::123456789012:role/{}",
                "a".repeat(512)
            ));
            assert!(source.validate().is_ok());
        }
        for name in ["a".repeat(513), "é".into(), "".into()] {
            source.role_arn = Some(format!("arn:aws:iam::123456789012:role/{name}"));
            assert!(source.validate().is_err());
        }
    }
}
