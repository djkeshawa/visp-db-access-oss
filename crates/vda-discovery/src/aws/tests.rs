use super::*;
use aws_sdk_rds::{
    operation::{
        describe_db_clusters::DescribeDbClustersOutput,
        describe_db_instances::{DescribeDBInstancesError, DescribeDbInstancesOutput},
    },
    types::{DbCluster, DbClusterMember, DbInstance, Endpoint},
};
use aws_smithy_mocks::{mock, mock_client, RuleMode};

fn cluster() -> DbCluster {
    DbCluster::builder()
        .db_cluster_identifier("shop")
        .db_cluster_arn("arn:aws:rds:us-east-1:123456789012:cluster:shop")
        .engine("aurora-postgresql")
        .endpoint("writer.example.com")
        .reader_endpoint("reader.example.com")
        .port(5432)
        .db_cluster_members(DbClusterMember::builder().is_cluster_writer(false).build())
        .build()
}

fn instance(name: &str) -> DbInstance {
    instance_builder(name).build()
}

fn instance_builder(name: &str) -> aws_sdk_rds::types::builders::DbInstanceBuilder {
    DbInstance::builder()
        .db_instance_identifier(name)
        .db_instance_arn(format!("arn:aws:rds:us-east-1:123456789012:db:{name}"))
        .engine("mysql")
        .db_instance_status("available")
        .endpoint(
            Endpoint::builder()
                .address(format!("{name}.example.com"))
                .port(3306)
                .build(),
        )
}

#[tokio::test]
async fn sdk_paginators_collect_all_pages_and_deduplicate_members_and_replicas() {
    let cluster_first = mock!(aws_sdk_rds::Client::describe_db_clusters)
        .match_requests(|request| request.marker().is_none())
        .then_output(|| {
            DescribeDbClustersOutput::builder()
                .db_clusters(cluster())
                .marker("cluster-page-2")
                .build()
        });
    let cluster_second = mock!(aws_sdk_rds::Client::describe_db_clusters)
        .match_requests(|request| request.marker() == Some("cluster-page-2"))
        .then_output(|| DescribeDbClustersOutput::builder().build());
    let instance_first = mock!(aws_sdk_rds::Client::describe_db_instances)
        .match_requests(|request| request.marker().is_none())
        .then_output(|| {
            DescribeDbInstancesOutput::builder()
                .db_instances(instance("orders"))
                .db_instances(
                    instance_builder("aurora-member")
                        .db_cluster_identifier("shop")
                        .build(),
                )
                .marker("instance-page-2")
                .build()
        });
    let instance_second = mock!(aws_sdk_rds::Client::describe_db_instances)
        .match_requests(|request| request.marker() == Some("instance-page-2"))
        .then_output(|| {
            DescribeDbInstancesOutput::builder()
                .db_instances(
                    instance_builder("orders-replica")
                        .read_replica_source_db_instance_identifier("orders")
                        .build(),
                )
                .build()
        });
    let client = mock_client!(
        aws_sdk_rds,
        RuleMode::MatchAny,
        [
            &cluster_first,
            &cluster_second,
            &instance_first,
            &instance_second
        ]
    );
    let outcome = scan_clients(vec![("us-east-1".into(), client)]).await;
    assert!(outcome.errors.is_empty());
    assert_eq!(outcome.resources.len(), 2);
    let instance = outcome
        .resources
        .iter()
        .find(|resource| resource.identifier == "orders")
        .unwrap();
    assert_eq!(
        instance.replica_host.as_deref(),
        Some("orders-replica.example.com")
    );
    let aurora = outcome
        .resources
        .iter()
        .find(|resource| resource.identifier == "shop")
        .unwrap();
    assert_eq!(aurora.replica_host.as_deref(), Some("reader.example.com"));
    assert_eq!(cluster_first.num_calls(), 1);
    assert_eq!(cluster_second.num_calls(), 1);
    assert_eq!(instance_first.num_calls(), 1);
    assert_eq!(instance_second.num_calls(), 1);
}

#[tokio::test]
async fn failed_region_discards_incomplete_pages_and_retains_other_regions() {
    let cluster_rule = mock!(aws_sdk_rds::Client::describe_db_clusters).then_output(|| {
        DescribeDbClustersOutput::builder()
            .db_clusters(cluster())
            .build()
    });
    let instance_good = mock!(aws_sdk_rds::Client::describe_db_instances).then_output(|| {
        DescribeDbInstancesOutput::builder()
            .db_instances(instance("orders"))
            .build()
    });
    let good = mock_client!(
        aws_sdk_rds,
        RuleMode::MatchAny,
        [&cluster_rule, &instance_good]
    );
    let cluster_bad = mock!(aws_sdk_rds::Client::describe_db_clusters).then_output(|| {
        DescribeDbClustersOutput::builder()
            .db_clusters(cluster())
            .build()
    });
    let instance_bad = mock!(aws_sdk_rds::Client::describe_db_instances)
        .then_error(|| DescribeDBInstancesError::unhandled("secret-value-must-not-escape"));
    let bad = mock_client!(
        aws_sdk_rds,
        RuleMode::MatchAny,
        [&cluster_bad, &instance_bad]
    );
    let outcome = scan_clients(vec![("eu-west-1".into(), bad), ("us-east-1".into(), good)]).await;
    assert_eq!(outcome.resources.len(), 2);
    assert_eq!(outcome.errors.len(), 1);
    assert_eq!(
        outcome.errors.first().unwrap().region.as_deref(),
        Some("eu-west-1")
    );
    assert!(!outcome
        .errors
        .first()
        .unwrap()
        .message
        .contains("secret-value"));
}

#[tokio::test]
async fn replicas_in_another_successful_region_attach_to_the_source_arn() {
    let empty = mock!(aws_sdk_rds::Client::describe_db_clusters)
        .then_output(|| DescribeDbClustersOutput::builder().build());
    let primary_rule = mock!(aws_sdk_rds::Client::describe_db_instances).then_output(|| {
        DescribeDbInstancesOutput::builder()
            .db_instances(instance("orders"))
            .build()
    });
    let primary = mock_client!(aws_sdk_rds, RuleMode::MatchAny, [&empty, &primary_rule]);
    let empty_replica = mock!(aws_sdk_rds::Client::describe_db_clusters)
        .then_output(|| DescribeDbClustersOutput::builder().build());
    let replica_rule = mock!(aws_sdk_rds::Client::describe_db_instances).then_output(|| {
        DescribeDbInstancesOutput::builder()
            .db_instances(
                instance_builder("replica")
                    .db_instance_arn("arn:aws:rds:eu-west-1:123456789012:db:replica")
                    .read_replica_source_db_instance_identifier(
                        "arn:aws:rds:us-east-1:123456789012:db:orders",
                    )
                    .build(),
            )
            .build()
    });
    let replica = mock_client!(
        aws_sdk_rds,
        RuleMode::MatchAny,
        [&empty_replica, &replica_rule]
    );
    let outcome = scan_clients(vec![
        ("us-east-1".into(), primary),
        ("eu-west-1".into(), replica),
    ])
    .await;
    assert!(outcome.errors.is_empty());
    assert_eq!(outcome.resources.len(), 1);
    assert_eq!(
        outcome.resources.first().unwrap().replica_host.as_deref(),
        Some("replica.example.com")
    );
}

#[tokio::test]
async fn source_test_reports_identity_and_regional_read_permissions() {
    let identity_rule = mock!(aws_sdk_sts::Client::get_caller_identity).then_output(|| {
        aws_sdk_sts::operation::get_caller_identity::GetCallerIdentityOutput::builder()
            .account("123456789012")
            .arn("arn:aws:iam::123456789012:role/discovery")
            .build()
    });
    let identity = mock_client!(aws_sdk_sts, [&identity_rule]);
    let good_rule = mock!(aws_sdk_rds::Client::describe_db_instances)
        .match_requests(|request| request.max_records() == Some(20))
        .then_output(|| DescribeDbInstancesOutput::builder().build());
    let good = mock_client!(aws_sdk_rds, [&good_rule]);
    let bad_rule = mock!(aws_sdk_rds::Client::describe_db_instances)
        .then_error(|| DescribeDBInstancesError::unhandled("credentials-secret"));
    let bad = mock_client!(aws_sdk_rds, [&bad_rule]);
    let report = test_clients(
        identity,
        vec![("us-east-1".into(), good), ("eu-west-1".into(), bad)],
    )
    .await;
    assert!(!report.ok);
    assert_eq!(report.account_id.as_deref(), Some("123456789012"));
    assert!(report
        .regions
        .iter()
        .any(|region| region.region == "us-east-1" && region.ok));
    assert!(report
        .regions
        .iter()
        .any(|region| region.region == "eu-west-1" && !region.ok));
    assert_eq!(identity_rule.num_calls(), 1);
    assert_eq!(good_rule.num_calls(), 1);
    assert!(!report
        .regions
        .iter()
        .filter_map(|region| region.error.as_ref())
        .any(|error| error.contains("credentials-secret")));
}

#[tokio::test]
async fn source_test_probes_regions_while_identity_lookup_is_pending() {
    use aws_smithy_http_client::test_util::NeverClient;
    let sts_never = NeverClient::new();
    let identity = aws_sdk_sts::Client::from_conf(
        aws_sdk_sts::Config::builder()
            .with_test_defaults_v2()
            .http_client(sts_never.clone())
            .build(),
    );
    let rds_never = NeverClient::new();
    let region = Client::from_conf(
        aws_sdk_rds::Config::builder()
            .with_test_defaults_v2()
            .http_client(rds_never.clone())
            .build(),
    );
    let _ = tokio::time::timeout(
        Duration::from_millis(100),
        test_clients(identity, vec![("us-east-1".into(), region)]),
    )
    .await;
    assert_eq!(sts_never.num_calls(), 1);
    assert_eq!(
        rds_never.num_calls(),
        1,
        "regional probes must not wait for the identity lookup"
    );
}

#[tokio::test]
async fn assume_role_request_wires_external_id_session_duration_and_role_without_network() {
    use aws_sdk_sts::config::{Credentials, ProvideCredentials};
    use aws_smithy_http_client::test_util::capture_request;
    use aws_smithy_types::body::SdkBody;
    let response = aws_sdk_sts::config::http::HttpResponse::new(200.try_into().unwrap(), SdkBody::from(
        "<AssumeRoleResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><AssumeRoleResult><Credentials><AccessKeyId>ASSUMEDKEY</AccessKeyId><SecretAccessKey>assumed-secret</SecretAccessKey><SessionToken>session</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration></Credentials></AssumeRoleResult><ResponseMetadata><RequestId>test</RequestId></ResponseMetadata></AssumeRoleResponse>"
    ));
    let (http_client, request) = capture_request(Some(response.try_into_http1x().unwrap()));
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "AMBIENTKEY",
            "ambient-secret",
            None,
            None,
            "test",
        ))
        .http_client(http_client)
        .load()
        .await;
    let provider = AwsProvider::from_config(config);
    let source = SourceConfig {
        role_arn: Some("arn:aws:iam::123456789012:role/discovery".into()),
        external_id: Some("external-id-example".into()),
        regions: vec!["eu-west-1".into()],
    };
    let configured = provider.source_config(&source).await;
    let credentials = configured
        .credentials_provider()
        .unwrap()
        .provide_credentials()
        .await
        .unwrap();
    assert_eq!(credentials.access_key_id(), "ASSUMEDKEY");
    let request = request.expect_request();
    let body = std::str::from_utf8(request.body().bytes().unwrap()).unwrap();
    assert!(body.contains("Action=AssumeRole"));
    assert!(body.contains("RoleSessionName=visp-db-access"));
    assert!(body.contains("ExternalId=external-id-example"));
    assert!(body.contains("DurationSeconds=3600"));
    assert!(body.contains("RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2Fdiscovery"));
    assert!(request.uri().contains("eu-west-1"));
    let retry = configured.retry_config().unwrap();
    assert_eq!(retry.max_attempts(), 3);
    let timeouts = configured.timeout_config().unwrap();
    assert_eq!(timeouts.operation_timeout(), Some(Duration::from_secs(20)));
    assert_eq!(
        timeouts.operation_attempt_timeout(),
        Some(Duration::from_secs(10))
    );
}

#[tokio::test]
async fn ambient_source_keeps_credentials_and_overrides_region() {
    use aws_sdk_sts::config::{Credentials, ProvideCredentials};
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "AMBIENTKEY",
            "ambient-secret",
            None,
            None,
            "test",
        ))
        .load()
        .await;
    let provider = AwsProvider::from_config(config);
    let config = provider
        .source_config(&SourceConfig {
            role_arn: None,
            external_id: None,
            regions: vec!["eu-west-1".into()],
        })
        .await;
    assert_eq!(config.region().unwrap().as_ref(), "eu-west-1");
    assert_eq!(
        config
            .credentials_provider()
            .unwrap()
            .provide_credentials()
            .await
            .unwrap()
            .access_key_id(),
        "AMBIENTKEY"
    );
}

#[tokio::test]
async fn total_deadline_retains_success_and_bounds_region_concurrency() {
    use aws_smithy_http_client::test_util::NeverClient;
    let cluster_rule = mock!(aws_sdk_rds::Client::describe_db_clusters)
        .then_output(|| DescribeDbClustersOutput::builder().build());
    let instance_rule = mock!(aws_sdk_rds::Client::describe_db_instances).then_output(|| {
        DescribeDbInstancesOutput::builder()
            .db_instances(instance("orders"))
            .build()
    });
    let good = mock_client!(
        aws_sdk_rds,
        RuleMode::MatchAny,
        [&cluster_rule, &instance_rule]
    );
    let never = NeverClient::new();
    let slow = Client::from_conf(
        aws_sdk_rds::Config::builder()
            .with_test_defaults_v2()
            .http_client(never.clone())
            .build(),
    );
    let mut clients = vec![("us-east-1".into(), good)];
    clients.extend((0..6).map(|index| (format!("slow-{index}"), slow.clone())));
    let outcome = scan_clients_before(clients, Duration::from_millis(100)).await;
    assert_eq!(outcome.resources.len(), 1);
    assert_eq!(outcome.errors.len(), 6);
    assert_eq!(
        never.num_calls(),
        4,
        "at most four region operations should be in flight"
    );
    assert!(outcome
        .errors
        .iter()
        .all(|error| error.message.contains("time budget")));
}

#[tokio::test]
async fn live_aws_discovery_is_opt_in() {
    let Ok(regions) = std::env::var("VDA_TEST_AWS_REGIONS") else {
        return;
    };
    let regions = regions
        .split(',')
        .map(str::trim)
        .filter(|region| !region.is_empty())
        .map(String::from)
        .collect();
    let source = SourceConfig {
        role_arn: None,
        external_id: None,
        regions,
    };
    let provider = AwsProvider::new().await;
    let report = provider.test(&source).await;
    assert!(report.ok, "{report:?}");
    let outcome = provider.scan(&source).await;
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
}
