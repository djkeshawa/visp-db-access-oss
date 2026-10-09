#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use vda_guard::{
    analyze, AccessLevel, Analysis, Dialect, GuardPolicy, Risk, Severity, StatementKind, Verdict,
};

fn policy() -> GuardPolicy {
    GuardPolicy {
        max_rows: 100,
        allow_writes: true,
        require_approval_for_writes: true,
        allow_ddl: true,
        blocked_tables: vec![],
    }
}
fn check(sql: &str, dialect: Dialect, level: AccessLevel) -> Analysis {
    analyze(sql, dialect, level, &policy())
}
fn issue(a: &Analysis, code: &str) -> bool {
    a.issues.iter().any(|i| i.code == code)
}

#[test]
fn parsing_and_statement_boundaries() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        for (sql, code) in [
            ("", "empty_query"),
            (" -- comment\n /* hi */ ;", "empty_query"),
            ("SELECT FROM", "parse_error"),
            ("SELECT 1; -- hidden\n DROP TABLE x", "multiple_statements"),
            (
                "SELECT 1; /* hidden */ DELETE FROM x",
                "multiple_statements",
            ),
        ] {
            let a = check(sql, dialect, AccessLevel::Admin);
            assert_eq!(a.verdict, Verdict::Deny, "{sql}: {a:?}");
            assert!(issue(&a, code), "{sql}: {a:?}");
            assert!(a.rewritten_sql.is_none());
        }
        let a = check("SELECT 1; SELECT 2", dialect, AccessLevel::Read);
        assert_eq!(a.statements.len(), 2);
        assert!(issue(
            &check(&"x".repeat(1024 * 1024 + 1), dialect, AccessLevel::Read),
            "query_too_large"
        ));
    }
}

#[test]
fn classification_and_permissions() {
    let cases = [
        ("SELECT 1", StatementKind::Select, Verdict::Allow),
        ("EXPLAIN SELECT 1", StatementKind::Explain, Verdict::Allow),
        ("SHOW search_path", StatementKind::Show, Verdict::Allow),
        (
            "INSERT INTO x VALUES (1)",
            StatementKind::Insert,
            Verdict::RequiresApproval,
        ),
        (
            "UPDATE x SET y = 1 WHERE id = 2",
            StatementKind::Update,
            Verdict::RequiresApproval,
        ),
        (
            "DELETE FROM x WHERE id = 2",
            StatementKind::Delete,
            Verdict::RequiresApproval,
        ),
        (
            "MERGE INTO x USING y ON x.id = y.id WHEN MATCHED THEN DELETE",
            StatementKind::Merge,
            Verdict::RequiresApproval,
        ),
        (
            "CREATE TABLE x (id INT)",
            StatementKind::Ddl,
            Verdict::RequiresApproval,
        ),
        (
            "DROP TABLE x",
            StatementKind::Ddl,
            Verdict::RequiresApproval,
        ),
        ("TRUNCATE x", StatementKind::Ddl, Verdict::RequiresApproval),
        ("GRANT SELECT ON x TO y", StatementKind::Dcl, Verdict::Deny),
        ("CREATE ROLE x", StatementKind::Dcl, Verdict::Deny),
        ("ALTER ROLE x LOGIN", StatementKind::Dcl, Verdict::Deny),
        ("DROP ROLE x", StatementKind::Dcl, Verdict::Deny),
        ("BEGIN", StatementKind::Transaction, Verdict::Deny),
        ("COMMIT", StatementKind::Transaction, Verdict::Deny),
        ("ROLLBACK", StatementKind::Transaction, Verdict::Deny),
        ("SAVEPOINT x", StatementKind::Transaction, Verdict::Deny),
        (
            "SET statement_timeout = 0",
            StatementKind::Utility,
            Verdict::Deny,
        ),
        ("COPY x TO STDOUT", StatementKind::Utility, Verdict::Deny),
        ("VACUUM x", StatementKind::Utility, Verdict::Deny),
        ("ANALYZE x", StatementKind::Utility, Verdict::Deny),
        ("CALL x()", StatementKind::Utility, Verdict::Deny),
        ("LISTEN x", StatementKind::Utility, Verdict::Deny),
        ("NOTIFY x", StatementKind::Utility, Verdict::Deny),
        (
            "PREPARE x AS SELECT 1",
            StatementKind::Utility,
            Verdict::Deny,
        ),
        ("EXECUTE x", StatementKind::Utility, Verdict::Deny),
        ("DEALLOCATE x", StatementKind::Utility, Verdict::Deny),
    ];
    for (sql, kind, verdict) in cases {
        let a = check(sql, Dialect::Postgres, AccessLevel::Admin);
        assert_eq!(
            a.statements.first().map(|s| s.kind),
            Some(kind),
            "{sql}: {a:?}"
        );
        assert_eq!(a.verdict, verdict, "{sql}: {a:?}");
    }
    for sql in [
        "INSERT INTO x VALUES (1)",
        "UPDATE x SET y=1 WHERE id=2",
        "DELETE FROM x WHERE id=2",
    ] {
        assert!(issue(
            &check(sql, Dialect::Postgres, AccessLevel::Read),
            "insufficient_access"
        ));
        let mut p = policy();
        p.allow_writes = false;
        assert!(issue(
            &analyze(sql, Dialect::Postgres, AccessLevel::Admin, &p),
            "writes_disabled"
        ));
        p.allow_writes = true;
        p.require_approval_for_writes = false;
        assert_eq!(
            analyze(sql, Dialect::Postgres, AccessLevel::Write, &p).verdict,
            Verdict::Allow
        );
    }
    assert!(issue(
        &check("DROP TABLE x", Dialect::Postgres, AccessLevel::Write),
        "insufficient_access"
    ));
    let mut p = policy();
    p.allow_ddl = false;
    assert!(issue(
        &analyze("DROP TABLE x", Dialect::Postgres, AccessLevel::Admin, &p),
        "ddl_disabled"
    ));
}

#[test]
fn whole_ast_writes_and_explain() {
    for (sql, kind, code) in [
        (
            "WITH gone AS (DELETE FROM x WHERE id=2 RETURNING *) SELECT * FROM gone",
            StatementKind::Delete,
            "data_modifying_cte",
        ),
        (
            "WITH changed AS (UPDATE x SET y=2 WHERE id=1 RETURNING *) SELECT * FROM changed",
            StatementKind::Update,
            "data_modifying_cte",
        ),
        (
            "WITH added AS (INSERT INTO x VALUES (1) RETURNING *) SELECT * FROM added",
            StatementKind::Insert,
            "data_modifying_cte",
        ),
        (
            "EXPLAIN ANALYZE DELETE FROM x WHERE id=2",
            StatementKind::Delete,
            "explain_analyze_write",
        ),
        (
            "EXPLAIN (ANALYZE TRUE) UPDATE x SET y=2 WHERE id=1",
            StatementKind::Update,
            "explain_analyze_write",
        ),
        (
            "SELECT * INTO new_table FROM x",
            StatementKind::Ddl,
            "select_into",
        ),
    ] {
        let a = check(sql, Dialect::Postgres, AccessLevel::Admin);
        assert_eq!(
            a.statements.first().map(|s| s.kind),
            Some(kind),
            "{sql}: {a:?}"
        );
        assert!(issue(&a, code), "{sql}: {a:?}");
        assert!(!a.is_read_only());
        assert_eq!(a.verdict, Verdict::RequiresApproval);
        assert_eq!(
            check(sql, Dialect::Postgres, AccessLevel::Read).verdict,
            Verdict::Deny
        );
    }
    let a = check(
        "EXPLAIN ANALYZE SELECT 1",
        Dialect::Postgres,
        AccessLevel::Read,
    );
    assert_eq!(a.verdict, Verdict::Allow);
    assert!(issue(&a, "explain_analyze_executes"));
    assert_eq!(
        check(
            "EXPLAIN DELETE FROM x",
            Dialect::Postgres,
            AccessLevel::Read
        )
        .verdict,
        Verdict::Allow
    );
    assert_eq!(
        check(
            "EXPLAIN (ANALYZE FALSE) DELETE FROM x",
            Dialect::Postgres,
            AccessLevel::Read
        )
        .verdict,
        Verdict::Allow
    );
    assert!(issue(
        &check(
            "WITH gone AS (DELETE FROM x RETURNING *) SELECT * FROM gone WHERE id=1",
            Dialect::Postgres,
            AccessLevel::Admin
        ),
        "write_without_where"
    ));
}

#[test]
fn write_predicates() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        for sql in ["DELETE FROM x", "UPDATE x SET y=1"] {
            assert!(issue(
                &check(sql, dialect, AccessLevel::Admin),
                "write_without_where"
            ));
        }
        for predicate in [
            "1=1",
            "true",
            "'a'='a'",
            "x=x",
            "x.id=x.id",
            "(true)",
            "id=1 OR 1=1",
            "true AND 2=2",
            "TRUE IS TRUE",
        ] {
            let sql = format!("DELETE FROM x WHERE {predicate}");
            let a = check(&sql, dialect, AccessLevel::Admin);
            assert!(issue(&a, "tautological_where"), "{sql}: {a:?}");
        }
        for predicate in ["1=0", "false", "id=1", "id=1 AND true", "random()=random()"] {
            let a = check(
                &format!("DELETE FROM x WHERE {predicate}"),
                dialect,
                AccessLevel::Admin,
            );
            assert!(!issue(&a, "tautological_where"), "{predicate}: {a:?}");
        }
        assert!(issue(
            &check("INSERT INTO x SELECT * FROM y", dialect, AccessLevel::Write),
            "unbounded_insert_select"
        ));
        assert!(!issue(
            &check(
                "INSERT INTO x SELECT * FROM y LIMIT 5",
                dialect,
                AccessLevel::Write
            ),
            "unbounded_insert_select"
        ));
    }
}

#[test]
fn functions_in_every_position() {
    for sql in [
        "SELECT pg_sleep(1)",
        "SELECT pg_catalog.pg_sleep(1)",
        "SELECT \"PG_SLEEP\"(1)",
        "SELECT (SELECT pg_sleep(1))",
        "WITH c AS (SELECT pg_sleep(1)) SELECT * FROM c",
        "SELECT 1 ORDER BY pg_sleep(1)",
        "SELECT sum(id) OVER (ORDER BY pg_sleep(1)) FROM x",
        "SELECT * FROM x CROSS JOIN LATERAL pg_sleep(1)",
        "SELECT * FROM pg_sleep(1)",
        "SELECT 1 UNION SELECT pg_sleep(1)",
        "SELECT 1 /* pg_sleep hidden? */ WHERE EXISTS (SELECT pg_sleep(1))",
    ] {
        let a = check(sql, Dialect::Postgres, AccessLevel::Admin);
        assert!(issue(&a, "dangerous_function"), "{sql}: {a:?}");
        assert_eq!(a.verdict, Verdict::Deny);
    }
    for sql in [
        "SELECT SLEEP(1)",
        "SELECT `SLEEP`(1)",
        "SELECT sys.sys_exec('ls')",
        "SELECT * FROM (SELECT load_file('x')) t",
    ] {
        assert!(
            issue(
                &check(sql, Dialect::MySql, AccessLevel::Admin),
                "dangerous_function"
            ),
            "{sql}"
        );
    }
    let a = check(
        "SELECT * FROM generate_series(1,100)",
        Dialect::Postgres,
        AccessLevel::Read,
    );
    assert_eq!(a.verdict, Verdict::Allow);
    assert!(issue(&a, "unbounded_generator"));
    assert_eq!(
        check("SELECT 'pg_sleep(1)'", Dialect::Postgres, AccessLevel::Read).verdict,
        Verdict::Allow
    );
}

#[test]
fn locking_and_file_writes() {
    for (dialect, clauses) in [
        (
            Dialect::Postgres,
            vec![
                "FOR UPDATE",
                "FOR SHARE",
                "FOR NO KEY UPDATE",
                "FOR KEY SHARE",
            ],
        ),
        (
            Dialect::MySql,
            vec!["FOR UPDATE", "FOR SHARE", "LOCK IN SHARE MODE"],
        ),
    ] {
        for clause in clauses {
            let sql = format!("SELECT * FROM x {clause}");
            let a = check(&sql, dialect, AccessLevel::Read);
            assert!(issue(&a, "locking_read"), "{sql}: {a:?}");
            assert_eq!(a.verdict, Verdict::Deny);
            assert_eq!(
                check(&sql, dialect, AccessLevel::Write).verdict,
                Verdict::RequiresApproval,
                "{sql}"
            );
        }
    }
    for sql in [
        "SELECT * FROM x INTO OUTFILE '/tmp/x'",
        "SELECT 1 INTO DUMPFILE '/tmp/x'",
        "SELECT * INTO OUTFILE '/tmp/x' FROM x",
    ] {
        let a = check(sql, Dialect::MySql, AccessLevel::Admin);
        assert_eq!(a.verdict, Verdict::Deny);
        assert!(issue(&a, "file_write"), "{sql}: {a:?}");
    }
}

#[test]
fn relations_ctes_and_patterns() {
    let a = check("WITH c AS (SELECT * FROM PUBLIC.Users) SELECT * FROM c JOIN db.app.Orders o ON true WHERE EXISTS (SELECT 1 FROM logs)", Dialect::Postgres, AccessLevel::Read);
    assert_eq!(
        a.statements[0].tables,
        vec!["app.orders", "logs", "public.users"]
    );
    for (pattern, table) in [
        ("x", "x"),
        ("x", "app.x"),
        ("app.*", "app.x"),
        ("*.x", "app.x"),
        ("*.secret*", "app.secrets"),
        ("app.sec*ts", "app.secrets"),
        ("APP.X", "app.x"),
    ] {
        let mut p = policy();
        p.blocked_tables.push(pattern.into());
        let a = analyze(
            &format!("SELECT * FROM {table}"),
            Dialect::Postgres,
            AccessLevel::Read,
            &p,
        );
        assert!(issue(&a, "blocked_table"), "{pattern} {table}: {a:?}");
    }
    for table in [
        "pg_catalog.pg_authid",
        "pg_authid",
        "pg_shadow",
        "pg_catalog.pg_user_mapping",
        "mysql.user",
        "mysql.global_priv",
    ] {
        assert!(
            issue(
                &check(
                    &format!("SELECT * FROM {table}"),
                    Dialect::Postgres,
                    AccessLevel::Read
                ),
                "sensitive_catalog"
            ),
            "{table}"
        );
    }
    for table in ["information_schema.tables", "pg_catalog.pg_class"] {
        assert_eq!(
            check(
                &format!("SELECT * FROM {table}"),
                Dialect::Postgres,
                AccessLevel::Read
            )
            .verdict,
            Verdict::Allow
        );
    }
    // A CTE in one branch must not hide a physical table in another scope.
    let mut p = policy();
    p.blocked_tables.push("secrets".into());
    let a = analyze("SELECT * FROM (WITH secrets AS (SELECT 1) SELECT * FROM secrets) c UNION SELECT * FROM secrets", Dialect::Postgres, AccessLevel::Read, &p);
    assert!(issue(&a, "blocked_table"));
    assert_eq!(
        check(
            "SELECT * FROM `app`.`Users`",
            Dialect::MySql,
            AccessLevel::Read
        )
        .statements[0]
            .tables,
        vec!["app.users"]
    );
}

#[test]
fn limit_rewrites_and_original_flags() {
    for (sql, dialect, expected, limited) in [
        (
            "SELECT * FROM x",
            Dialect::Postgres,
            "SELECT * FROM x LIMIT 101",
            false,
        ),
        (
            "SELECT * FROM x LIMIT 10",
            Dialect::Postgres,
            "SELECT * FROM x LIMIT 10",
            true,
        ),
        (
            "SELECT * FROM x LIMIT 1000 OFFSET 5",
            Dialect::Postgres,
            "SELECT * FROM x LIMIT 101 OFFSET 5",
            true,
        ),
        (
            "SELECT * FROM x OFFSET 5",
            Dialect::Postgres,
            "SELECT * FROM x LIMIT 101 OFFSET 5",
            false,
        ),
        (
            "SELECT * FROM x LIMIT ALL",
            Dialect::Postgres,
            "SELECT * FROM x LIMIT 101",
            true,
        ),
        (
            "SELECT 1 UNION SELECT 2",
            Dialect::Postgres,
            "SELECT 1 UNION SELECT 2 LIMIT 101",
            false,
        ),
        (
            "SELECT * FROM x LIMIT 5, 10",
            Dialect::MySql,
            "SELECT * FROM x LIMIT 5, 10",
            true,
        ),
        (
            "SELECT * FROM x LIMIT 5, 1000",
            Dialect::MySql,
            "SELECT * FROM x LIMIT 5, 101",
            true,
        ),
        (
            "SELECT * FROM x FETCH FIRST 1000 ROWS ONLY",
            Dialect::Postgres,
            "SELECT * FROM x FETCH FIRST 101 ROWS ONLY",
            true,
        ),
        (
            "SELECT * FROM x FETCH FIRST 10 ROWS ONLY",
            Dialect::Postgres,
            "SELECT * FROM x FETCH FIRST 10 ROWS ONLY",
            true,
        ),
    ] {
        let a = check(sql, dialect, AccessLevel::Read);
        assert_eq!(a.rewritten_sql.as_deref(), Some(expected), "{sql}: {a:?}");
        assert_eq!(a.statements[0].has_limit, limited, "{sql}");
    }
    for sql in [
        "EXPLAIN SELECT * FROM x",
        "SHOW search_path",
        "DELETE FROM x WHERE id=1",
        "DROP TABLE x",
    ] {
        assert_eq!(
            check(
                &format!("  {sql};  "),
                Dialect::Postgres,
                AccessLevel::Admin
            )
            .rewritten_sql
            .as_deref(),
            Some(sql)
        );
    }
}

#[test]
fn risks_and_json() {
    for (sql, risk) in [
        ("SELECT id FROM x LIMIT 10", Risk::Low),
        ("SELECT count(*) FROM x", Risk::Low),
        ("SELECT id FROM x", Risk::Medium),
        ("SELECT * FROM x LIMIT 10", Risk::Medium),
        ("SELECT a.id FROM a CROSS JOIN b LIMIT 10", Risk::Medium),
        ("DELETE FROM x WHERE id=1", Risk::High),
        ("DROP DATABASE x", Risk::Critical),
        ("DROP SCHEMA x CASCADE", Risk::Critical),
        ("TRUNCATE x", Risk::Critical),
    ] {
        let a = check(sql, Dialect::Postgres, AccessLevel::Admin);
        assert_eq!(a.risk, risk, "{sql}: {a:?}");
    }
    assert!(issue(
        &check(
            "SELECT * FROM a CROSS JOIN b",
            Dialect::Postgres,
            AccessLevel::Read
        ),
        "cross_join"
    ));
    let a = check(
        "SELECT * FROM x WHERE name LIKE '%x'",
        Dialect::Postgres,
        AccessLevel::Read,
    );
    assert!(a
        .issues
        .iter()
        .any(|i| i.code == "non_sargable_like" && i.severity == Severity::Info));
    let a = check(
        "UPDATE x SET y=1 WHERE id=2",
        Dialect::Postgres,
        AccessLevel::Write,
    );
    let json = serde_json::to_value(&a).unwrap();
    assert_eq!(json["verdict"], "requires_approval");
    assert_eq!(json["risk"], "high");
    assert_eq!(json["statements"][0]["kind"], "update");
    assert_eq!(serde_json::from_value::<Analysis>(json).unwrap(), a);
}

#[test]
fn random_strings_never_panic() {
    let alphabet = b" abcdefSELECTWHERE0123456789;()'\"`/*-+=$\\\n\t";
    let mut seed = 123456789_u64;
    for length in 0..512 {
        let input: String = (0..length)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                alphabet[(seed as usize) % alphabet.len()] as char
            })
            .collect();
        for dialect in [Dialect::Postgres, Dialect::MySql] {
            let _ = check(&input, dialect, AccessLevel::Admin);
        }
    }
}

#[test]
#[ignore = "run with cargo test --release -p vda-guard -- --ignored"]
fn ten_kib_query_under_twenty_milliseconds() {
    let sql = format!(
        "SELECT {}",
        (0..2000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(sql.len() > 10_000);
    let started = std::time::Instant::now();
    let a = check(&sql, Dialect::Postgres, AccessLevel::Read);
    assert_eq!(a.verdict, Verdict::Allow);
    assert!(started.elapsed() < std::time::Duration::from_millis(20));
}

#[test]
fn cte_visibility_does_not_hide_write_targets() {
    let mut p = policy();
    p.blocked_tables.push("secrets".into());
    for sql in [
        "WITH secrets AS (SELECT * FROM secrets) SELECT * FROM secrets",
        "WITH RECURSIVE secrets AS (SELECT 1 UNION ALL SELECT 2 FROM secrets), gone AS (DELETE FROM secrets WHERE id=1 RETURNING *) SELECT * FROM gone",
        "WITH secrets AS (SELECT 1) UPDATE secrets SET id=2 WHERE id=1",
        "WITH secrets AS (SELECT 1) INSERT INTO secrets VALUES (2)",
        "WITH secrets AS (SELECT 1) DELETE FROM secrets WHERE id=1",
    ] {
        let a = analyze(sql, Dialect::Postgres, AccessLevel::Admin, &p);
        assert!(issue(&a, "blocked_table"), "{sql}: {a:?}");
    }
    let a = analyze("WITH RECURSIVE secrets AS (SELECT 1 AS id UNION ALL SELECT id+1 FROM secrets WHERE id<5) SELECT * FROM secrets", Dialect::Postgres, AccessLevel::Read, &p);
    assert_eq!(a.verdict, Verdict::Allow, "{a:?}");
    let a = analyze(
        "WITH a AS (SELECT 1), secrets AS (SELECT * FROM a) SELECT * FROM secrets",
        Dialect::Postgres,
        AccessLevel::Read,
        &p,
    );
    assert_eq!(a.verdict, Verdict::Allow, "{a:?}");
}

#[test]
fn locking_rewrite_preserves_literals_and_exact_strength() {
    for (dialect, clause) in [
        (Dialect::Postgres, "FOR NO KEY UPDATE"),
        (Dialect::Postgres, "FOR KEY SHARE"),
        (Dialect::MySql, "LOCK IN SHARE MODE"),
    ] {
        let sql = format!("SELECT '雪 FOR SHARE' AS note, id FROM x {clause}");
        let a = check(&sql, dialect, AccessLevel::Write);
        assert_eq!(a.verdict, Verdict::RequiresApproval, "{a:?}");
        assert_eq!(
            a.rewritten_sql.as_deref(),
            Some(format!("SELECT '雪 FOR SHARE' AS note, id FROM x LIMIT 101 {clause}").as_str())
        );
        assert_eq!(a.statements[0].sql, sql);
    }
    let sql = "SELECT id FROM (SELECT id FROM x FOR KEY SHARE) s FOR NO KEY UPDATE";
    let a = check(sql, Dialect::Postgres, AccessLevel::Write);
    assert_eq!(
        a.rewritten_sql.as_deref(),
        Some("SELECT id FROM (SELECT id FROM x FOR KEY SHARE) s LIMIT 101 FOR NO KEY UPDATE")
    );
    let a = check(
        "SELECT id FROM x WHERE EXISTS (SELECT 1 FROM y FOR UPDATE)",
        Dialect::Postgres,
        AccessLevel::Read,
    );
    assert!(issue(&a, "locking_read"));
}

#[test]
fn limits_are_bounded_for_nonliteral_and_fetch_forms() {
    for sql in [
        "SELECT id FROM x LIMIT (1000)",
        "SELECT id FROM x LIMIT 50+500",
        "SELECT id FROM x LIMIT ALL OFFSET 5",
        "SELECT id FROM x FETCH FIRST 50 PERCENT ROWS ONLY",
        "SELECT id FROM x FETCH FIRST 10 ROWS WITH TIES",
    ] {
        let a = check(sql, Dialect::Postgres, AccessLevel::Read);
        assert_eq!(a.verdict, Verdict::Allow, "{sql}: {a:?}");
        assert!(a.statements[0].has_limit, "{sql}");
        assert!(
            a.rewritten_sql.as_ref().unwrap().contains("101"),
            "{sql}: {a:?}"
        );
        assert!(!a.rewritten_sql.as_ref().unwrap().contains("WITH TIES"));
        assert!(!a.rewritten_sql.as_ref().unwrap().contains("PERCENT"));
    }
    let mut p = policy();
    p.max_rows = u64::MAX;
    let a = analyze("SELECT 1", Dialect::Postgres, AccessLevel::Read, &p);
    assert_eq!(
        a.rewritten_sql.as_deref(),
        Some("SELECT 1 LIMIT 18446744073709551616")
    );
    p.max_rows = 0;
    assert_eq!(
        analyze("SELECT 1", Dialect::MySql, AccessLevel::Read, &p)
            .rewritten_sql
            .as_deref(),
        Some("SELECT 1 LIMIT 1")
    );
    assert_eq!(
        check(
            "SELECT id FROM x LIMIT 100",
            Dialect::Postgres,
            AccessLevel::Read
        )
        .rewritten_sql
        .as_deref(),
        Some("SELECT id FROM x LIMIT 100")
    );
}

#[test]
fn additional_dialect_classification_and_collections() {
    for (sql, kind, verdict) in [
        ("DESCRIBE x", StatementKind::Show, Verdict::Allow),
        ("EXPLAIN x", StatementKind::Explain, Verdict::Allow),
        ("SHOW TABLES", StatementKind::Show, Verdict::Allow),
        ("SHOW COLUMNS FROM x", StatementKind::Show, Verdict::Allow),
        ("SHOW CREATE TABLE x", StatementKind::Show, Verdict::Allow),
        ("SHOW VARIABLES", StatementKind::Show, Verdict::Allow),
        ("CREATE USER x", StatementKind::Dcl, Verdict::Deny),
        ("DROP USER x", StatementKind::Dcl, Verdict::Deny),
        (
            "RENAME TABLE x TO y",
            StatementKind::Ddl,
            Verdict::RequiresApproval,
        ),
        ("USE db", StatementKind::Utility, Verdict::Deny),
        ("KILL 1", StatementKind::Utility, Verdict::Deny),
        ("LOCK TABLES x READ", StatementKind::Utility, Verdict::Deny),
    ] {
        let a = check(sql, Dialect::MySql, AccessLevel::Admin);
        assert_eq!(
            a.statements.first().map(|s| s.kind),
            Some(kind),
            "{sql}: {a:?}"
        );
        assert_eq!(a.verdict, verdict, "{sql}: {a:?}");
    }
    assert_eq!(
        check("CALL pg_sleep(1)", Dialect::Postgres, AccessLevel::Admin).statements[0].functions,
        vec!["pg_sleep"]
    );
    let mut p = policy();
    p.blocked_tables.push("x".into());
    assert!(issue(
        &analyze("SHOW CREATE TABLE x", Dialect::MySql, AccessLevel::Read, &p),
        "blocked_table"
    ));
    for sql in ["SELECT uuid_short()", "SELECT UUID_SHORT()"] {
        let a = check(sql, Dialect::MySql, AccessLevel::Read);
        assert_eq!(a.verdict, Verdict::Allow);
        assert!(a.issues.iter().any(|i| i.severity == Severity::Warning));
    }
    let a = check(
        "SELECT id FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id JOIN d ON c.id=d.id LIMIT 5",
        Dialect::Postgres,
        AccessLevel::Read,
    );
    assert_eq!(a.risk, Risk::Medium);
    let a = check(
        "SELECT pg_sleep(1); DROP TABLE x",
        Dialect::Postgres,
        AccessLevel::Admin,
    );
    assert_eq!(a.statements.len(), 2);
    assert_eq!(a.risk, Risk::Critical);
    assert!(issue(&a, "multiple_statements"));
    assert!(issue(&a, "dangerous_function"));
}

#[test]
fn lexical_and_complexity_bypasses_fail_closed() {
    for sql in [
        "SELECT 1 /*!; DROP TABLE x */",
        "SELECT 1 /*M!; DROP TABLE x */",
        "DELETE FROM x WHERE id=1 --x; DROP TABLE x",
    ] {
        let a = check(sql, Dialect::MySql, AccessLevel::Admin);
        assert_eq!(a.verdict, Verdict::Deny, "{sql}: {a:?}");
        assert!(
            issue(&a, "parse_error")
                || issue(&a, "multiple_statements")
                || issue(&a, "executable_comment"),
            "{sql}: {a:?}"
        );
    }
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        for sql in [
            format!("SELECT {}1{}", "(".repeat(200), ")".repeat(200)),
            format!("SELECT {}", vec!["1"; 20_000].join("+")),
            format!("SELECT {}", vec!["coalesce(1,2)"; 20_000].join("+")),
            vec!["SELECT 1"; 500].join(" UNION ALL "),
        ] {
            let a = check(&sql, dialect, AccessLevel::Admin);
            assert_eq!(a.verdict, Verdict::Deny);
            assert!(issue(&a, "parse_error"));
        }
    }
    for predicate in ["X=x", "x.ID=x.id", "(x)=(x)"] {
        assert!(issue(
            &check(
                &format!("DELETE FROM x WHERE {predicate}"),
                Dialect::Postgres,
                AccessLevel::Admin
            ),
            "tautological_where"
        ));
    }
}

#[test]
fn relation_collection_in_all_statement_forms() {
    for (sql, dialect, expected) in [
        (
            "INSERT INTO app.x SELECT id FROM app.y LIMIT 1",
            Dialect::Postgres,
            vec!["app.x", "app.y"],
        ),
        (
            "UPDATE app.x SET id=2 FROM app.y WHERE x.id=y.id",
            Dialect::Postgres,
            vec!["app.x", "app.y"],
        ),
        (
            "DELETE FROM app.x USING app.y WHERE x.id=y.id",
            Dialect::Postgres,
            vec!["app.x", "app.y"],
        ),
        (
            "MERGE INTO app.x USING app.y ON x.id=y.id WHEN MATCHED THEN DELETE",
            Dialect::Postgres,
            vec!["app.x", "app.y"],
        ),
        ("DROP TABLE app.x", Dialect::Postgres, vec!["app.x"]),
        (
            "CREATE TABLE app.x AS SELECT * FROM app.y",
            Dialect::Postgres,
            vec!["app.x", "app.y"],
        ),
        (
            "SELECT * FROM app.y UNION TABLE app.x",
            Dialect::Postgres,
            vec!["app.x", "app.y"],
        ),
        (
            "SELECT * FROM (SELECT * FROM app.x) t",
            Dialect::MySql,
            vec!["app.x"],
        ),
        ("DESCRIBE app.x", Dialect::MySql, vec!["app.x"]),
        ("SHOW COLUMNS FROM app.x", Dialect::MySql, vec!["app.x"]),
    ] {
        let a = check(sql, dialect, AccessLevel::Admin);
        assert_eq!(
            a.statements.first().map(|s| &s.tables),
            Some(&expected.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
            "{sql}: {a:?}"
        );
    }
    let mut p = policy();
    p.blocked_tables = vec!["secret*".into(), "audit.*".into()];
    for sql in [
        "SELECT * FROM app.public_data",
        "SELECT * FROM audits.public_data",
        "WITH secrets AS (SELECT 1) SELECT * FROM secrets",
    ] {
        assert_eq!(
            analyze(sql, Dialect::Postgres, AccessLevel::Read, &p).verdict,
            Verdict::Allow,
            "{sql}"
        );
    }
    let a = analyze(
        "WITH \"Secrets\" AS (SELECT 1) SELECT * FROM secrets",
        Dialect::Postgres,
        AccessLevel::Read,
        &p,
    );
    assert!(issue(&a, "blocked_table"));
}

#[test]
fn nested_effects_and_serialized_issue_shape() {
    for (sql, dialect) in [
        ("SELECT 1 ORDER BY (SELECT sleep(1))", Dialect::MySql),
        (
            "WITH c AS (SELECT benchmark(100,1)) SELECT * FROM c",
            Dialect::MySql,
        ),
        (
            "SELECT sum(id) OVER (ORDER BY sleep(1)) FROM x",
            Dialect::MySql,
        ),
        ("SELECT /*!50000 SLEEP(1) */", Dialect::MySql),
        (
            "SELECT * FROM x WHERE id IN (SELECT setval('seq', 1))",
            Dialect::Postgres,
        ),
        (
            "SELECT 1 ORDER BY (SELECT pg_notify('channel','msg'))",
            Dialect::Postgres,
        ),
    ] {
        let a = check(sql, dialect, AccessLevel::Admin);
        assert_eq!(a.verdict, Verdict::Deny, "{sql}: {a:?}");
        let expected = if sql.contains("/*!") {
            "executable_comment"
        } else {
            "dangerous_function"
        };
        assert!(issue(&a, expected), "{sql}: {a:?}");
    }
    let a = check("SELECT pg_sleep(1)", Dialect::Postgres, AccessLevel::Read);
    let json = serde_json::to_value(&a).unwrap();
    assert_eq!(json["verdict"], "deny");
    assert!(json["rewritten_sql"].is_null());
    assert_eq!(json["issues"][0]["severity"], "block");
    assert_eq!(json["issues"][0]["code"], "dangerous_function");
    assert_eq!(json["issues"], json["statements"][0]["issues"]);
    assert!(json["statements"][0]["has_where"].is_boolean());
    assert!(json["statements"][0]["has_limit"].is_boolean());
    for text in [
        "雪",
        "\0",
        "SELECT '🙂'",
        "SELECT \"Δ\" FROM \"Ω\"",
        "/*雪*/ SELECT 1",
        "SELECT E'\\\\'",
        "SELECT $q$pg_sleep(1);$q$",
    ] {
        for dialect in [Dialect::Postgres, Dialect::MySql] {
            let _ = check(text, dialect, AccessLevel::Admin);
        }
    }
}

#[test]
fn ddl_relation_fields_missing_visitor_annotations_are_checked() {
    let mut p = policy();
    p.blocked_tables.push("secrets".into());
    for (sql, dialect) in [
        ("DROP TABLE secrets", Dialect::Postgres),
        ("DROP VIEW secrets", Dialect::Postgres),
        ("RENAME TABLE x TO secrets", Dialect::MySql),
        ("CREATE TABLE x LIKE secrets", Dialect::MySql),
        (
            "COMMENT ON TABLE secrets IS 'description'",
            Dialect::Postgres,
        ),
        (
            "COMMENT ON COLUMN secrets.id IS 'description'",
            Dialect::Postgres,
        ),
        ("ALTER TABLE x RENAME TO secrets", Dialect::Postgres),
        (
            "CREATE TABLE x (id INT REFERENCES secrets(id))",
            Dialect::Postgres,
        ),
        (
            "CREATE TABLE x (id INT, FOREIGN KEY (id) REFERENCES secrets(id))",
            Dialect::Postgres,
        ),
        (
            "CREATE TABLE x (id INT) INHERITS (secrets)",
            Dialect::Postgres,
        ),
        (
            "ALTER TABLE x ADD FOREIGN KEY (id) REFERENCES secrets(id)",
            Dialect::Postgres,
        ),
    ] {
        let a = analyze(sql, dialect, AccessLevel::Admin, &p);
        assert!(issue(&a, "blocked_table"), "{sql}: {a:?}");
        assert_eq!(a.verdict, Verdict::Deny);
    }
}

#[test]
fn constant_predicate_folding_cannot_allow_unrestricted_writes() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        for predicate in [
            "2 > 1",
            "1 <> 2",
            "+1 = 1",
            "1 + 1 = 2",
            "01 = 1",
            "NOT false",
            "true IS NOT FALSE",
            "NULL IS NULL",
            "'x' IS NOT NULL",
        ] {
            let a = check(
                &format!("DELETE FROM x WHERE {predicate}"),
                dialect,
                AccessLevel::Admin,
            );
            assert!(issue(&a, "tautological_where"), "{predicate}: {a:?}");
        }
        for predicate in [
            "2 < 1",
            "1 = 2",
            "NULL = NULL",
            "id IS NOT NULL",
            "id > 1",
            "2 > 1 AND id=2",
        ] {
            let a = check(
                &format!("DELETE FROM x WHERE {predicate}"),
                dialect,
                AccessLevel::Admin,
            );
            assert!(!issue(&a, "tautological_where"), "{predicate}: {a:?}");
        }
    }
    assert!(issue(
        &check("DELETE FROM x WHERE 1", Dialect::MySql, AccessLevel::Admin),
        "tautological_where"
    ));
}

#[test]
fn mysql_executable_comments_and_session_mutations_are_blocked() {
    for sql in [
        "SELECT 1 /*! INTO @x */",
        "SELECT /*!50000 SQL_NO_CACHE */ 1",
        "/*!50000 DELETE FROM t */",
        "SELECT 1 /*M! SET @x=1 */",
    ] {
        let a = check(sql, Dialect::MySql, AccessLevel::Admin);
        assert_eq!(a.verdict, Verdict::Deny, "{sql}: {a:?}");
        assert!(issue(&a, "executable_comment"), "{sql}: {a:?}");
    }
    for sql in [
        "SELECT /*+ SET_VAR(max_execution_time=0) */ 1",
        "SELECT /*+ MAX_EXECUTION_TIME(0) */ 1",
        "SELECT 1 INTO @var",
        "SELECT @a := 1",
        "SELECT IF(1, @a := 2, 0)",
        "DO 1",
        "HANDLER t OPEN",
        "LOAD DATA LOCAL INFILE '/tmp/x' INTO TABLE t",
        "LOAD DATA INFILE '/tmp/x' INTO TABLE t",
        "LOAD XML INFILE '/tmp/x' INTO TABLE t",
        "LOCK TABLES t WRITE",
        "FLUSH TABLES",
        "RESET MASTER",
        "PURGE BINARY LOGS BEFORE NOW()",
        "INSTALL PLUGIN p SONAME 'x.so'",
        "CREATE FUNCTION f RETURNS INTEGER SONAME 'x.so'",
        "SELECT GET_LOCK('x',0)",
        "SELECT `GET_LOCK`('x',0)",
        "SELECT RELEASE_ALL_LOCKS()",
        "SELECT sys.ps_thread_id(1)",
        "CALL sys.ps_setup_disable_thread(1)",
        "CALL p()",
        "UPDATE performance_schema.setup_consumers SET ENABLED='YES' WHERE NAME='x'",
    ] {
        let a = check(sql, Dialect::MySql, AccessLevel::Admin);
        assert_eq!(a.verdict, Verdict::Deny, "{sql}: {a:?}");
        assert!(a.rewritten_sql.is_none());
    }
    for clause in ["FOR UPDATE SKIP LOCKED", "FOR UPDATE NOWAIT"] {
        let sql = format!("SELECT * FROM t WHERE id=1 {clause}");
        assert_eq!(
            check(&sql, Dialect::MySql, AccessLevel::Read).verdict,
            Verdict::Deny
        );
        assert_ne!(
            check(&sql, Dialect::MySql, AccessLevel::Admin).verdict,
            Verdict::Allow
        );
    }
}

#[test]
fn mysql_comments_and_strings_cannot_hide_statements() {
    for sql in [
        "SELECT 1; # hidden\n DROP TABLE t",
        "SELECT 1--x; DROP TABLE t",
        r"SELECT '\\'; DROP TABLE t; -- '",
        r"SELECT '\'; DROP TABLE t; -- '",
    ] {
        assert_eq!(
            check(sql, Dialect::MySql, AccessLevel::Admin).verdict,
            Verdict::Deny,
            "{sql}"
        );
    }
    for sql in [
        "SELECT 1 # comment\n",
        "SELECT 1 -- comment\n",
        "SELECT '/*!50000 not a comment */'",
        r"SELECT 'it\'s safe'",
        r"SELECT '\\' AS slash",
    ] {
        assert_eq!(
            check(sql, Dialect::MySql, AccessLevel::Read).verdict,
            Verdict::Allow,
            "{sql}"
        );
    }
}

#[test]
fn blocked_table_wildcard_matches_names_containing_a_literal_star() {
    for (pattern, quoted_table, dialect) in [
        ("a*c", "\"a*bc\"", Dialect::Postgres),
        ("a*s", "\"a*xs\"", Dialect::Postgres),
        ("app.sec*ts", "app.\"sec*rets\"", Dialect::Postgres),
        ("a*c", "`a*bc`", Dialect::MySql),
    ] {
        let mut p = policy();
        p.blocked_tables.push(pattern.into());
        let a = analyze(
            &format!("SELECT * FROM {quoted_table}"),
            dialect,
            AccessLevel::Read,
            &p,
        );
        assert!(
            issue(&a, "blocked_table"),
            "{pattern} {quoted_table}: {a:?}"
        );
    }
}

#[test]
fn postfix_operator_chains_fail_closed() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        for suffix in [
            " IS NULL",
            " IS NOT NULL",
            " IS TRUE",
            " ISNULL",
            " NOTNULL",
            " COLLATE x",
            " AT TIME ZONE 1",
        ] {
            let sql = format!("SELECT 1{}", suffix.repeat(5_000));
            let a = check(&sql, dialect, AccessLevel::Admin);
            assert_eq!(a.verdict, Verdict::Deny, "{suffix}: {:?}", a.issues.len());
            assert!(issue(&a, "parse_error"), "{suffix}");
        }
    }
}

fn hint<'a>(a: &'a Analysis, code: &str) -> Option<&'a vda_guard::Suggestion> {
    a.suggestions.iter().find(|s| s.code == code)
}

#[test]
fn performance_suggestions() {
    let wide = GuardPolicy {
        max_rows: 1000,
        ..policy()
    };
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        let a = analyze("SELECT * FROM users", dialect, AccessLevel::Read, &wide);
        assert_eq!(a.verdict, Verdict::Allow);
        assert!(hint(&a, "select_star").is_some());
        let fix = hint(&a, "add_limit").and_then(|s| s.fix.clone()).unwrap();
        assert_eq!(fix.action, vda_guard::FixAction::Replace);
        assert_eq!(fix.sql, "SELECT * FROM users LIMIT 100");
        // The fix must itself be allowed and carry no repeat suggestion.
        let fixed = analyze(&fix.sql, dialect, AccessLevel::Read, &wide);
        assert_eq!(fixed.verdict, Verdict::Allow);
        assert!(hint(&fixed, "add_limit").is_none());

        for (sql, code) in [
            (
                "SELECT id FROM users WHERE email LIKE '%@x.io'",
                "leading_wildcard",
            ),
            (
                "SELECT id FROM users WHERE lower(email) = 'a@x.io'",
                "function_on_column",
            ),
            (
                "SELECT id FROM users WHERE CAST(created_at AS DATE) = '2026-01-01'",
                "function_on_column",
            ),
            (
                "SELECT id FROM users WHERE id NOT IN (SELECT user_id FROM bans)",
                "not_in_subquery",
            ),
            (
                "SELECT id FROM users ORDER BY id LIMIT 10 OFFSET 50000",
                "large_offset",
            ),
        ] {
            let a = analyze(sql, dialect, AccessLevel::Read, &wide);
            assert!(hint(&a, code).is_some(), "{sql} should suggest {code}");
        }
        let random = if dialect == Dialect::MySql {
            "rand()"
        } else {
            "random()"
        };
        let a = analyze(
            &format!("SELECT id FROM users ORDER BY {random} LIMIT 5"),
            dialect,
            AccessLevel::Read,
            &wide,
        );
        assert!(hint(&a, "order_by_random").is_some());

        // Sargable, bounded, or aggregate reads stay quiet.
        for sql in [
            "SELECT id, email FROM users WHERE created_at > now() LIMIT 10",
            "SELECT count(*) FROM users",
            "SELECT id FROM users WHERE email LIKE 'a%' LIMIT 5",
        ] {
            let a = analyze(sql, dialect, AccessLevel::Read, &wide);
            assert!(a.suggestions.is_empty(), "{sql}: {:?}", a.suggestions);
        }
    }
}

#[test]
fn write_preview_and_denied_queries() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        let a = check(
            "UPDATE users SET active = false WHERE last_login < '2020-01-01'",
            dialect,
            AccessLevel::Write,
        );
        let fix = hint(&a, "preview_write")
            .and_then(|s| s.fix.clone())
            .unwrap();
        assert_eq!(fix.action, vda_guard::FixAction::NewTab);
        assert_eq!(
            fix.sql,
            "SELECT COUNT(*) FROM users WHERE last_login < '2020-01-01'"
        );
        assert_eq!(
            check(&fix.sql, dialect, AccessLevel::Read).verdict,
            Verdict::Allow
        );

        let a = check(
            "DELETE FROM sessions WHERE expires_at < '2026-01-01'",
            dialect,
            AccessLevel::Write,
        );
        assert_eq!(
            hint(&a, "preview_write")
                .and_then(|s| s.fix.as_ref())
                .map(|f| f.sql.as_str()),
            Some("SELECT COUNT(*) FROM sessions WHERE expires_at < '2026-01-01'")
        );
        // Denied SQL gets no performance advice: the block must be fixed first.
        assert!(check("DELETE FROM sessions", dialect, AccessLevel::Write)
            .suggestions
            .is_empty());
    }
    let a = check(
        r"SELECT * FROM t WHERE note LIKE '%a\\b'",
        Dialect::MySql,
        AccessLevel::Read,
    );
    assert!(hint(&a, "leading_wildcard").is_some());
}
