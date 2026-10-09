//! Password authentication and server-side, hashed sessions.
mod passwords;
use crate::{
    app::AppState,
    error::{ApiError, Input},
    network::ClientIp,
};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
pub use passwords::{hash_password, validate_password, verify_password};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Public user shape. Password hashes are held separately.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub org_role: String,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}
/// A login body; debug deliberately excludes credentials.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub email: String,
    pub password: String,
}
/// SHA-256 session digest; only digests are persisted.
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}
fn cookie(value: String, secure: bool) -> Cookie<'static> {
    Cookie::build(("vda_session", value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .max_age(
            std::time::Duration::from_secs(7 * 86400)
                .try_into()
                .unwrap_or_default(),
        )
        .build()
}
/// Reject mutating requests without the same-origin application's required header.
pub async fn csrf(req: Request, next: Next) -> Response {
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && req
        .headers()
        .get("x-requested-with")
        .is_none_or(|v| v != "vda")
    {
        return ApiError::forbidden().into_response();
    }
    next.run(req).await
}
/// Load active sessions and slide expiration without exceeding the absolute lifetime.
/// Sliding is skipped while more than 715 minutes remain, so busy sessions do not write per request.
pub async fn authenticate(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let jar = CookieJar::from_headers(req.headers());
    let result=async {
        let token=jar.get("vda_session").ok_or_else(ApiError::unauthenticated)?.value();
        if URL_SAFE_NO_PAD.decode(token).is_err_and(|_|true) || token.len()!=43 {return Err(ApiError::unauthenticated());}
        let user:Option<User>=sqlx::query_as(
"WITH active AS (SELECT user_id FROM sessions WHERE token_hash=$1 AND expires_at>now() AND
         absolute_expires_at>now()), slid AS (UPDATE sessions SET expires_at=LEAST(now()+interval '12 hours',absolute_expires_at)
         WHERE token_hash=$1 AND expires_at>now() AND absolute_expires_at>now()
         AND expires_at<LEAST(now()+interval '715 minutes',absolute_expires_at) RETURNING 1) SELECT
         u.id,u.email,u.name,u.org_role,u.disabled,u.created_at,u.last_login_at FROM users u JOIN active a ON
         a.user_id=u.id WHERE NOT u.disabled",
)
            .bind(token_hash(token))
.fetch_optional(&state.db).await?;
        user.ok_or_else(ApiError::unauthenticated)
    }.await;
    match result {
        Ok(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        Err(error) => error.into_response(),
    }
}
/// Authenticate a password and issue a fresh unpredictable session.
pub async fn login(
    State(state): State<AppState>,
    Extension(ip): Extension<ClientIp>,
    jar: CookieJar,
    headers: axum::http::HeaderMap,
    Input(body): Input<Login>,
) -> Result<impl IntoResponse, ApiError> {
    let email = body.email.trim().to_lowercase();
    if email.len() > 254 || body.password.len() > 1024 {
        return Err(ApiError::unauthenticated());
    }
    state.check_login_rate(ip.0, &email).await?;
    state.check_account(&email).await?;
    let record: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id,password_hash,disabled FROM users WHERE email=$1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;
    let hash = record
        .as_ref()
        .map(|r| r.1.clone())
        .unwrap_or_else(|| state.dummy_password_hash.clone());
    let permit = state
        .password_workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Try again later",
            )
        })?;
    let valid = verify_password(body.password, hash).await?;
    drop(permit);
    let id = match record {
        Some((id, _, false)) if valid => id,
        _ => {
            state.login_failed(&email).await;
            return Err(ApiError::unauthenticated());
        }
    };
    state.login_succeeded(&email).await;
    let user = crate::db::user(&state.db, id).await?;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let mut tx = state.db.begin().await?;
    sqlx::query(
"INSERT INTO sessions(token_hash,user_id,expires_at,ip,user_agent) VALUES ($1,$2,now()+interval '12 hours',$3::text::inet,$4)",
)
.bind(token_hash(&token))
.bind(id)
.bind(ip.0.to_string())
.bind(headers.get("user-agent").and_then(|v| v.to_str().ok()).map(|v|v.chars().take(1024).collect::<String>()))
.execute(&mut *tx).await?;
    sqlx::query("UPDATE users SET last_login_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "auth.login", Some(id), ip.0)
        .await?;
    tx.commit().await?;
    Ok((
        jar.add(cookie(token, state.config.cookie_secure)),
        Json(serde_json::json!({"user":user})),
    ))
}
/// Revoke the caller's session and expire the cookie.
pub async fn logout(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let mut tx = state.db.begin().await?;
    if let Some(token) = jar.get("vda_session") {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(token_hash(token.value()))
            .execute(&mut *tx)
            .await?;
    }
    state
        .audit_change(&mut tx, &user, "auth.logout", Some(user.id), ip.0)
        .await?;
    tx.commit().await?;
    let mut expired = cookie(String::new(), state.config.cookie_secure);
    expired.make_removal();
    Ok((jar.add(expired), StatusCode::NO_CONTENT))
}
/// Return the current authenticated user.
pub async fn me(Extension(user): Extension<User>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"user":user}))
}
#[derive(Deserialize)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}
/// Change a password and revoke every existing session.
pub async fn change_password(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Extension(ip): Extension<ClientIp>,
    jar: CookieJar,
    Input(body): Input<PasswordChange>,
) -> Result<impl IntoResponse, ApiError> {
    let _permit = state
        .password_workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Try again later",
            )
        })?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
    if !verify_password(body.current_password, hash.clone()).await? {
        return Err(ApiError::unauthenticated());
    }
    let replacement = hash_password(body.new_password).await?;
    let mut tx = state.db.begin().await?;
    let updated = sqlx::query("UPDATE users SET password_hash=$1 WHERE id=$2 AND password_hash=$3")
        .bind(replacement)
        .bind(user.id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::conflict("Password changed concurrently"));
    }
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    state
        .audit_change(&mut tx, &user, "auth.change_password", Some(user.id), ip.0)
        .await?;
    tx.commit().await?;
    let mut expired = cookie(String::new(), state.config.cookie_secure);
    expired.make_removal();
    Ok((jar.add(expired), StatusCode::NO_CONTENT))
}
/// Race-safe first-run administrator creation using a transaction advisory lock.
pub async fn bootstrap(state: &AppState) -> Result<(), ApiError> {
    use secrecy::ExposeSecret;
    let (Some(email), Some(password)) = (
        &state.config.bootstrap_admin_email,
        &state.config.bootstrap_admin_password,
    ) else {
        return Ok(());
    };
    let hash = hash_password(password.expose_secret().to_owned()).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(762104220)")
        .execute(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)")
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        super::admin::validate_email(email)?;
        let id = Uuid::new_v4();
        sqlx::query(
"INSERT INTO users(id,email,name,password_hash,org_role) VALUES ($1,$2,'Administrator',$3,'admin')",
)
.bind(id)
.bind(email.trim().to_lowercase())
.bind(hash)
.execute(&mut *tx).await?;
        crate::audit::persist_on(
            &mut tx,
            &crate::audit::Event {
                id: Uuid::new_v4(),
                actor_id: Some(id),
                action: "user.bootstrap".into(),
                target_type: Some("user".into()),
                target_id: Some(id),
                ip: None,
                details: serde_json::json!({}),
                created_at: Utc::now(),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("email", &self.email)
            .field("password", &"[redacted]")
            .finish()
    }
}
impl std::fmt::Debug for PasswordChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordChange([redacted])")
    }
}
