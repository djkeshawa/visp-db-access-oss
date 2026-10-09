use std::{collections::BTreeSet, time::Duration};

use async_trait::async_trait;
use aws_config::{sts::AssumeRoleProvider, BehaviorVersion, SdkConfig};
use aws_sdk_rds::{
    config::{retry::RetryConfig, timeout::TimeoutConfig, Region, SharedCredentialsProvider},
    Client,
};
use futures::{stream, StreamExt};
use secrecy::ExposeSecret;

use crate::{mapping, Provider, RegionError, RegionTest, ScanOutcome, SourceConfig, TestReport};

/// Read-only AWS implementation with a single ambient SDK configuration.
#[derive(Clone, Debug)]
pub struct AwsProvider {
    ambient: SdkConfig,
}

impl AwsProvider {
    /// Load the ambient credential chain once. Credentials resolve lazily on calls.
    /// Regions belong to each source; an explicit bootstrap region avoids IMDS
    /// region lookup blocking application startup on non-AWS hosts.
    pub async fn new() -> Self {
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".into());
        let ambient = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region))
            .retry_config(retries())
            .timeout_config(timeouts())
            .load()
            .await;
        Self { ambient }
    }

    /// Use an explicitly provided SDK configuration (useful for network-free tests).
    pub fn from_config(ambient: SdkConfig) -> Self {
        Self { ambient }
    }

    async fn source_config(&self, source: &SourceConfig) -> SdkConfig {
        let mut base = self
            .ambient
            .clone()
            .into_builder()
            .retry_config(retries())
            .timeout_config(timeouts());
        if let Some(region) = source.regions.first() {
            base = base.region(Region::new(region.clone()));
        }
        let base = base.build();
        let Some(role) = &source.role_arn else {
            return base;
        };
        let mut role_provider = AssumeRoleProvider::builder(role)
            .configure(&base)
            .session_name("visp-db-access")
            .session_length(Duration::from_secs(3600));
        if let Some(external_id) = &source.external_id {
            role_provider = role_provider.external_id(external_id.expose_secret());
        }
        let credentials = role_provider.build().await;
        base.into_builder()
            .credentials_provider(SharedCredentialsProvider::new(credentials))
            .build()
    }

    fn regional_clients(config: &SdkConfig, regions: &[String]) -> Vec<(String, Client)> {
        regions
            .iter()
            .map(|region| {
                let client = Client::from_conf(
                    aws_sdk_rds::config::Builder::from(config)
                        .region(Region::new(region.clone()))
                        .retry_config(retries())
                        .timeout_config(timeouts())
                        .build(),
                );
                (region.clone(), client)
            })
            .collect()
    }
}

fn retries() -> RetryConfig {
    RetryConfig::standard().with_max_attempts(3)
}

fn timeouts() -> TimeoutConfig {
    TimeoutConfig::builder()
        .operation_timeout(Duration::from_secs(20))
        .operation_attempt_timeout(Duration::from_secs(10))
        .build()
}

#[async_trait]
impl Provider for AwsProvider {
    async fn test(&self, source: &SourceConfig) -> TestReport {
        let config = self.source_config(source).await;
        let clients = Self::regional_clients(&config, &source.regions);
        test_clients(aws_sdk_sts::Client::new(&config), clients).await
    }

    async fn scan(&self, source: &SourceConfig) -> ScanOutcome {
        let config = self.source_config(source).await;
        scan_clients(Self::regional_clients(&config, &source.regions)).await
    }
}

async fn test_clients(
    identity_client: aws_sdk_sts::Client,
    clients: Vec<(String, Client)>,
) -> TestReport {
    // Identity and regional probes are independent, so they run together.
    let identity = async { identity_client.get_caller_identity().send().await };
    let probes = stream::iter(clients)
        .map(|(region, client)| async move {
            let error = client
                .describe_db_instances()
                .max_records(20)
                .send()
                .await
                .err()
                .map(|_| describe_error("DescribeDBInstances"));
            RegionTest {
                region,
                ok: error.is_none(),
                error,
            }
        })
        .buffer_unordered(4)
        .collect::<Vec<_>>();
    let (identity, mut regions) = tokio::join!(identity, probes);
    let (account_id, identity_arn, identity_ok) = match identity {
        Ok(identity) => (
            identity.account().map(String::from),
            identity.arn().map(String::from),
            true,
        ),
        Err(_) => (None, None, false),
    };
    if !identity_ok {
        for region in &mut regions {
            region.ok = false;
            region
                .error
                .get_or_insert_with(|| "Unable to resolve AWS caller identity".into());
        }
    }
    regions.sort_by(|a, b| a.region.cmp(&b.region));
    TestReport {
        ok: identity_ok && regions.iter().all(|region| region.ok),
        account_id,
        identity_arn,
        regions,
    }
}

fn describe_error(operation: &str) -> String {
    format!("{operation} failed; check credentials, permissions, and region reachability")
}

async fn scan_clients(clients: Vec<(String, Client)>) -> ScanOutcome {
    scan_clients_before(clients, Duration::from_secs(50)).await
}

async fn scan_clients_before(clients: Vec<(String, Client)>, budget: Duration) -> ScanOutcome {
    let mut pending: BTreeSet<String> = clients.iter().map(|(region, _)| region.clone()).collect();
    let deadline = tokio::time::Instant::now() + budget;
    let mut results = stream::iter(clients)
        .map(|(region, client)| async move {
            let result = match scan_region(&client, &region).await {
                Ok(result) => result,
                Err(message) => (
                    ScanOutcome {
                        errors: vec![RegionError {
                            region: Some(region.clone()),
                            message,
                        }],
                        ..ScanOutcome::default()
                    },
                    Vec::new(),
                ),
            };
            (region, result)
        })
        .buffer_unordered(4);
    let mut outcome = ScanOutcome::default();
    let mut instances = Vec::new();
    loop {
        tokio::select! {
            result = results.next() => match result {
                Some((region, (result, region_instances))) => {
                    pending.remove(&region);
                    instances.extend(region_instances);
                    outcome.resources.extend(result.resources);
                    outcome.errors.extend(result.errors);
                    outcome.skipped += result.skipped;
                }
                None => break,
            },
            _ = tokio::time::sleep_until(deadline) => {
                outcome.errors.extend(pending.into_iter().map(|region| RegionError {
                    region: Some(region),
                    message: "Region scan exceeded the source scan time budget".into(),
                }));
                break;
            }
        }
    }
    mapping::attach_replicas(&mut outcome.resources, &instances);
    outcome.resources.sort_by(|a, b| a.arn.cmp(&b.arn));
    outcome.errors.sort_by(|a, b| a.region.cmp(&b.region));
    outcome
}

async fn scan_region(
    client: &Client,
    region: &str,
) -> Result<(ScanOutcome, Vec<aws_sdk_rds::types::DbInstance>), String> {
    let mut clusters = Vec::new();
    let mut cluster_pages = client.describe_db_clusters().into_paginator().send();
    while let Some(page) = cluster_pages.next().await {
        let page = page.map_err(|_| describe_error("DescribeDBClusters"))?;
        clusters.extend(page.db_clusters().iter().cloned());
    }
    let mut instances = Vec::new();
    let mut instance_pages = client.describe_db_instances().into_paginator().send();
    while let Some(page) = instance_pages.next().await {
        let page = page.map_err(|_| describe_error("DescribeDBInstances"))?;
        instances.extend(page.db_instances().iter().cloned());
    }
    Ok((
        mapping::map_region(&clusters, &instances, region),
        instances,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
