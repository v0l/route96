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
use serde_json::Value as JsonValue;

use crate::db::Database;
use crate::settings::Settings;

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

/// Seed the `config` table from the current `Settings` loaded from the static
/// config file.  Only scalar leaf values are inserted; complex types (arrays,
/// nested objects beyond one level, secrets) are skipped.
///
/// Uses `INSERT IGNORE` semantics — existing DB overrides are never
/// overwritten, so the admin can change a value at runtime without it being
/// reverted on the next restart.
pub async fn seed_from_settings(db: &Database, settings: &Settings) -> anyhow::Result<()> {
    let json = serde_json::to_value(settings)?;
    let pairs = flatten_json("", &json);
    for (key, value) in pairs {
        // Skip keys the admin UI cannot meaningfully display or edit, and keys
        // that contain secrets / filesystem paths that differ per deployment.
        if should_skip(&key) {
            continue;
        }
        // INSERT IGNORE: if the key already exists in the DB keep that value.
        db.config_seed(&key, &value).await?;
    }
    Ok(())
}

/// Keys whose values we deliberately do not seed into the database.
fn should_skip(key: &str) -> bool {
    // Secrets and deployment-specific paths that must not be overridden via UI
    const SKIP: &[&str] = &[
        "database",
        "storage_dir",  // server-local path, not safe to change via UI
        "listen",       // requires restart, not a runtime config
        "models_dir",
        // whitelist serialises as a complex type; the UI manages it directly
        "whitelist",
        // payments sub-tree contains LND credentials
        "payments",
    ];
    SKIP.iter().any(|s| key == *s || key.starts_with(&format!("{}.", s)))
}

/// Recursively flatten a JSON value into `(dot.notation.key, string_value)` pairs.
/// Only scalar leaves (string, number, bool) are emitted; nulls and arrays are skipped.
fn flatten_json(prefix: &str, value: &JsonValue) -> Vec<(String, String)> {
    match value {
        JsonValue::Object(map) => {
            let mut out = Vec::new();
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                out.extend(flatten_json(&key, v));
            }
            out
        }
        JsonValue::String(s) => vec![(prefix.to_owned(), s.clone())],
        JsonValue::Number(n) => vec![(prefix.to_owned(), n.to_string())],
        JsonValue::Bool(b) => vec![(prefix.to_owned(), b.to_string())],
        // Skip nulls (Option::None fields) and arrays (too complex for scalar storage)
        JsonValue::Null | JsonValue::Array(_) => vec![],
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
    fn test_flatten_json_scalars() {
        let v: JsonValue = serde_json::json!({
            "a": "hello",
            "b": 42,
            "c": true,
            "d": null,
            "e": ["x", "y"]
        });
        let pairs = flatten_json("", &v);
        let map: std::collections::HashMap<_, _> = pairs.into_iter().collect();
        assert_eq!(map["a"], "hello");
        assert_eq!(map["b"], "42");
        assert_eq!(map["c"], "true");
        assert!(!map.contains_key("d")); // null skipped
        assert!(!map.contains_key("e")); // array skipped
    }

    #[test]
    fn test_flatten_json_nested() {
        let v: JsonValue = serde_json::json!({ "outer": { "inner": 99 } });
        let pairs = flatten_json("", &v);
        assert_eq!(pairs, vec![("outer.inner".to_string(), "99".to_string())]);
    }

    #[test]
    fn test_should_skip() {
        assert!(should_skip("database"));
        assert!(should_skip("payments"));
        assert!(should_skip("payments.lnd.tls"));
        assert!(should_skip("storage_dir"));
        assert!(!should_skip("max_upload_bytes"));
        assert!(!should_skip("public_url"));
        assert!(!should_skip("webhook_url"));
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
