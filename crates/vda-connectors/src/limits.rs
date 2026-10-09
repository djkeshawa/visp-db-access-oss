//! Runtime budgets and bounded result collection.

use crate::{Column, ExecLimits, QueryResult};
use serde_json::Value;
use std::time::{Duration, Instant};

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
        .min(i32::MAX as u64)
}
pub(crate) fn seconds(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
        .max(1)
        .min(i32::MAX as u64)
}
pub(crate) fn hard_timeout(limits: &ExecLimits) -> Duration {
    Duration::from_millis(millis(limits.statement_timeout)).saturating_add(Duration::from_secs(2))
}
pub(crate) fn elapsed(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn json_bytes(value: &Value) -> usize {
    match value {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(number) => number_bytes(number),
        Value::String(string) => string_bytes(string),
        Value::Array(values) => values.iter().fold(
            2usize.saturating_add(values.len().saturating_sub(1)),
            |sum, value| sum.saturating_add(json_bytes(value)),
        ),
        Value::Object(values) => values.iter().fold(
            2usize.saturating_add(values.len().saturating_sub(1)),
            |sum, (key, value)| {
                sum.saturating_add(string_bytes(key))
                    .saturating_add(1)
                    .saturating_add(json_bytes(value))
            },
        ),
    }
}
/// Serialized width of a JSON number; integers are counted without allocating.
fn number_bytes(number: &serde_json::Number) -> usize {
    let digits = |value: u64| value.checked_ilog10().map_or(1, |log| log as usize + 1);
    if let Some(value) = number.as_u64() {
        digits(value)
    } else if let Some(value) = number.as_i64() {
        digits(value.unsigned_abs()).saturating_add(usize::from(value < 0))
    } else {
        number.to_string().len()
    }
}
fn string_bytes(string: &str) -> usize {
    string.bytes().fold(2usize, |sum, byte| {
        sum.saturating_add(match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        })
    })
}

pub(crate) struct Results<'a> {
    limits: &'a ExecLimits,
    columns: Vec<Column>,
    rows: Vec<Vec<Value>>,
    bytes: usize,
    truncated: bool,
}
impl<'a> Results<'a> {
    pub(crate) fn new(limits: &'a ExecLimits) -> Self {
        Self {
            limits,
            columns: Vec::new(),
            rows: Vec::new(),
            bytes: 2,
            truncated: false,
        }
    }
    pub(crate) fn columns(&mut self, columns: Vec<Column>) {
        if self.columns.is_empty() {
            self.columns = columns;
        }
    }
    /// True until column metadata is known; lets callers skip building it per row.
    pub(crate) fn needs_columns(&self) -> bool {
        self.columns.is_empty()
    }
    pub(crate) fn full(&self) -> bool {
        self.rows.len() >= self.limits.max_rows || self.truncated
    }
    /// Retains the row built by `row` unless a budget is exhausted; the row is
    /// only decoded when it can still be kept. Returns whether it was kept.
    pub(crate) fn offer(&mut self, row: impl FnOnce() -> Vec<Value>) -> bool {
        if self.full() {
            self.truncated = true;
            return false;
        }
        self.push(row())
    }
    pub(crate) fn push(&mut self, row: Vec<Value>) -> bool {
        let size = row.iter().fold(
            2usize.saturating_add(row.len().saturating_sub(1)),
            |sum, value| sum.saturating_add(json_bytes(value)),
        );
        let bytes = self
            .bytes
            .saturating_add(size)
            .saturating_add(usize::from(!self.rows.is_empty()));
        if self.full() || bytes > self.limits.max_bytes {
            self.truncated = true;
            return false;
        }
        self.bytes = bytes;
        self.rows.push(row);
        true
    }
    pub(crate) fn finish(self, affected_rows: Option<u64>) -> QueryResult {
        QueryResult {
            columns: self.columns,
            row_count: self.rows.len(),
            rows: self.rows,
            truncated: self.truncated,
            affected_rows,
            elapsed_ms: 0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_size_accounts_for_escaping_and_nested_values() {
        let value = json!([null, true, 42, "\n\"\\", {"key": "value"}]);
        assert_eq!(
            json_bytes(&value),
            serde_json::to_vec(&value).unwrap().len()
        );
    }

    #[test]
    fn caps_discard_the_overflow_row_and_detect_one_extra_row() {
        let limits = crate::ExecLimits {
            statement_timeout: std::time::Duration::from_secs(1),
            lock_timeout: std::time::Duration::from_secs(1),
            max_rows: 1,
            max_bytes: 100,
        };
        let mut result = Results::new(&limits);
        assert!(result.push(vec![json!(1)]));
        assert!(!result.push(vec![json!(2)]));
        let result = result.finish(None);
        assert_eq!(result.rows, vec![vec![json!(1)]]);
        assert!(result.truncated);
        let mut limits = limits;
        limits.max_bytes = 2;
        let mut result = Results::new(&limits);
        assert!(!result.push(vec![json!("large")]));
        assert!(result.finish(None).rows.is_empty());
    }

    #[test]
    fn json_size_of_numbers_matches_serialization() {
        for value in [
            json!(0),
            json!(-1),
            json!(9),
            json!(10),
            json!(-10),
            json!(i64::MIN),
            json!(i64::MAX),
            json!(u64::MAX),
            json!(1.5),
            json!(-0.000_1),
        ] {
            assert_eq!(
                json_bytes(&value),
                serde_json::to_vec(&value).unwrap().len(),
                "{value}"
            );
        }
    }

    #[test]
    fn offer_decodes_rows_only_while_budget_remains() {
        let limits = crate::ExecLimits {
            statement_timeout: std::time::Duration::from_secs(1),
            lock_timeout: std::time::Duration::from_secs(1),
            max_rows: 1,
            max_bytes: 100,
        };
        let mut result = Results::new(&limits);
        assert!(result.offer(|| vec![json!(1)]));
        let decoded = std::cell::Cell::new(false);
        assert!(!result.offer(|| {
            decoded.set(true);
            vec![json!(2)]
        }));
        assert!(!decoded.get(), "row decoded after the cap was reached");
        let result = result.finish(None);
        assert_eq!(result.rows, vec![vec![json!(1)]]);
        assert!(result.truncated);
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;
    #[test]
    fn timeouts_never_disable_server_limits_or_overflow() {
        assert_eq!(millis(Duration::ZERO), 1);
        assert_eq!(seconds(Duration::ZERO), 1);
        assert_eq!(seconds(Duration::from_millis(1001)), 2);
        assert_eq!(millis(Duration::MAX), i32::MAX as u64);
        assert_eq!(seconds(Duration::MAX), i32::MAX as u64);
    }
}
