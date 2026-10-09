//! Bounded, deterministic catalog introspection.
use crate::{
    ConnectorError, ExecLimits, SchemaColumn, SchemaInfo, SchemaTable, SchemaTree, TableKind,
    TargetPool,
};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};
use tokio_util::sync::CancellationToken;

const MAX_TABLES: usize = 5000;
const MAX_COLUMNS: usize = 100_000;
const POSTGRES: &str = r#"
WITH tables AS (
 SELECT n.nspname, c.relname, c.relkind::text AS kind, c.oid,
        CASE WHEN c.reltuples < 0 THEN NULL ELSE c.reltuples::bigint END AS row_estimate
 FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid
 WHERE c.relkind IN ('r','p','v','m')
   AND n.nspname NOT IN ('pg_catalog','information_schema')
   AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'
 ORDER BY n.nspname, c.relname LIMIT 5000
)
SELECT t.nspname AS schema_name, t.relname AS table_name, t.kind, t.row_estimate,
       a.attname AS column_name, pg_catalog.format_type(a.atttypid, a.atttypmod) AS data_type,
       NOT a.attnotnull AS nullable,
       EXISTS (SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=t.oid AND i.indisprimary AND a.attnum=ANY(i.indkey)) AS is_primary_key
FROM tables t LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=t.oid AND a.attnum>0 AND NOT a.attisdropped
ORDER BY t.nspname, t.relname, a.attnum LIMIT 100000
"#;
const MYSQL: &str = r#"
SELECT CONVERT(t.TABLE_SCHEMA USING utf8mb4) COLLATE utf8mb4_general_ci AS schema_name,
       CONVERT(t.TABLE_NAME USING utf8mb4) COLLATE utf8mb4_general_ci AS table_name,
       CONVERT(CASE WHEN t.TABLE_TYPE='VIEW' THEN 'v' ELSE 'r' END USING utf8mb4) COLLATE utf8mb4_general_ci AS kind,
       CAST(t.TABLE_ROWS AS SIGNED) AS row_estimate,
       CONVERT(c.COLUMN_NAME USING utf8mb4) COLLATE utf8mb4_general_ci AS column_name,
       CONVERT(c.COLUMN_TYPE USING utf8mb4) COLLATE utf8mb4_general_ci AS data_type, (c.IS_NULLABLE='YES') AS nullable, (c.COLUMN_KEY='PRI') AS is_primary_key
FROM (SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, TABLE_ROWS FROM information_schema.TABLES
      WHERE TABLE_SCHEMA=DATABASE() AND TABLE_SCHEMA NOT IN ('mysql','sys','performance_schema','information_schema')
      ORDER BY TABLE_SCHEMA,TABLE_NAME LIMIT 5000) t
LEFT JOIN information_schema.COLUMNS c ON c.TABLE_SCHEMA=t.TABLE_SCHEMA AND c.TABLE_NAME=t.TABLE_NAME
ORDER BY t.TABLE_SCHEMA,t.TABLE_NAME,c.ORDINAL_POSITION LIMIT 100000
"#;

pub(crate) async fn load(pool: &TargetPool) -> Result<SchemaTree, ConnectorError> {
    let limits = ExecLimits {
        statement_timeout: Duration::from_secs(5),
        lock_timeout: Duration::from_secs(1),
        max_rows: MAX_COLUMNS,
        max_bytes: 64 * 1024 * 1024,
    };
    let sql = match pool {
        TargetPool::Postgres(_) => POSTGRES,
        TargetPool::Mysql(_) => MYSQL,
    };
    let results = pool
        .execute_read(sql, &limits, CancellationToken::new())
        .await?;
    assemble(results.rows)
}
fn text(row: &[Value], index: usize) -> Result<String, ConnectorError> {
    row.get(index)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(catalog_error)
}
fn catalog_error() -> ConnectorError {
    ConnectorError::Database("unexpected schema catalog value".into())
}
fn integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|value| *value >= 0)
}
fn boolean(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_bool()
            .unwrap_or_else(|| value.as_i64().is_some_and(|number| number != 0))
    })
}
fn assemble(rows: Vec<Vec<Value>>) -> Result<SchemaTree, ConnectorError> {
    let mut tables = BTreeMap::<(String, String), SchemaTable>::new();
    let mut columns = 0usize;
    for row in rows {
        let key = (text(&row, 0)?, text(&row, 1)?);
        if tables.len() >= MAX_TABLES && !tables.contains_key(&key) {
            continue;
        }
        let kind = match row.get(2).and_then(Value::as_str) {
            Some("v") => TableKind::View,
            Some("m") => TableKind::MaterializedView,
            Some(_) => TableKind::Table,
            None => return Err(catalog_error()),
        };
        let table = tables.entry(key).or_insert_with_key(|key| SchemaTable {
            name: key.1.clone(),
            kind,
            row_estimate: row.get(3).and_then(integer),
            columns: Vec::new(),
        });
        if row.get(4).is_some_and(|value| !value.is_null()) && columns < MAX_COLUMNS {
            table.columns.push(SchemaColumn {
                name: text(&row, 4)?,
                data_type: text(&row, 5)?,
                nullable: boolean(row.get(6)),
                is_primary_key: boolean(row.get(7)),
            });
            columns += 1;
        }
    }
    let mut schemas = BTreeMap::<String, Vec<SchemaTable>>::new();
    for ((schema, _), table) in tables {
        schemas.entry(schema).or_default().push(table);
    }
    Ok(SchemaTree {
        schemas: schemas
            .into_iter()
            .map(|(name, tables)| SchemaInfo { name, tables })
            .collect(),
    })
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn catalog_caps_table_count_and_rejects_invalid_rows() {
        let rows = (0..=MAX_TABLES)
            .map(|index| {
                vec![
                    json!("public"),
                    json!(format!("table_{index:05}")),
                    json!("r"),
                    json!(10),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                ]
            })
            .collect();
        let tree = assemble(rows).unwrap();
        assert_eq!(tree.schemas.first().unwrap().tables.len(), MAX_TABLES);
        assert!(assemble(vec![vec![]]).is_err());
        assert_eq!(
            integer(&json!("9007199254740993")),
            Some(9_007_199_254_740_993)
        );
    }

    #[test]
    fn catalogs_preserve_column_order_and_sort_schemas_and_tables() {
        let tree = assemble(vec![
            vec![
                json!("z"),
                json!("b"),
                json!("v"),
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
            ],
            vec![
                json!("a"),
                json!("c"),
                json!("r"),
                json!(-1),
                json!("id"),
                json!("bigint"),
                json!(0),
                json!(1),
            ],
            vec![
                json!("a"),
                json!("c"),
                json!("r"),
                json!(-1),
                json!("name"),
                json!("text"),
                json!(true),
                json!(false),
            ],
        ])
        .unwrap();
        let schema = tree.schemas.first().unwrap();
        assert_eq!(schema.name, "a");
        let table = schema.tables.first().unwrap();
        assert_eq!(table.row_estimate, None);
        assert_eq!(table.columns.len(), 2);
        assert!(table.columns.first().unwrap().is_primary_key);
        assert!(!table.columns.first().unwrap().nullable);
        assert!(tree
            .schemas
            .last()
            .unwrap()
            .tables
            .first()
            .unwrap()
            .columns
            .is_empty());
    }
}
