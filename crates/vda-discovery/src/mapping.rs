//! Pure transformations from AWS SDK records into safe discovery inventory.

use std::collections::{BTreeMap, BTreeSet};

use aws_sdk_rds::types::{DbCluster, DbInstance, Tag};

use crate::{DiscoveredDb, ScanOutcome};

/// Tag keys consulted, in order, when a source does not configure its own.
pub const DEFAULT_ENVIRONMENT_TAG_KEYS: [&str; 3] = ["environment", "env", "stage"];

/// Map only database engines supported by the gateway.
pub fn supported_engine(engine: &str) -> Option<&'static str> {
    match engine {
        "postgres" | "aurora-postgresql" => Some("postgres"),
        "mysql" | "aurora-mysql" | "aurora" | "mariadb" => Some("mysql"),
        _ => None,
    }
}

/// Infer an environment by ordered, case-insensitive tag keys, then name tokens.
/// Unknown names are production, which selects the safest default policy.
pub fn suggested_environment(
    tags: &BTreeMap<String, String>,
    identifier: &str,
    tag_keys: &[String],
) -> String {
    for key in tag_keys {
        if let Some((_, value)) = tags.iter().find(|(tag, _)| tag.eq_ignore_ascii_case(key)) {
            if let Some(environment) = normalized_environment(value) {
                return environment.to_owned();
            }
            break;
        }
    }
    identifier
        .split(['-', '_', '.', '/'])
        .find_map(normalized_environment)
        .unwrap_or("production")
        .to_owned()
}

fn normalized_environment(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "prod" | "production" | "prd" | "live" => Some("production"),
        "stg" | "stage" | "staging" | "uat" | "preprod" | "qa" => Some("staging"),
        "dev" | "development" | "test" | "sandbox" => Some("development"),
        _ => None,
    }
}

fn tags(values: &[Tag]) -> BTreeMap<String, String> {
    values
        .iter()
        .filter_map(|tag| Some((tag.key()?.to_owned(), tag.value()?.to_owned())))
        .collect()
}

fn account_id(arn: &str) -> String {
    arn.split(':').nth(4).unwrap_or_default().to_owned()
}

fn valid_port(port: i32) -> Option<u16> {
    u16::try_from(port).ok().filter(|port| *port != 0)
}

fn environment(tags: &BTreeMap<String, String>, identifier: &str) -> String {
    let keys = DEFAULT_ENVIRONMENT_TAG_KEYS.map(String::from);
    suggested_environment(tags, identifier, &keys)
}

/// Map an Aurora writer and its reader endpoint (only when a reader exists).
pub fn map_cluster(cluster: &DbCluster, region: &str) -> Option<DiscoveredDb> {
    let detail = cluster.engine()?;
    if !detail.starts_with("aurora") {
        return None;
    }
    let engine = supported_engine(detail)?;
    let arn = cluster.db_cluster_arn()?;
    let identifier = cluster.db_cluster_identifier()?;
    let host = cluster.endpoint()?.trim();
    if host.is_empty() {
        return None;
    }
    let port = valid_port(cluster.port()?)?;
    let has_reader = cluster
        .db_cluster_members()
        .iter()
        .any(|member| member.is_cluster_writer() == Some(false));
    let replica_host = cluster
        .reader_endpoint()
        .filter(|host| has_reader && !host.is_empty())
        .map(String::from);
    let tags = tags(cluster.tag_list());
    Some(DiscoveredDb {
        kind: "aurora_cluster".into(),
        arn: arn.into(),
        identifier: identifier.into(),
        account_id: account_id(arn),
        region: region.into(),
        engine: engine.into(),
        engine_detail: detail.into(),
        engine_version: cluster.engine_version().unwrap_or_default().into(),
        host: host.into(),
        port,
        replica_port: replica_host.as_ref().map(|_| port),
        replica_host,
        database: cluster.database_name().map(String::from),
        status_detail: cluster.status().unwrap_or_default().into(),
        publicly_accessible: cluster.publicly_accessible().unwrap_or(false),
        encrypted: cluster.storage_encrypted().unwrap_or(false),
        multi_az: cluster.multi_az().unwrap_or(false),
        iam_auth_enabled: cluster
            .iam_database_authentication_enabled()
            .unwrap_or(false),
        vpc_id: None,
        suggested_environment: environment(&tags, identifier),
        tags,
    })
}

/// Map a standalone RDS endpoint. Creating records without endpoints are skipped.
pub fn map_instance(instance: &DbInstance, region: &str) -> Option<DiscoveredDb> {
    if instance.db_cluster_identifier().is_some()
        || instance
            .read_replica_source_db_instance_identifier()
            .is_some()
    {
        return None;
    }
    let detail = instance.engine()?;
    let engine = supported_engine(detail)?;
    let arn = instance.db_instance_arn()?;
    let identifier = instance.db_instance_identifier()?;
    let endpoint = instance.endpoint()?;
    let host = endpoint.address()?.trim();
    if host.is_empty() {
        return None;
    }
    let port = valid_port(endpoint.port()?)?;
    let tags = tags(instance.tag_list());
    Some(DiscoveredDb {
        kind: "rds_instance".into(),
        arn: arn.into(),
        identifier: identifier.into(),
        account_id: account_id(arn),
        region: region.into(),
        engine: engine.into(),
        engine_detail: detail.into(),
        engine_version: instance.engine_version().unwrap_or_default().into(),
        host: host.into(),
        port,
        replica_host: None,
        replica_port: None,
        database: instance.db_name().map(String::from),
        status_detail: instance.db_instance_status().unwrap_or_default().into(),
        publicly_accessible: instance.publicly_accessible().unwrap_or(false),
        encrypted: instance.storage_encrypted().unwrap_or(false),
        multi_az: instance.multi_az().unwrap_or(false),
        iam_auth_enabled: instance
            .iam_database_authentication_enabled()
            .unwrap_or(false),
        vpc_id: instance
            .db_subnet_group()
            .and_then(|group| group.vpc_id())
            .map(String::from),
        suggested_environment: environment(&tags, identifier),
        tags,
    })
}

/// Combine a complete region's pages, deduplicate members, and attach replicas.
/// Replica selection is lexical by identifier, independent of AWS response order.
pub fn map_region(clusters: &[DbCluster], instances: &[DbInstance], region: &str) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    let mut seen = BTreeSet::new();
    let mut members: BTreeMap<&str, Vec<&DbInstance>> = BTreeMap::new();
    for instance in instances {
        if let Some(cluster_id) = instance.db_cluster_identifier() {
            members.entry(cluster_id).or_default().push(instance);
        }
    }
    for cluster in clusters {
        match map_cluster(cluster, region) {
            Some(mut resource) => {
                let cluster_members = members.get(resource.identifier.as_str());
                resource.vpc_id = cluster_members
                    .into_iter()
                    .flatten()
                    .find_map(|instance| {
                        instance.db_subnet_group().and_then(|group| group.vpc_id())
                    })
                    .map(String::from);
                // Aurora reports accessibility on member instances in some API versions.
                resource.publicly_accessible |= cluster_members
                    .into_iter()
                    .flatten()
                    .any(|instance| instance.publicly_accessible().unwrap_or(false));
                if seen.insert(resource.arn.clone()) {
                    outcome.resources.push(resource);
                }
            }
            None => outcome.skipped += 1,
        }
    }
    for instance in instances {
        match map_instance(instance, region) {
            Some(resource) => {
                if seen.insert(resource.arn.clone()) {
                    outcome.resources.push(resource);
                }
            }
            None => {
                if instance.db_cluster_identifier().is_none()
                    && instance
                        .read_replica_source_db_instance_identifier()
                        .is_none()
                {
                    outcome.skipped += 1;
                }
            }
        }
    }
    attach_replicas(&mut outcome.resources, instances);
    outcome.resources.sort_by(|a, b| a.arn.cmp(&b.arn));
    tracing::debug!(
        region,
        skipped = outcome.skipped,
        "Mapped AWS database inventory"
    );
    outcome
}

/// Attach same-region and cross-region replicas from all successful inventories.
/// Relative source identifiers are scoped by account and region using the ARN.
pub(crate) fn attach_replicas(resources: &mut [DiscoveredDb], instances: &[DbInstance]) {
    let mut replicas: BTreeMap<String, Vec<&DbInstance>> = BTreeMap::new();
    for instance in instances {
        let Some(source) = instance.read_replica_source_db_instance_identifier() else {
            continue;
        };
        let source_arn = if source.starts_with("arn:") {
            source.to_owned()
        } else {
            let Some((prefix, _)) = instance
                .db_instance_arn()
                .and_then(|arn| arn.rsplit_once(':'))
            else {
                continue;
            };
            format!("{prefix}:{source}")
        };
        replicas.entry(source_arn).or_default().push(instance);
    }
    for values in replicas.values_mut() {
        values.sort_by_key(|instance| {
            (
                instance.db_instance_identifier(),
                instance.db_instance_arn(),
            )
        });
    }
    for resource in resources
        .iter_mut()
        .filter(|resource| resource.kind == "rds_instance")
    {
        let replica = replicas
            .get(&resource.arn)
            .into_iter()
            .flatten()
            .find_map(|replica| {
                if replica.db_instance_status() != Some("available") {
                    return None;
                }
                let endpoint = replica.endpoint()?;
                let host = endpoint.address()?.trim();
                if host.is_empty() || supported_engine(replica.engine()?)? != resource.engine {
                    return None;
                }
                Some((host.to_owned(), valid_port(endpoint.port()?)?))
            });
        if let Some((host, port)) = replica {
            resource.replica_host = Some(host);
            resource.replica_port = Some(port);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
