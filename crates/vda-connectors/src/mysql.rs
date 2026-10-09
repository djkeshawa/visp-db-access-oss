//! MySQL guarded transactions and diagnostics.
use crate::{
    cancel::{self, GuardedConnection},
    decode,
    limits::{self, Results},
    tls, ConnectionSpec, ConnectorError, ExecLimits, HealthReport, PoolOptions, QueryResult,
};
use futures::TryStreamExt;
use sqlx::{mysql::MySqlPoolOptions, Executor, MySqlConnection, MySqlPool, Row};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn pool(spec: &ConnectionSpec, options: PoolOptions) -> MySqlPool {
    let application = spec.application_name.clone();
    MySqlPoolOptions::new()
        .max_connections(options.max_connections)
        .min_connections(0)
        .acquire_timeout(options.acquire_timeout.min(options.connect_timeout))
        .idle_timeout(options.idle_timeout)
        .test_before_acquire(true)
        .after_connect(move |connection, _| {
            let application = application.clone();
            Box::pin(async move {
                match sqlx::query("SET SESSION transaction_read_only = ON")
                    .persistent(false)
                    .execute(&mut *connection)
                    .await
                {
                    Err(sqlx::Error::Database(error))
                        if error
                            .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                            .is_some_and(|error| error.number() == 1193) =>
                    {
                        sqlx::query("SET SESSION tx_read_only = ON")
                            .persistent(false)
                            .execute(&mut *connection)
                            .await?;
                    }
                    Err(error) => return Err(error),
                    Ok(_) => {}
                }
                sqlx::query("SET @vda_application_name = ?")
                    .bind(application)
                    .persistent(false)
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect_lazy_with(tls::mysql(spec))
}
async fn command(connection: &mut MySqlConnection, sql: &str) -> Result<(), ConnectorError> {
    // START TRANSACTION READ ONLY/WRITE is unsupported by MySQL's prepared
    // statement protocol. These commands are trusted, internally generated SQL.
    connection.execute(sql).await.map_err(cancel::error)?;
    Ok(())
}
async fn set_statement_timeout(
    connection: &mut MySqlConnection,
    timeout: std::time::Duration,
) -> Result<(), ConnectorError> {
    let sql = format!(
        "SET SESSION max_execution_time = {}",
        limits::millis(timeout)
    );
    match connection.execute(sql.as_str()).await {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(error))
            if error
                .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                .is_some_and(|error| error.number() == 1193) =>
        {
            // MariaDB uses seconds (including fractions) and also bounds writes.
            let millis = limits::millis(timeout);
            command(
                connection,
                &format!(
                    "SET SESSION max_statement_time = {}.{:03}",
                    millis / 1000,
                    millis % 1000
                ),
            )
            .await
        }
        Err(error) => Err(cancel::error(error)),
    }
}
async fn setup(
    connection: &mut MySqlConnection,
    limits: &ExecLimits,
    write: bool,
) -> Result<(), ConnectorError> {
    // Match the guard's MySQL lexer regardless of server defaults. UTC keeps
    // TIMESTAMP decoding stable; ANSI_QUOTES/NO_BACKSLASH_ESCAPES alter syntax.
    command(
        connection,
        "SET SESSION sql_mode = 'STRICT_TRANS_TABLES,NO_ENGINE_SUBSTITUTION'",
    )
    .await?;
    command(connection, "SET SESSION time_zone = '+00:00'").await?;
    set_statement_timeout(connection, limits.statement_timeout).await?;
    for sql in [
        format!(
            "SET SESSION innodb_lock_wait_timeout = {}",
            limits::seconds(limits.lock_timeout)
        ),
        format!(
            "SET SESSION lock_wait_timeout = {}",
            limits::seconds(limits.lock_timeout)
        ),
    ] {
        command(connection, &sql).await?;
    }
    command(
        connection,
        if write {
            "START TRANSACTION READ WRITE"
        } else {
            "START TRANSACTION READ ONLY"
        },
    )
    .await
}
pub(crate) async fn execute(
    pool: &MySqlPool,
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
    let id = AtomicU64::new(0);
    let operation = async {
        let backend: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
            .persistent(false)
            .fetch_one(&mut *connection)
            .await
            .map_err(cancel::error)?;
        id.store(backend, Ordering::Relaxed);
        let cancel_pool = pool.clone();
        connection.on_discard(move || {
            // Aborting/dropping the execution future must stop work server-side
            // too; SQLx graceful TLS close can otherwise hold the sole pool slot.
            // Dropping outside a runtime (e.g. during shutdown) must not panic.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    cancel::mysql(&cancel_pool, backend).await;
                });
            }
        });
        setup(&mut connection, limits, write_cap.is_some()).await?;
        let result = if let Some(cap) = write_cap {
            write(&mut connection, sql, cap).await
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
                connection.clean();
                Ok(result)
            }
            Err(error) => {
                if command(&mut connection, "ROLLBACK").await.is_ok() {
                    connection.clean();
                }
                Err(error)
            }
        }
    };
    let result = cancel::race(operation, &token, timeout.saturating_sub(start.elapsed())).await;
    if matches!(
        result,
        Err(ConnectorError::Cancelled | ConnectorError::Timeout)
    ) || matches!(&result, Ok(result) if result.truncated && write_cap.is_none())
    {
        connection.discard();
        connection.clear_discard_hook();
        let backend = id.load(Ordering::Relaxed);
        if backend != 0 {
            cancel::mysql(pool, backend).await;
        }
    }
    result.map(|mut result| {
        result.elapsed_ms = limits::elapsed(start);
        result
    })
}
async fn read(
    connection: &mut MySqlConnection,
    sql: &str,
    limits: &ExecLimits,
) -> Result<QueryResult, ConnectorError> {
    let mut results = Results::new(limits);
    results.columns(decode::columns(
        connection
            .describe(sql)
            .await
            .map_err(cancel::error)?
            .columns(),
    ));
    let mut stream = sqlx::query(sql).persistent(false).fetch(&mut *connection);
    while let Some(row) = stream.try_next().await.map_err(cancel::error)? {
        if results.needs_columns() {
            results.columns(decode::mysql_columns(&row));
        }
        if !results.offer(|| decode::mysql(&row)) {
            break;
        }
    }
    Ok(results.finish(None))
}
async fn write(
    connection: &mut MySqlConnection,
    sql: &str,
    cap: u64,
) -> Result<QueryResult, ConnectorError> {
    let affected = sqlx::query(sql)
        .persistent(false)
        .execute(connection)
        .await
        .map_err(cancel::error)?
        .rows_affected();
    if affected > cap {
        return Err(ConnectorError::TooManyAffectedRows {
            actual: affected,
            limit: cap,
        });
    }
    Ok(QueryResult {
        columns: Vec::new(),
        rows: Vec::new(),
        row_count: 0,
        truncated: false,
        affected_rows: Some(affected),
        elapsed_ms: 0,
    })
}
pub(crate) fn cost(plan: &serde_json::Value) -> Option<f64> {
    let block = plan.get("query_block")?;
    let value = block
        .get("cost_info")
        .and_then(|info| info.get("query_cost"))
        .or_else(|| block.get("cost"))?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}
pub(crate) async fn explain(
    pool: &MySqlPool,
    sql: &str,
    limits: &ExecLimits,
) -> Result<Option<f64>, ConnectorError> {
    let mut budget = *limits;
    budget.max_rows = 1;
    budget.max_bytes = 4 * 1024 * 1024;
    let result = execute(
        pool,
        &format!("EXPLAIN FORMAT=JSON {sql}"),
        &budget,
        None,
        CancellationToken::new(),
    )
    .await?;
    let plan = result.rows.first().and_then(|row| row.first());
    Ok(plan.and_then(|value| {
        if let Some(text) = value.as_str() {
            serde_json::from_str(text).ok().as_ref().and_then(cost)
        } else {
            cost(value)
        }
    }))
}
pub(crate) async fn health(
    pool: &MySqlPool,
    report: &mut HealthReport,
) -> Result<(), ConnectorError> {
    let mut connection = GuardedConnection::new(pool.acquire().await.map_err(cancel::error)?);
    let start = Instant::now();
    sqlx::query("SELECT 1")
        .persistent(false)
        .execute(&mut *connection)
        .await
        .map_err(cancel::error)?;
    report.ok = true;
    report.latency_ms = Some(limits::elapsed(start));
    report.server_version = sqlx::query_scalar("SELECT VERSION()")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok();
    report.is_replica = sqlx::query_scalar::<_, i64>("SELECT @@global.read_only")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok()
        .map(|value| value != 0);
    let replica = match sqlx::query("SHOW REPLICA STATUS")
        .persistent(false)
        .fetch_optional(&mut *connection)
        .await
    {
        Ok(row) => Some(row.is_some()),
        Err(_) => sqlx::query("SHOW SLAVE STATUS")
            .persistent(false)
            .fetch_optional(&mut *connection)
            .await
            .ok()
            .map(|row| row.is_some()),
    };
    if replica == Some(true) {
        report.is_replica = Some(true);
    } else if report.is_replica.is_none() {
        report.is_replica = replica;
    }
    report.active_connections = sqlx::query("SHOW GLOBAL STATUS LIKE 'Threads_connected'")
        .persistent(false)
        .fetch_optional(&mut *connection)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<String, _>(1).ok()?.parse().ok());
    report.max_connections = sqlx::query_scalar::<_, u64>("SELECT @@max_connections")
        .persistent(false)
        .fetch_one(&mut *connection)
        .await
        .ok()
        .and_then(|value| i64::try_from(value).ok());
    connection.clean();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_real_explain_plan_shape() {
        let plan = serde_json::json!({"query_block":{"select_id":1,"cost_info":{"query_cost":"1.20"},"table":{"table_name":"users","access_type":"ALL","rows_examined_per_scan":2,"cost_info":{"read_cost":"1.00","eval_cost":"0.20"}}}});
        assert_eq!(cost(&plan), Some(1.2));
        assert_eq!(
            cost(&serde_json::json!({"query_block":{"cost_info":{"query_cost":4.25}}})),
            Some(4.25)
        );
        assert_eq!(
            cost(&serde_json::json!({"query_block":{"cost":0.0597548}})),
            Some(0.0597548)
        );
        assert_eq!(cost(&serde_json::json!({})), None);
        assert_eq!(
            cost(&serde_json::json!({"query_block":{"cost_info":{"query_cost":"NaN"}}})),
            None
        );
    }
}
