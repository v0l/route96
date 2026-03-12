//! Database-backed [`config::AsyncSource`] implementation.
//!
//! [`DbConfigSource`] reads key/value rows from the `config` table and
//! presents them as a config layer that is merged on top of the static
//! `config.yaml` file.  Because it runs after the file source, any key set
//! in the database overrides the same key in the file.
//!
//! Keys use dot-notation to address nested YAML paths
//! (e.g. `"max_upload_bytes"` or `"payments.cost"`).

use std::fmt;

use async_trait::async_trait;
use config::{AsyncSource, ConfigError, Map, Value, ValueKind};

use crate::db::Database;

/// A [`config::AsyncSource`] that loads overrides from the `config` DB table.
#[derive(Clone)]
pub struct DbConfigSource {
    pub db: Database,
}

impl fmt::Debug for DbConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbConfigSource").finish()
    }
}

impl fmt::Display for DbConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database config table")
    }
}

#[async_trait]
impl AsyncSource for DbConfigSource {
    async fn collect(&self) -> Result<Map<String, Value>, ConfigError> {
        let rows = self
            .db
            .config_list()
            .await
            .map_err(|e| ConfigError::Foreign(Box::new(e)))?;

        let mut map = Map::new();
        for (key, raw_value) in rows {
            let value = parse_value(&raw_value);
            // Support nested keys via dot-notation by inserting into nested maps.
            insert_nested(&mut map, &key, value);
        }
        Ok(map)
    }
}

/// Parse a raw string value into the most specific [`Value`] type possible:
/// boolean → bool, integer → i64, float → f64, otherwise string.
fn parse_value(raw: &str) -> Value {
    // Boolean
    match raw.to_lowercase().as_str() {
        "true" => return Value::new(None, ValueKind::Boolean(true)),
        "false" => return Value::new(None, ValueKind::Boolean(false)),
        _ => {}
    }
    // Integer
    if let Ok(i) = raw.parse::<i64>() {
        return Value::new(None, ValueKind::I64(i));
    }
    // Float
    if let Ok(f) = raw.parse::<f64>() {
        return Value::new(None, ValueKind::Float(f));
    }
    // String fallback
    Value::new(None, ValueKind::String(raw.to_owned()))
}

/// Insert `value` into `map` at the path described by `key`, creating
/// intermediate [`ValueKind::Table`] entries as needed.
///
/// For example, key `"payments.cost"` with value `42` produces:
/// ```json
/// { "payments": { "cost": 42 } }
/// ```
fn insert_nested(map: &mut Map<String, Value>, key: &str, value: Value) {
    let mut parts = key.splitn(2, '.');
    let head = parts.next().unwrap();
    match parts.next() {
        None => {
            map.insert(head.to_owned(), value);
        }
        Some(tail) => {
            let sub = map
                .entry(head.to_owned())
                .or_insert_with(|| Value::new(None, ValueKind::Table(Map::new())));
            if let ValueKind::Table(ref mut sub_map) = sub.kind {
                insert_nested(sub_map, tail, value);
            } else {
                // Key conflict: the parent was set to a scalar by an earlier
                // row.  Overwrite it with a table containing the new child.
                let mut sub_map = Map::new();
                insert_nested(&mut sub_map, tail, value);
                *sub = Value::new(None, ValueKind::Table(sub_map));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_map(pairs: &[(&str, &str)]) -> Map<String, Value> {
        let mut map = Map::new();
        for (k, v) in pairs {
            insert_nested(&mut map, k, parse_value(v));
        }
        map
    }

    #[test]
    fn test_parse_bool() {
        assert!(matches!(
            parse_value("true").kind,
            ValueKind::Boolean(true)
        ));
        assert!(matches!(
            parse_value("false").kind,
            ValueKind::Boolean(false)
        ));
        assert!(matches!(
            parse_value("True").kind,
            ValueKind::Boolean(true)
        ));
    }

    #[test]
    fn test_parse_integer() {
        assert!(matches!(parse_value("42").kind, ValueKind::I64(42)));
        assert!(matches!(
            parse_value("104857600").kind,
            ValueKind::I64(104857600)
        ));
    }

    #[test]
    fn test_parse_float() {
        assert!(matches!(parse_value("3.14").kind, ValueKind::Float(_)));
    }

    #[test]
    fn test_parse_string_fallback() {
        assert!(matches!(
            parse_value("hello").kind,
            ValueKind::String(_)
        ));
    }

    #[test]
    fn test_flat_key() {
        let m = collect_map(&[("max_upload_bytes", "104857600")]);
        assert!(m.contains_key("max_upload_bytes"));
    }

    #[test]
    fn test_nested_key() {
        let m = collect_map(&[("payments.cost", "100")]);
        let payments = m.get("payments").unwrap();
        if let ValueKind::Table(ref sub) = payments.kind {
            assert!(sub.contains_key("cost"));
        } else {
            panic!("expected nested table");
        }
    }

    #[test]
    fn test_deeply_nested_key() {
        let m = collect_map(&[("a.b.c", "1")]);
        let a = m.get("a").unwrap();
        if let ValueKind::Table(ref sub_a) = a.kind {
            let b = sub_a.get("b").unwrap();
            if let ValueKind::Table(ref sub_b) = b.kind {
                assert!(sub_b.contains_key("c"));
            } else {
                panic!("expected b to be table");
            }
        } else {
            panic!("expected a to be table");
        }
    }
}
