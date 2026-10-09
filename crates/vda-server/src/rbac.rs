//! Effective access from organization role and unexpired project/cluster grants.
use crate::{auth::User, db::ClusterRecord, error::ApiError};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use vda_guard::AccessLevel;
/// Resolve the maximum level while ignoring expired grants.
pub fn resolve_level(
    admin: bool,
    grants: impl IntoIterator<Item = (AccessLevel, Option<DateTime<Utc>>)>,
    now: DateTime<Utc>,
) -> Option<AccessLevel> {
    if admin {
        return Some(AccessLevel::Admin);
    }
    grants
        .into_iter()
        .filter(|(_, expiry)| expiry.is_none_or(|e| e > now))
        .map(|(level, _)| level)
        .max()
}
/// Require organization administrator privileges.
pub fn require_admin(user: &User) -> Result<(), ApiError> {
    if user.org_role == "admin" {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}
/// Resolve persisted access for a cluster.
pub async fn effective(
    pool: &sqlx::PgPool,
    user: &User,
    cluster: &ClusterRecord,
) -> Result<Option<AccessLevel>, ApiError> {
    if user.org_role == "admin" {
        return Ok(Some(AccessLevel::Admin));
    }
    let levels:Vec<String>=sqlx::query_scalar(
"SELECT level FROM grants WHERE user_id=$1 AND ((scope='cluster' AND scope_id=$2) OR (scope='project'
         AND scope_id=$3)) AND (expires_at IS NULL OR expires_at>now())",
)
.bind(user.id)
.bind(cluster.id)
.bind(cluster.project_id)
.fetch_all(pool).await?;
    Ok(levels
        .into_iter()
        .filter_map(|l| match l.as_str() {
            "read" => Some(AccessLevel::Read),
            "write" => Some(AccessLevel::Write),
            "admin" => Some(AccessLevel::Admin),
            _ => None,
        })
        .max())
}
/// Fetch a cluster and require a minimum effective level.
pub async fn require_level(
    pool: &sqlx::PgPool,
    user: &User,
    id: Uuid,
    level: AccessLevel,
) -> Result<ClusterRecord, ApiError> {
    Ok(require_level_effective(pool, user, id, level).await?.0)
}
/// Like [`require_level`], but also returns the resolved level so callers need no second lookup.
pub async fn require_level_effective(
    pool: &sqlx::PgPool,
    user: &User,
    id: Uuid,
    level: AccessLevel,
) -> Result<(ClusterRecord, AccessLevel), ApiError> {
    let cluster = crate::db::cluster(pool, id).await?;
    match effective(pool, user, &cluster).await? {
        Some(effective) if effective >= level => Ok((cluster, effective)),
        _ => Err(ApiError::forbidden()),
    }
}
/// Authorization predicate used by scoped list queries.
pub const ACCESS_SQL: &str = "
    (u.org_role='admin' OR EXISTS (
        SELECT 1 FROM grants g WHERE g.user_id=u.id
        AND (g.expires_at IS NULL OR g.expires_at>now())
        AND ((g.scope='cluster' AND g.scope_id=c.id)
            OR (g.scope='project' AND g.scope_id=c.project_id))))";
