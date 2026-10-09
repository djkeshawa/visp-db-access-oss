#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use chrono::Utc;
use std::net::IpAddr;
use vda_guard::AccessLevel;
use vda_server::{
    crypto::Crypto,
    db::cursor::Cursor,
    masking::matches_column,
    network::{client_ip, target_ip_allowed},
    policy::Policy,
    rbac::resolve_level,
};

#[test]
fn secrets_are_bound_to_cluster_and_detect_tampering() {
    let cipher = Crypto::new(&[7; 32]);
    let id = uuid::Uuid::new_v4();
    let encrypted = cipher.encrypt(id, "secret").unwrap();
    assert_eq!(cipher.decrypt(id, &encrypted).unwrap(), "secret");
    assert!(cipher.decrypt(uuid::Uuid::new_v4(), &encrypted).is_err());
    assert!(cipher.decrypt(id, &(encrypted + "x")).is_err());
}

#[test]
fn forwarded_headers_cannot_spoof_an_untrusted_peer() {
    let peer: IpAddr = "203.0.113.10".parse().unwrap();
    let trusted = vec!["10.0.0.0/8".parse().unwrap()];
    assert_eq!(
        client_ip(peer, Some("127.0.0.1"), true, &trusted).unwrap(),
        peer
    );
    assert_eq!(
        client_ip(
            "10.1.1.1".parse().unwrap(),
            Some("127.0.0.1, 203.0.113.10, 10.2.1.1"),
            true,
            &trusted
        )
        .unwrap(),
        peer
    );
    assert!(client_ip("10.1.1.1".parse().unwrap(), Some("bad"), true, &trusted).is_err());
}

#[test]
fn dangerous_target_addresses_are_refused() {
    for value in [
        "169.254.169.254",
        "0.1.2.3",
        "100.100.100.200",
        "::169.254.169.254",
        "::ffff:169.254.169.254",
        "64:ff9b::a9fe:a9fe",
        "fd00:ec2::254",
        "::1",
        "127.0.0.1",
        "::ffff:127.0.0.1",
    ] {
        assert!(!target_ip_allowed(value.parse().unwrap(), false));
    }
    assert!(target_ip_allowed("10.1.2.3".parse().unwrap(), false));
    assert!(target_ip_allowed("127.0.0.1".parse().unwrap(), true));
    assert!(!target_ip_allowed("169.254.169.254".parse().unwrap(), true));
}

#[test]
fn masks_respect_table_context_and_wildcards() {
    assert!(matches_column(
        "*.password*",
        "password_hash",
        &["users".into()]
    ));
    assert!(matches_column("users.ssn", "ssn", &["public.users".into()]));
    assert!(!matches_column("users.ssn", "ssn", &["customers".into()]));
    assert!(matches_column("email", "EMAIL", &[]));
}

#[test]
fn policy_limits_and_conversions() {
    let mut policy = Policy::for_environment("production");
    assert!(!policy.allow_writes);
    assert_eq!(policy.max_rows, 1000);
    assert!(policy.validate().is_ok());
    assert_eq!(policy.guard().max_rows, 1000);
    assert_eq!(policy.limits().max_bytes, 32 * 1024 * 1024);
    policy.max_rows = 100001;
    assert!(policy.validate().is_err());
    assert_eq!(Policy::for_environment("development").max_rows, 5000);
}

#[test]
fn rbac_uses_maximum_unexpired_grant() {
    let now = Utc::now();
    assert_eq!(
        resolve_level(
            false,
            [
                (AccessLevel::Admin, Some(now - chrono::Duration::seconds(1))),
                (AccessLevel::Read, None)
            ],
            now
        ),
        Some(AccessLevel::Read)
    );
    assert_eq!(resolve_level(true, [], now), Some(AccessLevel::Admin));
    assert_eq!(resolve_level(false, [], now), None);
}

#[test]
fn cursor_round_trip_and_invalid_input() {
    let cursor = Cursor {
        created_at: Utc::now(),
        id: uuid::Uuid::new_v4(),
    };
    assert_eq!(Cursor::decode(&cursor.encode()).unwrap(), cursor);
    assert!(Cursor::decode("not a cursor").is_err());
}

#[test]
fn encryption_uses_fresh_nonces_and_rejects_bad_keys() {
    let crypto = Crypto::new(&[1; 32]);
    let id = uuid::Uuid::new_v4();
    assert_ne!(
        crypto.encrypt(id, "same").unwrap(),
        crypto.encrypt(id, "same").unwrap()
    );
    assert!(Crypto::from_base64("short").is_err());
    assert!(crypto.decrypt(id, "v2:not-supported").is_err());
}

#[test]
fn masking_changes_all_cells_and_preserves_unmatched_columns() {
    let mut result = vda_connectors::QueryResult {
        columns: vec![
            vda_connectors::Column {
                name: "email".into(),
                type_name: "TEXT".into(),
            },
            vda_connectors::Column {
                name: "id".into(),
                type_name: "INT".into(),
            },
        ],
        rows: vec![
            vec![
                serde_json::json!("secret@example.com"),
                serde_json::json!(1),
            ],
            vec![serde_json::Value::Null, serde_json::json!(2)],
        ],
        row_count: 2,
        truncated: false,
        affected_rows: None,
        elapsed_ms: 1,
    };
    let columns = vda_server::masking::apply(&mut result, &["email".into()], &[]);
    assert!(columns[0].masked);
    assert!(!columns[1].masked);
    assert_eq!(result.rows[0][0], "••••••");
    assert_eq!(result.rows[1][0], "••••••");
    assert_eq!(result.rows[0][1], 1);
}

#[test]
fn policy_rejects_unbounded_resources_and_invalid_cidrs() {
    let original = Policy::for_environment("production");
    for timeout in [0, 99, 600001] {
        let mut p = original.clone();
        p.statement_timeout_ms = timeout;
        assert!(p.validate().is_err());
    }
    for concurrency in [0, 65] {
        let mut p = original.clone();
        p.max_concurrent_queries = concurrency;
        assert!(p.validate().is_err());
    }
    for cost in [0., -1., f64::INFINITY, f64::NAN] {
        let mut p = original.clone();
        p.max_cost = Some(cost);
        assert!(p.validate().is_err());
    }
    let mut p = original.clone();
    p.lock_timeout_ms = p.statement_timeout_ms + 1;
    assert!(p.validate().is_err());
    p = original;
    p.allowed_cidrs = vec!["not-a-cidr".into()];
    assert!(p.validate().is_err());
}

#[test]
fn allowlists_and_proxy_hops_fail_closed() {
    use vda_server::network::{allowed, parse_cidrs};
    let ip = "10.1.2.3".parse().unwrap();
    assert!(allowed(ip, &[]));
    let cidrs = parse_cidrs(&["10.0.0.0/8".into()]).unwrap();
    assert!(allowed(ip, &cidrs));
    assert!(!allowed("203.0.113.1".parse().unwrap(), &cidrs));
    assert!(parse_cidrs(&["10.0.0.0/99".into()]).is_err());
    assert_eq!(client_ip(ip, Some("127.0.0.1"), false, &cidrs).unwrap(), ip);
    assert_eq!(
        client_ip(ip, Some("203.0.113.1, 10.1.1.1"), true, &cidrs).unwrap(),
        "203.0.113.1".parse::<IpAddr>().unwrap()
    );
}

#[test]
fn rbac_resolves_maximum_level_and_expiry_boundary() {
    let now = Utc::now();
    assert_eq!(
        resolve_level(
            false,
            [
                (AccessLevel::Read, None),
                (AccessLevel::Write, None),
                (AccessLevel::Admin, Some(now))
            ],
            now
        ),
        Some(AccessLevel::Write)
    );
    assert_eq!(
        resolve_level(
            false,
            [(AccessLevel::Admin, Some(now + chrono::Duration::seconds(1)))],
            now
        ),
        Some(AccessLevel::Admin)
    );
}

#[test]
fn keyset_page_preserves_ties_and_has_no_cursor_at_end() {
    let date = Utc::now();
    let ids = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()];
    let rows = ids
        .iter()
        .map(|id| serde_json::json!({"id":id,"created_at":date}))
        .collect::<Vec<_>>();
    let page = vda_server::db::page(rows.clone(), 1).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    let decoded = Cursor::decode(page["next_cursor"].as_str().unwrap()).unwrap();
    assert_eq!(decoded.id, ids[0]);
    assert!(vda_server::db::page(rows, 2).unwrap()["next_cursor"].is_null());
}

#[test]
fn multi_line_forwarded_headers_use_the_rightmost_untrusted_hop() {
    use axum::http::{HeaderMap, HeaderValue};
    use vda_server::network::forwarded_chain;
    let mut headers = HeaderMap::new();
    headers.append("x-forwarded-for", HeaderValue::from_static("127.0.0.1"));
    headers.append(
        "x-forwarded-for",
        HeaderValue::from_static("203.0.113.20, 10.2.3.4"),
    );
    let joined = forwarded_chain(&headers).unwrap();
    let ip = client_ip(
        "10.1.1.1".parse().unwrap(),
        joined.as_deref(),
        true,
        &["10.0.0.0/8".parse().unwrap()],
    )
    .unwrap();
    assert_eq!(ip, "203.0.113.20".parse::<IpAddr>().unwrap());
    assert_eq!(client_ip(ip, Some("bad"), false, &[]).unwrap(), ip);
    assert_eq!(
        client_ip(
            "10.1.1.1".parse().unwrap(),
            Some("127.0.0.1, 203.0.113.20"),
            true,
            &[]
        )
        .unwrap(),
        "10.1.1.1".parse::<IpAddr>().unwrap(),
        "an empty trusted proxy list must not honour X-Forwarded-For"
    );
}

#[test]
fn masked_serialization_is_denied_but_wildcard_columns_survive() {
    use vda_guard::{analyze, Dialect, Verdict};
    for sql in [
        "SELECT row_to_json(u) FROM users u",
        "SELECT to_json(u) FROM users u",
        "SELECT to_jsonb(u) FROM users u",
        "SELECT json_agg(u) FROM users u",
        "SELECT jsonb_agg(u) FROM users u",
        "SELECT json_build_object('x',u.email) FROM users u",
        "SELECT jsonb_build_object('x',u.email) FROM users u",
        "SELECT ROW(email) FROM users",
        "SELECT u FROM users u",
        "WITH x AS (SELECT u FROM users u) SELECT * FROM x",
        "SELECT (email,id) FROM users",
        "SELECT public.users FROM public.users",
    ] {
        let mut a = analyze(
            sql,
            Dialect::Postgres,
            AccessLevel::Read,
            &Policy::for_environment("development").guard(),
        );
        vda_server::masking::harden_analysis(
            &mut a,
            &["users.email".into()],
            sql,
            Dialect::Postgres,
        );
        assert_eq!(a.verdict, Verdict::Deny, "{sql}");
        assert!(
            a.issues
                .iter()
                .any(|i| i.code == "masked_data_serialization"),
            "{sql}"
        );
    }
    for sql in [
        "SELECT * FROM users",
        "SELECT u.* FROM users u",
        "SELECT email FROM users",
        "SELECT row_to_json(c) FROM customers c",
        // A projection alias sharing a table name is a column, not a whole row.
        "SELECT u.id, count(o.id) AS orders FROM users u JOIN orders o ON o.user_id = u.id \
         GROUP BY 1 ORDER BY orders DESC",
        // Row constructors outside the projection return no data.
        "SELECT id FROM users WHERE (id, email) IN (SELECT 1, 'a')",
        "SELECT id FROM users u ORDER BY u.id",
    ] {
        let mut a = analyze(
            sql,
            Dialect::Postgres,
            AccessLevel::Read,
            &Policy::for_environment("development").guard(),
        );
        vda_server::masking::harden_analysis(
            &mut a,
            &["users.email".into()],
            sql,
            Dialect::Postgres,
        );
        assert_ne!(a.verdict, Verdict::Deny, "{sql}: {:?}", a.issues);
    }
}

#[test]
fn masked_columns_cannot_be_renamed_or_derived_to_escape_masking() {
    use vda_guard::{analyze, Dialect, Verdict};
    let run = |sql: &str| {
        let mut a = analyze(
            sql,
            Dialect::Postgres,
            AccessLevel::Read,
            &Policy::for_environment("development").guard(),
        );
        vda_server::masking::harden_analysis(
            &mut a,
            &["users.email".into()],
            sql,
            Dialect::Postgres,
        );
        a
    };
    for sql in [
        "SELECT email AS contact FROM users",
        "SELECT u.email AS contact FROM users u",
        "SELECT lower(email) FROM users",
        "SELECT email || 'x' FROM users",
        "SELECT (SELECT email FROM users LIMIT 1) AS x",
        "SELECT x FROM (SELECT email AS x FROM users) t",
        "WITH t AS (SELECT email AS x FROM users) SELECT x FROM t",
        "WITH t(x) AS (SELECT email FROM users) SELECT x FROM t",
        "SELECT x FROM users AS u(id, x)",
        "SELECT 'a' AS label UNION ALL SELECT email FROM users",
    ] {
        let a = run(sql);
        assert_eq!(a.verdict, Verdict::Deny, "{sql}");
        assert!(
            a.issues
                .iter()
                .any(|i| i.code == "masked_data_transformation"),
            "{sql}"
        );
    }
    for sql in [
        "SELECT email FROM users",
        "SELECT u.email FROM users u",
        "SELECT email AS email FROM users",
        "SELECT lower(email) AS email FROM users",
        "SELECT email FROM users UNION ALL SELECT email FROM users",
        "SELECT id, lower(name) AS n FROM users WHERE lower(email) = 'a' ORDER BY id",
        "SELECT count(*) FROM users",
        // PostgreSQL keeps the column name through casts, so masking by name still applies.
        "SELECT email::integer FROM users",
        "SELECT CAST(email AS text) FROM users",
    ] {
        let a = run(sql);
        assert_ne!(a.verdict, Verdict::Deny, "{sql}: {:?}", a.issues);
    }
}

#[test]
fn masked_columns_cannot_escape_through_dml_returning() {
    use vda_guard::{analyze, Dialect, Verdict};
    let run = |sql: &str| {
        let mut a = analyze(
            sql,
            Dialect::Postgres,
            AccessLevel::Write,
            &Policy::for_environment("development").guard(),
        );
        vda_server::masking::harden_analysis(
            &mut a,
            &["users.email".into()],
            sql,
            Dialect::Postgres,
        );
        a
    };
    let masked_issue =
        |a: &vda_guard::Analysis, code: &str| a.issues.iter().any(|i| i.code == code);
    for (sql, code) in [
        (
            "UPDATE users SET name = 'x' WHERE id = 1 RETURNING email AS contact",
            "masked_data_transformation",
        ),
        (
            "INSERT INTO users (name) VALUES ('x') RETURNING lower(email)",
            "masked_data_transformation",
        ),
        (
            "DELETE FROM users WHERE id = 1 RETURNING email AS c",
            "masked_data_transformation",
        ),
        (
            "DELETE FROM users WHERE id = 1 RETURNING id, email || '' AS e",
            "masked_data_transformation",
        ),
        (
            "WITH d AS (DELETE FROM users WHERE id = 1 RETURNING email AS e) SELECT e FROM d",
            "masked_data_transformation",
        ),
        (
            "DELETE FROM users WHERE id = 1 RETURNING users",
            "masked_data_serialization",
        ),
        (
            "UPDATE users AS u SET name = 'x' WHERE id = 1 RETURNING u",
            "masked_data_serialization",
        ),
        (
            "INSERT INTO users (name) VALUES ('x') RETURNING users",
            "masked_data_serialization",
        ),
        (
            "DELETE FROM users WHERE id = 1 RETURNING (id, email)",
            "masked_data_serialization",
        ),
    ] {
        let a = run(sql);
        assert_eq!(a.verdict, Verdict::Deny, "{sql}: {:?}", a.issues);
        assert!(
            masked_issue(&a, code),
            "{sql} should report {code}: {:?}",
            a.issues
        );
    }
    for sql in [
        "UPDATE users SET name = 'x' WHERE id = 1 RETURNING id, email",
        "UPDATE users SET name = 'x' WHERE id = 1 RETURNING lower(email) AS email",
        "DELETE FROM users WHERE id = 1 RETURNING *",
        "INSERT INTO users (name) VALUES ('x') RETURNING id",
    ] {
        let a = run(sql);
        assert!(
            !masked_issue(&a, "masked_data_transformation")
                && !masked_issue(&a, "masked_data_serialization"),
            "{sql}: {:?}",
            a.issues
        );
    }
}

#[test]
fn empty_trusted_proxy_list_never_trusts_forwarded_for() {
    let peer: IpAddr = "203.0.113.10".parse().unwrap();
    assert_eq!(
        client_ip(peer, Some("198.51.100.7"), true, &[]).unwrap(),
        peer
    );
}
