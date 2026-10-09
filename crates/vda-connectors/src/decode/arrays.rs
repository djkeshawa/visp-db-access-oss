//! Multidimensional PostgreSQL arrays, whose shape SQLx's Vec decoder cannot retain.
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{
    postgres::{PgTypeKind, PgValueFormat, PgValueRef},
    TypeInfo, ValueRef,
};

pub(super) fn postgres(raw: PgValueRef<'_>) -> Option<Value> {
    if raw.format() != PgValueFormat::Binary {
        return None;
    }
    let info = raw.type_info();
    let PgTypeKind::Array(element) = info.kind() else {
        return None;
    };
    let is_enum = matches!(element.kind(), PgTypeKind::Enum(_));
    array(raw.as_bytes().ok()?, |bytes| {
        scalar(element.name(), bytes, is_enum)
    })
}
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(length)?;
        let bytes = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(bytes)
    }
    fn int(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
}
fn array(bytes: &[u8], decode: impl Fn(&[u8]) -> Option<Value>) -> Option<Value> {
    let mut reader = Reader { bytes, position: 0 };
    let dimensions = usize::try_from(reader.int()?).ok()?;
    // PostgreSQL itself caps arrays at six dimensions.
    if dimensions > 6 {
        return None;
    }
    reader.take(8)?; // NULL flag and element OID; SQLx has already resolved the element type.
    let mut shape = Vec::with_capacity(dimensions);
    let mut count = usize::from(dimensions != 0);
    for _ in 0..dimensions {
        let length = usize::try_from(reader.int()?).ok()?;
        if length == 0 {
            return None;
        }
        reader.take(4)?; // Lower bounds do not have a JSON-array equivalent.
        count = count.checked_mul(length)?;
        shape.push(length);
    }
    if count > bytes.len().saturating_sub(reader.position) / 4 {
        return None;
    }
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let length = reader.int()?;
        values.push(if length == -1 {
            Value::Null
        } else {
            decode(reader.take(usize::try_from(length).ok()?)?)?
        });
    }
    if reader.position != bytes.len() {
        return None;
    }
    if dimensions == 0 {
        return Some(Value::Array(Vec::new()));
    }
    reshape(&shape, &mut values.into_iter())
}
fn reshape(shape: &[usize], values: &mut impl Iterator<Item = Value>) -> Option<Value> {
    let Some((&length, rest)) = shape.split_first() else {
        return values.next();
    };
    Some(Value::Array(
        (0..length)
            .map(|_| reshape(rest, values))
            .collect::<Option<Vec<_>>>()?,
    ))
}
fn scalar(name: &str, bytes: &[u8], is_enum: bool) -> Option<Value> {
    macro_rules! number {
        ($ty:ty) => {
            <$ty>::from_be_bytes(bytes.try_into().ok()?)
        };
    }
    let name = name.to_ascii_uppercase();
    Some(match name.as_str() {
        "BOOL" if bytes.len() == 1 => Value::Bool(*bytes.first()? != 0),
        "INT2" => super::signed(i64::from(number!(i16))),
        "INT4" => super::signed(i64::from(number!(i32))),
        "INT8" => super::signed(number!(i64)),
        "FLOAT4" => super::float32(number!(f32)),
        "FLOAT8" => super::float(number!(f64)),
        "NUMERIC" => Value::String(super::numeric(bytes)?),
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CITEXT" => {
            Value::String(std::str::from_utf8(bytes).ok()?.to_owned())
        }
        "UUID" => Value::String(uuid::Uuid::from_slice(bytes).ok()?.to_string()),
        "JSON" => serde_json::from_slice(bytes).ok()?,
        "JSONB" if bytes.first() == Some(&1) => serde_json::from_slice(bytes.get(1..)?).ok()?,
        "BYTEA" => super::binary(bytes),
        "DATE" => {
            let days = number!(i32);
            match days {
                i32::MAX => Value::String("Infinity".into()),
                i32::MIN => Value::String("-Infinity".into()),
                _ => Value::String(
                    epoch()?
                        .date()
                        .checked_add_signed(Duration::days(i64::from(days)))?
                        .to_string(),
                ),
            }
        }
        "TIME" => {
            let micros = number!(i64);
            if micros == 86_400_000_000 {
                Value::String("24:00:00".into())
            } else {
                Value::String(
                    NaiveTime::from_num_seconds_from_midnight_opt(
                        u32::try_from(micros / 1_000_000).ok()?,
                        u32::try_from(micros % 1_000_000).ok()? * 1000,
                    )?
                    .to_string(),
                )
            }
        }
        "TIMESTAMP" | "TIMESTAMPTZ" => {
            let micros = number!(i64);
            match micros {
                i64::MAX => Value::String("Infinity".into()),
                i64::MIN => Value::String("-Infinity".into()),
                _ => {
                    let timestamp = epoch()?.checked_add_signed(Duration::microseconds(micros))?;
                    if name == "TIMESTAMP" {
                        super::iso_datetime(timestamp)
                    } else {
                        Value::String(
                            DateTime::<Utc>::from_naive_utc_and_offset(timestamp, Utc).to_rfc3339(),
                        )
                    }
                }
            }
        }
        "INTERVAL" if bytes.len() == 16 => super::interval(sqlx::postgres::types::PgInterval {
            microseconds: i64::from_be_bytes(bytes.get(..8)?.try_into().ok()?),
            days: i32::from_be_bytes(bytes.get(8..12)?.try_into().ok()?),
            months: i32::from_be_bytes(bytes.get(12..16)?.try_into().ok()?),
        }),
        "INET" | "CIDR" | "MACADDR" | "MACADDR8" => Value::String(super::network(bytes, &name)?),
        _ if is_enum => Value::String(std::str::from_utf8(bytes).ok()?.to_owned()),
        _ => return None,
    })
}
fn epoch() -> Option<chrono::NaiveDateTime> {
    NaiveDate::from_ymd_opt(2000, 1, 1)?.and_hms_opt(0, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn payload(shape: &[i32], cells: &[Option<&[u8]>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [shape.len() as i32, 1, 23] {
            bytes.extend(word.to_be_bytes());
        }
        for &length in shape {
            bytes.extend(length.to_be_bytes());
            bytes.extend(1i32.to_be_bytes());
        }
        for cell in cells {
            bytes.extend(cell.map_or(-1, |cell| cell.len() as i32).to_be_bytes());
            if let Some(cell) = cell {
                bytes.extend_from_slice(cell);
            }
        }
        bytes
    }
    #[test]
    fn matrices_preserve_shape_nulls_and_integer_precision() {
        let one = 1i64.to_be_bytes();
        let large = 9_007_199_254_740_993i64.to_be_bytes();
        let bytes = payload(&[2, 2], &[Some(&one), None, Some(&large), Some(&one)]);
        assert_eq!(
            array(&bytes, |cell| scalar("INT8", cell, false)),
            Some(json!([[1, null], ["9007199254740993", 1]]))
        );
        assert_eq!(
            array(&payload(&[], &[]), |cell| scalar("INT4", cell, false)),
            Some(json!([]))
        );
        assert_eq!(
            scalar("TEXT", b"quoted\"text", false),
            Some(json!("quoted\"text"))
        );
    }
    #[test]
    fn malformed_or_excessive_dimensions_are_rejected_without_allocating() {
        assert_eq!(array(&[], |_| Some(Value::Null)), None);
        assert_eq!(
            array(&payload(&[i32::MAX, i32::MAX], &[]), |_| Some(Value::Null)),
            None
        );
        assert_eq!(array(&payload(&[1; 7], &[]), |_| Some(Value::Null)), None);
        assert_eq!(
            array(&payload(&[1], &[Some(b"short")]), |cell| scalar(
                "INT8", cell, false
            )),
            None
        );
    }
}
