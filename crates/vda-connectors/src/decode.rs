//! Lossless, panic-free conversion of database cells to JSON values.
mod arrays;

use crate::Column;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{
    mysql::{
        types::{MySqlTime, MySqlTimeSign},
        MySqlRow,
    },
    postgres::{
        types::PgInterval, PgHasArrayType, PgRow, PgTypeInfo, PgTypeKind, PgValueFormat, PgValueRef,
    },
    Column as _, Row, TypeInfo, ValueRef,
};
use std::fmt::Write as _;

const JS_SAFE_INTEGER: u64 = (1 << 53) - 1;
pub(crate) fn signed(value: i64) -> Value {
    if value.unsigned_abs() > JS_SAFE_INTEGER {
        Value::String(value.to_string())
    } else {
        value.into()
    }
}
pub(crate) fn unsigned(value: u64) -> Value {
    if value > JS_SAFE_INTEGER {
        Value::String(value.to_string())
    } else {
        value.into()
    }
}
fn float(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map_or_else(|| Value::String(value.to_string()), Value::Number)
}
/// FLOAT4 values keep the shortest decimal that identifies them (0.1, not 0.10000000149011612).
fn float32(value: f32) -> Value {
    float(if value.is_finite() {
        value
            .to_string()
            .parse()
            .unwrap_or_else(|_| f64::from(value))
    } else {
        f64::from(value)
    })
}
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().unwrap_or(0);
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        for index in [first >> 2, ((first & 3) << 4) | (second >> 4)] {
            result.push(char::from(
                ALPHABET.get(usize::from(index)).copied().unwrap_or(b'='),
            ));
        }
        result.push(if chunk.len() > 1 {
            char::from(
                ALPHABET
                    .get(usize::from(((second & 15) << 2) | (third >> 6)))
                    .copied()
                    .unwrap_or(b'='),
            )
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            char::from(
                ALPHABET
                    .get(usize::from(third & 63))
                    .copied()
                    .unwrap_or(b'='),
            )
        } else {
            '='
        });
    }
    result
}

fn binary(value: &[u8]) -> Value {
    let mut encoded = format!(
        "base64:{}",
        base64(value.get(..value.len().min(4096)).unwrap_or_default())
    );
    if value.len() > 4096 {
        encoded.push_str("…(truncated)");
    }
    Value::String(encoded)
}
fn iso_datetime(value: NaiveDateTime) -> Value {
    Value::String(value.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
}
fn interval(value: PgInterval) -> Value {
    Value::String(format!(
        "P{}M{}DT{}{}.{:06}S",
        value.months,
        value.days,
        if value.microseconds < 0 { "-" } else { "" },
        value.microseconds.unsigned_abs() / 1_000_000,
        value.microseconds.unsigned_abs() % 1_000_000
    ))
}
fn mysql_time(value: MySqlTime) -> Value {
    let fraction = if value.microseconds() == 0 {
        String::new()
    } else {
        format!(".{:06}", value.microseconds())
            .trim_end_matches('0')
            .to_owned()
    };
    let text = if value.is_valid_time_of_day() {
        format!(
            "{:02}:{:02}:{:02}{fraction}",
            value.hours(),
            value.minutes(),
            value.seconds()
        )
    } else {
        format!(
            "{}PT{}H{}M{}{fraction}S",
            if value.sign() == MySqlTimeSign::Negative {
                "-"
            } else {
                ""
            },
            value.hours(),
            value.minutes(),
            value.seconds()
        )
    };
    Value::String(text)
}
fn unsupported(name: &str) -> Value {
    Value::String(format!("<unsupported type: {name}>"))
}

/// Driver column metadata (from a row or a prepared statement) as result columns.
pub(crate) fn columns<C: sqlx::Column>(columns: &[C]) -> Vec<Column> {
    columns
        .iter()
        .map(|column| Column {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}
pub(crate) fn pg_columns(row: &PgRow) -> Vec<Column> {
    columns(row.columns())
}
pub(crate) fn mysql_columns(row: &MySqlRow) -> Vec<Column> {
    columns(row.columns())
}
pub(crate) fn postgres(row: &PgRow) -> Vec<Value> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| pg_cell(row, index, column.type_info()))
        .collect()
}
pub(crate) fn mysql(row: &MySqlRow) -> Vec<Value> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| mysql_cell(row, index, column.type_info().name()))
        .collect()
}

fn pg_cell(row: &PgRow, index: usize, info: &PgTypeInfo) -> Value {
    if row.try_get_raw(index).is_ok_and(|value| value.is_null()) {
        return Value::Null;
    }
    let name = info.name();
    macro_rules! get {
        ($ty:ty, $convert:expr) => {
            row.try_get::<$ty, _>(index).ok().map($convert)
        };
    }
    macro_rules! array {
        ($ty:ty, $convert:expr) => {
            get!(Vec<Option<$ty>>, |values: Vec<Option<$ty>>| Value::Array(
                values
                    .into_iter()
                    .map(|v| v.map_or(Value::Null, $convert))
                    .collect()
            ))
        };
    }
    let value = match name {
        "BOOL" => get!(bool, Value::Bool),
        "INT2" => get!(i16, |v| signed(i64::from(v))),
        "INT4" => get!(i32, |v| signed(i64::from(v))),
        "INT8" => get!(i64, signed),
        "FLOAT4" => get!(f32, float32),
        "FLOAT8" => get!(f64, float),
        "NUMERIC" => get!(ExactNumeric, |v: ExactNumeric| Value::String(v.0)),
        "UUID" => get!(uuid::Uuid, |v: uuid::Uuid| Value::String(v.to_string())),
        "DATE" => get!(NaiveDate, |v: NaiveDate| Value::String(v.to_string())),
        "TIME" => get!(NaiveTime, |v: NaiveTime| Value::String(v.to_string())),
        "TIMESTAMP" => get!(NaiveDateTime, iso_datetime),
        "TIMESTAMPTZ" => get!(DateTime<Utc>, |v: DateTime<Utc>| Value::String(
            v.to_rfc3339()
        )),
        "INTERVAL" => get!(PgInterval, interval),
        "JSON" | "JSONB" => get!(Value, |v| v),
        "BYTEA" => get!(&[u8], binary),
        "INET" | "CIDR" | "MACADDR" | "MACADDR8" => row
            .try_get_raw(index)
            .ok()
            .and_then(|raw| pg_network(raw, name)),
        "BOOL[]" => array!(bool, Value::Bool),
        "INT2[]" => array!(i16, |v| signed(i64::from(v))),
        "INT4[]" => array!(i32, |v| signed(i64::from(v))),
        "INT8[]" => array!(i64, signed),
        "FLOAT4[]" => array!(f32, float32),
        "FLOAT8[]" => array!(f64, float),
        "NUMERIC[]" => array!(ExactNumeric, |v: ExactNumeric| Value::String(v.0)),
        "UUID[]" => array!(uuid::Uuid, |v: uuid::Uuid| Value::String(v.to_string())),
        "DATE[]" => array!(NaiveDate, |v: NaiveDate| Value::String(v.to_string())),
        "TIME[]" => array!(NaiveTime, |v: NaiveTime| Value::String(v.to_string())),
        "TIMESTAMP[]" => array!(NaiveDateTime, iso_datetime),
        "TIMESTAMPTZ[]" => array!(DateTime<Utc>, |v: DateTime<Utc>| Value::String(
            v.to_rfc3339()
        )),
        "INTERVAL[]" => array!(PgInterval, interval),
        "JSON[]" | "JSONB[]" => array!(Value, |v| v),
        "BYTEA[]" => array!(Vec<u8>, |v: Vec<u8>| binary(&v)),
        "TEXT[]" | "VARCHAR[]" | "BPCHAR[]" | "NAME[]" | "CITEXT[]" => {
            array!(String, Value::String)
        }
        _ if matches!(info.kind(), PgTypeKind::Enum(_)) => row
            .try_get_unchecked::<String, _>(index)
            .ok()
            .map(Value::String),
        _ => get!(String, Value::String),
    };
    value
        .or_else(|| row.try_get_raw(index).ok().and_then(arrays::postgres))
        .unwrap_or_else(|| unsupported(name))
}

fn mysql_cell(row: &MySqlRow, index: usize, name: &str) -> Value {
    if row.try_get_raw(index).is_ok_and(|value| value.is_null()) {
        return Value::Null;
    }
    macro_rules! get {
        ($ty:ty, $convert:expr) => {
            row.try_get::<$ty, _>(index).ok().map($convert)
        };
    }
    let value = match name {
        "BOOLEAN" => get!(bool, Value::Bool),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => get!(i64, signed),
        "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED"
        | "BIGINT UNSIGNED" | "YEAR" => get!(u64, unsigned),
        "FLOAT" => get!(f32, float32),
        "DOUBLE" => get!(f64, float),
        // Decimal's wire payload is ASCII in both MySQL protocols; avoid fixed precision types.
        "DECIMAL" => row
            .try_get_unchecked::<String, _>(index)
            .ok()
            .map(Value::String),
        "JSON" => get!(Value, |v| v),
        "DATE" => get!(NaiveDate, |v: NaiveDate| Value::String(v.to_string())),
        "DATETIME" => get!(NaiveDateTime, iso_datetime),
        "TIMESTAMP" => get!(DateTime<Utc>, |v: DateTime<Utc>| Value::String(
            v.to_rfc3339()
        )),
        "TIME" => get!(MySqlTime, mysql_time),
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => {
            get!(&[u8], binary)
        }
        "BIT" => row.try_get_unchecked::<&[u8], _>(index).ok().map(binary),
        "ENUM" | "SET" => row
            .try_get_unchecked::<String, _>(index)
            .ok()
            .map(Value::String),
        _ => get!(String, Value::String),
    };
    value.unwrap_or_else(|| unsupported(name))
}

#[derive(Debug)]
struct ExactNumeric(String);
impl sqlx::Type<sqlx::Postgres> for ExactNumeric {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("NUMERIC")
    }
}
impl PgHasArrayType for ExactNumeric {
    fn array_type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("NUMERIC[]")
    }
}
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for ExactNumeric {
    fn decode(value: PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        if value.format() == PgValueFormat::Text {
            return Ok(Self(value.as_str()?.to_owned()));
        }
        numeric(value.as_bytes()?)
            .map(Self)
            .ok_or_else(|| "invalid NUMERIC payload".into())
    }
}
fn word(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn numeric(bytes: &[u8]) -> Option<String> {
    let count = usize::from(word(bytes, 0)?);
    let weight = i32::from(word(bytes, 2)? as i16);
    let sign = word(bytes, 4)?;
    let scale = usize::from(word(bytes, 6)?);
    match sign {
        0xc000 => return Some("NaN".into()),
        0xd000 => return Some("Infinity".into()),
        0xf000 => return Some("-Infinity".into()),
        0 | 0x4000 => {}
        _ => return None,
    }
    if bytes.len() != 8usize.checked_add(count.checked_mul(2)?)? || scale > 16383 {
        return None;
    }
    let groups = (0..count)
        .map(|i| word(bytes, 8 + i * 2).filter(|v| *v < 10000))
        .collect::<Option<Vec<_>>>()?;
    let digit = |position: i32| -> u16 {
        usize::try_from(weight - position)
            .ok()
            .and_then(|index| groups.get(index).copied())
            .unwrap_or(0)
    };
    let mut result = String::new();
    if sign == 0x4000 {
        result.push('-');
    }
    if weight < 0 {
        result.push('0');
    } else {
        for position in (0..=weight).rev() {
            let _ = if position == weight {
                write!(result, "{}", digit(position))
            } else {
                write!(result, "{:04}", digit(position))
            };
        }
    }
    if scale > 0 {
        result.push('.');
        let start = result.len();
        for position in 1..=scale.div_ceil(4) {
            let _ = write!(result, "{:04}", digit(-(position as i32)));
        }
        result.truncate(start + scale);
    }
    Some(result)
}
fn pg_network(value: PgValueRef<'_>, name: &str) -> Option<Value> {
    if value.format() == PgValueFormat::Text {
        return value.as_str().ok().map(|v| Value::String(v.to_owned()));
    }
    network(value.as_bytes().ok()?, name).map(Value::String)
}
fn network(bytes: &[u8], name: &str) -> Option<String> {
    if name.starts_with("MACADDR") {
        let expected = if name == "MACADDR8" { 8 } else { 6 };
        return (bytes.len() == expected).then(|| {
            bytes
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<Vec<_>>()
                .join(":")
        });
    }
    let family = *bytes.first()?;
    let prefix = *bytes.get(1)?;
    let length = *bytes.get(3)?;
    let address = match (family, length) {
        (2, 4) if prefix <= 32 => std::net::IpAddr::V4(std::net::Ipv4Addr::from(
            <[u8; 4]>::try_from(bytes.get(4..8)?).ok()?,
        )),
        (3, 16) if prefix <= 128 => std::net::IpAddr::V6(std::net::Ipv6Addr::from(
            <[u8; 16]>::try_from(bytes.get(4..20)?).ok()?,
        )),
        _ => return None,
    };
    Some(
        if name == "CIDR" || prefix != if family == 2 { 32 } else { 128 } {
            format!("{address}/{prefix}")
        } else {
            address.to_string()
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn integers_keep_javascript_precision() {
        assert_eq!(signed(42), serde_json::json!(42));
        assert_eq!(signed(i64::MIN), Value::String(i64::MIN.to_string()));
        assert_eq!(unsigned(u64::MAX), Value::String(u64::MAX.to_string()));
        assert!(signed(JS_SAFE_INTEGER as i64).is_number());
        assert!(signed((JS_SAFE_INTEGER + 1) as i64).is_string());
        assert!(unsigned(JS_SAFE_INTEGER).is_number());
    }
    #[test]
    fn non_finite_floats_and_binary_are_safe() {
        assert_eq!(float(f64::NAN), "NaN");
        assert_eq!(float(f64::INFINITY), "inf");
        assert_eq!(binary(b"abc"), "base64:YWJj");
        assert_eq!(binary(b""), "base64:");
        assert_eq!(binary(b"f"), "base64:Zg==");
        assert_eq!(binary(b"fo"), "base64:Zm8=");
        assert_eq!(binary(b"foobar"), "base64:Zm9vYmFy");
        let value = binary(&vec![0; 4097]);
        assert!(value.as_str().unwrap().ends_with("…(truncated)"));
        assert_eq!(
            value.as_str().unwrap().len(),
            7 + 4096usize.div_ceil(3) * 4 + "…(truncated)".len()
        );
    }
    #[test]
    fn float4_keeps_its_shortest_decimal_form() {
        assert_eq!(float32(0.1), serde_json::json!(0.1));
        assert_eq!(float32(1.5), serde_json::json!(1.5));
        assert_eq!(float32(-16_777_216.0), serde_json::json!(-16_777_216.0));
        assert_eq!(float32(f32::NAN), "NaN");
        assert_eq!(float32(f32::INFINITY), "inf");
    }
    #[test]
    fn exact_numeric_handles_scale_sign_and_large_magnitudes() {
        let encode = |weight: i16, sign: u16, scale: u16, digits: &[u16]| {
            [digits.len() as u16, weight as u16, sign, scale]
                .into_iter()
                .chain(digits.iter().copied())
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            numeric(&encode(0, 0, 4, &[12, 3400])),
            Some("12.3400".into())
        );
        assert_eq!(
            numeric(&encode(-2, 0x4000, 9, &[1234, 5000])),
            Some("-0.000012345".into())
        );
        assert_eq!(
            numeric(&encode(10, 0, 0, &[1])),
            Some(format!("1{}", "0".repeat(40)))
        );
        assert_eq!(numeric(&encode(0, 0xc000, 0, &[])), Some("NaN".into()));
        assert_eq!(numeric(&[]), None);
        assert_eq!(numeric(&encode(0, 0, 0, &[10000])), None);
    }
    #[test]
    fn mysql_time_uses_iso_clock_or_duration_strings() {
        let time = MySqlTime::new(MySqlTimeSign::Positive, 3, 4, 5, 0).unwrap();
        assert_eq!(mysql_time(time), "03:04:05");
        let duration = MySqlTime::new(MySqlTimeSign::Negative, 30, 4, 5, 123400).unwrap();
        assert_eq!(mysql_time(duration), "-PT30H4M5.1234S");
    }

    #[test]
    fn network_and_interval_strings() {
        assert_eq!(
            network(&[2, 24, 1, 4, 192, 168, 1, 0], "CIDR"),
            Some("192.168.1.0/24".into())
        );
        assert_eq!(
            network(&[0, 1, 2, 3, 4, 255], "MACADDR"),
            Some("00:01:02:03:04:ff".into())
        );
        assert_eq!(network(&[2], "INET"), None);
        assert_eq!(
            interval(PgInterval {
                months: 1,
                days: 2,
                microseconds: -1
            }),
            "P1M2DT-0.000001S"
        );
    }
}
