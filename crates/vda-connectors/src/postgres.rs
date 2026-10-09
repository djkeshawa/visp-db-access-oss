//! PostgreSQL guarded transactions and diagnostics.
use crate::{
    cancel::{self, GuardedConnection},
    decode,
    limits::{self, Results},
    tls, Column, ConnectionSpec, ConnectorError, ExecLimits, HealthReport, PoolOptions,
    QueryResult,
};
use futures::TryStreamExt;
use sqlx::Either;
use sqlx::{postgres::PgPoolOptions, Connection, Executor, PgConnection, PgPool, Statement};
use std::{
    sync::atomic::{AtomicI32, Ordering},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn pool(spec: &ConnectionSpec, options: PoolOptions) -> PgPool {
    let application = spec.application_name.clone();
    PgPoolOptions::new()
        .max_connections(options.max_connections)
        .min_connections(0)
        // SQLx's acquire deadline includes establishing the connection and after_connect.
        .acquire_timeout(options.acquire_timeout.min(options.connect_timeout))
        .idle_timeout(options.idle_timeout)
        .test_before_acquire(true)
        .after_connect(move |connection, _| {
            let application = application.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('application_name', $1, false)")
                    .bind(application)
                    .persistent(false)
                    .execute(&mut *connection)
                    .await?;
                for sql in [
                    "SET idle_in_transaction_session_timeout = '60s'",
                    "SET default_transaction_read_only = on",
                    "SET TIME ZONE 'UTC'",
                ] {
                    sqlx::query(sql)
                        .persistent(false)
                        .execute(&mut *connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_lazy_with(tls::postgres(spec))
}

async fn setup(
    connection: &mut PgConnection,
    limits: &ExecLimits,
    write: bool,
) -> Result<(), ConnectorError> {
    command(
        connection,
        if write {
            "BEGIN TRANSACTION READ WRITE ISOLATION LEVEL READ COMMITTED"
        } else {
            "BEGIN TRANSACTION READ ONLY ISOLATION LEVEL READ COMMITTED"
        },
    )
    .await?;
    let settings = [
        (
            "statement_timeout",
            limits::millis(limits.statement_timeout),
        ),
        ("lock_timeout", limits::millis(limits.lock_timeout)),
        (
            "idle_in_transaction_session_timeout",
            limits::millis(limits.statement_timeout)
                .saturating_mul(2)
                .min(i32::MAX as u64),
        ),
    ];
    for (name, value) in settings {
        sqlx::query("SELECT set_config($1, $2, true)")
            .bind(name)
            .bind(format!("{value}ms"))
            .persistent(false)
            .execute(&mut *connection)
            .await
            .map_err(cancel::error)?;
    }
    Ok(())
}
async fn command(connection: &mut PgConnection, sql: &str) -> Result<(), ConnectorError> {
    sqlx::query(sql)
        .persistent(false)
        .execute(connection)
        .await
        .map_err(cancel::error)?;
    Ok(())
}

pub(crate) async fn execute(
    pool: &PgPool,
    sql: &str,
    limits: &ExecLimits,
    write_cap: Option<u64>,
    token: CancellationToken,
) -> Result<QueryResult, ConnectorError> {
    let start = Instant::now();
    let timeout = limits::hard_timeout(limits);
    let acquired = cancel::race(
        async { pool.acquire().await.map_err(cancel::error) },
        &token,
        timeout,
    )
    .await?;
    let mut connection = GuardedConnection::new(acquired);
    let pid = AtomicI32::new(0);
    let operation = async {
        let id: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .persistent(false)
            .fetch_one(&mut *connection)
            .await
            .map_err(cancel::error)?;
        pid.store(id, Ordering::Relaxed);
        setup(&mut connection, limits, write_cap.is_some()).await?;
        let result = if let Some(cap) = write_cap {
            write(&mut connection, sql, limits, cap).await
        } else {
            read(&mut connection, sql, limits).await
        };
        match result {
            Ok(result) if result.truncated && write_cap.is_none() => Ok(result),
            Ok(result) => {
                command(
                    &mut connection,
                    if write_cap.is_some() {
                        "COMMIT"
                    } else {
                        "ROLLBACK"
                    },
                )
                .await?;
                clear_statements(&mut connection).await?;
                connection.clean();
                Ok(result)
            }
            Err(error) => {
                if command(&mut connection, "ROLLBACK").await.is_ok()
                    && clear_statements(&mut connection).await.is_ok()
                {
                    connection.clean();
                }
                Err(error)
            }
        }
    };
    let remaining = timeout.saturating_sub(start.elapsed());
    let result = cancel::race(operation, &token, remaining).await;
    if matches!(
        result,
        Err(ConnectorError::Cancelled | ConnectorError::Timeout)
    ) || matches!(&result, Ok(result) if result.truncated && write_cap.is_none())
    {
        connection.discard();
        let id = pid.load(Ordering::Relaxed);
        if id != 0 {
            cancel::postgres(pool, id).await;
        }
        // Drop closes unfinished transactions; pool capacity stays reserved until close.
    }
    result.map(|mut result| {
        result.elapsed_ms = limits::elapsed(start);
        result
    })
}
async fn clear_statements(connection: &mut PgConnection) -> Result<(), ConnectorError> {
    connection
        .clear_cached_statements()
        .await
        .map_err(cancel::error)?;
    // SQLx metadata preparation uses named statements even with a zero-capacity cache.
    // Remove those untracked server-side statements before reusing the connection.
    connection
        .execute("DEALLOCATE ALL")
        .await
        .map_err(cancel::error)?;
    Ok(())
}

async fn describe(connection: &mut PgConnection, sql: &str) -> Result<Vec<Column>, ConnectorError> {
    Ok(decode::columns(
        connection
            .prepare(sql)
            .await
            .map_err(cancel::error)?
            .columns(),
    ))
}
async fn read(
    connection: &mut PgConnection,
    sql: &str,
    limits: &ExecLimits,
) -> Result<QueryResult, ConnectorError> {
    let mut results = Results::new(limits);
    results.columns(describe(connection, sql).await?);
    let mut stream = sqlx::query(sql).persistent(false).fetch(&mut *connection);
    while let Some(row) = stream.try_next().await.map_err(cancel::error)? {
        if results.needs_columns() {
            results.columns(decode::pg_columns(&row));
        }
        if !results.offer(|| decode::postgres(&row)) {
            break;
        }
    }
    Ok(results.finish(None))
}
#[allow(deprecated)]
async fn write(
    connection: &mut PgConnection,
    sql: &str,
    limits: &ExecLimits,
    cap: u64,
) -> Result<QueryResult, ConnectorError> {
    let mut results = Results::new(limits);
    results.columns(describe(connection, sql).await?);
    let mut affected = 0u64;
    // CommandComplete carries the true affected count even with RETURNING.
    let mut stream = sqlx::query(sql)
        .persistent(false)
        .fetch_many(&mut *connection);
    while let Some(item) = stream.try_next().await.map_err(cancel::error)? {
        match item {
            Either::Left(result) => {
                affected = affected.saturating_add(result.rows_affected());
                if affected > cap {
                    return Err(ConnectorError::TooManyAffectedRows {
                        actual: affected,
                        limit: cap,
                    });
                }
            }
            Either::Right(row) => {
                if results.needs_columns() {
                    results.columns(decode::pg_columns(&row));
                }
                results.offer(|| decode::postgres(&row));
            }
        }
    }
    Ok(results.finish(Some(affected)))
}

pub(crate) fn cost(plan: &serde_json::Value) -> Option<f64> {
    plan.get(0)?
        .get("Plan")?
        .get("Total Cost")?
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
}
pub(crate) async fn explain(
    pool: &PgPool,
    sql: &str,
    limits: &ExecLimits,
) -> Result<Option<f64>, ConnectorError> {
    let mut budget = *limits;
    budget.max_rows = 1;
    budget.max_bytes = 4 * 1024 * 1024;
    let result = execute(
        pool,
        &format!("EXPLAIN (FORMAT JSON) {sql}"),
        &budget,
        None,
        CancellationToken::new(),
    )
    .await?;
    Ok(result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(cost))
}

pub(crate) async fn health(pool: &PgPool, report: &mut HealthReport) -> Result<(), ConnectorError> {
    let mut connection = GuardedConnection::new(pool.acquire().await.map_err(cancel::error)?);
    let start = Instant::now();
    sqlx::query("SELECT 1")
        .persistent(false)
        .execute(&mut *connection)
        .await
        .map_err(cancel::error)?;
    report.ok = true;
    report.latency_ms = Some(limits::elapsed(start));
    report.server_version = sqlx::query_scalar("SHOW server_version")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok();
    report.is_replica = sqlx::query_scalar("SELECT pg_is_in_recovery()")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok();
    report.active_connections = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok();
    report.max_connections = sqlx::query_scalar::<_, String>("SHOW max_connections")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok()
        .and_then(|value| value.parse().ok());
    connection.clean();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_explain_plan_shape() {
        let plan = serde_json::json!([{"Plan":{"Node Type":"Seq Scan","Relation Name":"users","Alias":"users","Startup Cost":0.00,"Total Cost":18.10,"Plan Rows":810,"Plan Width":36}}]);
        assert_eq!(cost(&plan), Some(18.1));
        assert_eq!(cost(&serde_json::json!([])), None);
        assert_eq!(cost(&serde_json::json!([{"Plan":{"Total Cost":-1}}])), None);
    }
}
