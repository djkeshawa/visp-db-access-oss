//! Client network attribution, allowlists and target-address validation.
use crate::{app::AppState, error::ApiError};
use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};

/// Organization-level network settings persisted in the metadata store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkSettings {
    pub allowed_cidrs: Vec<String>,
    pub trust_proxy_headers: bool,
}
/// Effective request IP set by the network gate.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);
/// Validate a bounded list of network ranges.
pub fn parse_cidrs(values: &[String]) -> Result<Vec<IpNet>, ApiError> {
    if values.len() > 128 {
        return Err(ApiError::validation("Too many CIDRs"));
    }
    values
        .iter()
        .map(|s| s.parse().map_err(|_| ApiError::validation("Invalid CIDR")))
        .collect()
}
/// Empty allowlists allow all; independent allowlists are intersected by callers.
pub fn allowed(ip: IpAddr, cidrs: &[IpNet]) -> bool {
    cidrs.is_empty() || cidrs.iter().any(|net| net.contains(&ip))
}
/// Walk X-Forwarded-For from the trusted socket toward the rightmost untrusted hop.
pub fn client_ip(
    peer: IpAddr,
    forwarded: Option<&str>,
    trust: bool,
    proxies: &[IpNet],
) -> Result<IpAddr, ApiError> {
    // Fail closed: with no configured trusted proxies only the direct peer is trusted.
    if !trust || !proxies.iter().any(|n| n.contains(&peer)) {
        return Ok(peer);
    }
    let Some(value) = forwarded else {
        return Ok(peer);
    };
    if value.len() > 4096 {
        return Err(ApiError::validation("Invalid forwarded IP chain"));
    }
    let hops: Vec<IpAddr> = value
        .split(',')
        .map(|s| {
            s.trim()
                .parse()
                .map_err(|_| ApiError::validation("Invalid forwarded IP chain"))
        })
        .collect::<Result<_, _>>()?;
    for hop in hops.into_iter().rev() {
        if !proxies.iter().any(|n| n.contains(&hop)) {
            return Ok(hop);
        }
    }
    Ok(peer)
}
/// Refuse cloud metadata and link-local addresses, and loopback unless explicitly enabled.
pub fn target_ip_allowed(ip: IpAddr, allow_private: bool) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !v.is_link_local()
                && v.octets().first() != Some(&0)
                && v != std::net::Ipv4Addr::new(100, 100, 100, 200)
                && !v.is_multicast()
                && !v.is_broadcast()
                && (!v.is_loopback() || allow_private)
        }
        IpAddr::V6(v) => {
            if v.is_loopback() {
                return allow_private;
            }
            if let Some(v4) = v.to_ipv4() {
                return target_ip_allowed(v4.into(), allow_private);
            }
            let octets = v.octets();
            if octets.get(..12) == Some(&[0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0]) {
                if let Some(tail) = octets.get(12..) {
                    if let Ok(bytes) = <[u8; 4]>::try_from(tail) {
                        return target_ip_allowed(
                            std::net::Ipv4Addr::from(bytes).into(),
                            allow_private,
                        );
                    }
                }
            }
            !v.is_unicast_link_local()
                && !v.is_unspecified()
                && !v.is_multicast()
                && v.to_string() != "fd00:ec2::254"
                && (!v.is_loopback() || allow_private)
        }
    }
}
/// Resolve every address and refuse a mixed safe/unsafe DNS answer.
pub async fn validate_host(host: &str, port: u16, allow_private: bool) -> Result<(), ApiError> {
    crate::sanitation::text(host)?;
    if host.is_empty()
        || host.len() > 253
        || port == 0
        || host
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '#' | '?'))
    {
        return Err(ApiError::validation("Invalid target host or port"));
    }
    let addresses = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| ApiError::validation("Target DNS lookup timed out"))?
    .map_err(|_| ApiError::validation("Target host cannot be resolved"))?;
    let mut found = false;
    for addr in addresses {
        found = true;
        if !target_ip_allowed(addr.ip(), allow_private) {
            return Err(ApiError::validation("Target address is prohibited"));
        }
    }
    if !found {
        return Err(ApiError::validation("Target host has no addresses"));
    }
    Ok(())
}
/// Apply runtime and immutable environment allowlists before authentication.
pub async fn gate(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let result = async {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|v| v.0.ip())
            .ok_or_else(ApiError::forbidden)?;
        let settings = state.network_settings().await?;
        let trust = state.config.trust_proxy_headers || settings.trust_proxy_headers;
        let forwarded = if trust {
            forwarded_chain(req.headers())?
        } else {
            None
        };
        let ip = client_ip(
            peer,
            forwarded.as_deref(),
            trust,
            &state.config.trusted_proxy_cidrs,
        )?;
        if !allowed(ip, &state.config.allowed_cidrs)
            || !allowed(ip, &parse_cidrs(&settings.allowed_cidrs)?)
        {
            return Err(ApiError::forbidden());
        }
        req.extensions_mut().insert(ClientIp(ip));
        Ok::<_, ApiError>(())
    }
    .await;
    match result {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

/// Join all XFF header lines in wire order; reject malformed UTF-8 and oversized chains.
pub fn forwarded_chain(headers: &axum::http::HeaderMap) -> Result<Option<String>, ApiError> {
    let mut chain = String::new();
    for value in headers.get_all("x-forwarded-for") {
        if !chain.is_empty() {
            chain.push(',');
        }
        chain.push_str(
            value
                .to_str()
                .map_err(|_| ApiError::validation("Invalid forwarded IP chain"))?,
        );
        if chain.len() > 4096 {
            return Err(ApiError::validation("Invalid forwarded IP chain"));
        }
    }
    Ok(if chain.is_empty() { None } else { Some(chain) })
}

/// Enforce a cluster network policy on every target operation, including cached schema access.
pub async fn require_cluster(state: &AppState, id: uuid::Uuid, ip: IpAddr) -> Result<(), ApiError> {
    let policy = crate::db::policy(&state.db, id).await?;
    if !allowed(ip, &parse_cidrs(&policy.allowed_cidrs)?) {
        return Err(ApiError::forbidden());
    }
    Ok(())
}
