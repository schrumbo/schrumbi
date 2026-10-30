use serde::Serialize;
use serde_json::Value;

/// Serializes like Swift's `JSONEncoder` with `.prettyPrinted, .sortedKeys, .withoutEscapingSlashes`,
/// so files written by either app stay diff-identical.
pub fn to_swift_json<T: Serialize>(value: &T) -> serde_json::Result<String> {
    let v = serde_json::to_value(value)?;
    let mut out = String::new();
    write(&v, 0, &mut out);
    Ok(out)
}

fn indent(n: usize, out: &mut String) {
    out.extend(std::iter::repeat_n("  ", n));
}

fn write(v: &Value, depth: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0. && f.abs() < 1e15 => out.push_str(&format!("{}", f as i64)),
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
        Value::Array(items) => {
            out.push_str("[\n");
            if items.is_empty() {
                out.push('\n');
            }
            for (i, item) in items.iter().enumerate() {
                indent(depth + 1, out);
                write(item, depth + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(depth, out);
            out.push(']');
        }
        Value::Object(map) => {
            out.push_str("{\n");
            if map.is_empty() {
                out.push('\n');
            }
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                indent(depth + 1, out);
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push_str(" : ");
                write(&map[*k], depth + 1, out);
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(depth, out);
            out.push('}');
        }
    }
}

/// ISO 8601 dates with whole seconds, as Swift's `.iso8601` strategy reads and writes them.
pub mod iso8601 {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&d.to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let s = String::deserialize(d)?;
        DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc)).map_err(serde::de::Error::custom)
    }

    pub mod map {
        use std::collections::BTreeMap;

        use chrono::{DateTime, SecondsFormat, Utc};
        use serde::ser::SerializeMap;
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(m: &BTreeMap<String, DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
            let mut map = s.serialize_map(Some(m.len()))?;
            for (k, v) in m {
                map.serialize_entry(k, &v.to_rfc3339_opts(SecondsFormat::Secs, true))?;
            }
            map.end()
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, DateTime<Utc>>, D::Error> {
            BTreeMap::<String, String>::deserialize(d)?
                .into_iter()
                .map(|(k, v)| DateTime::parse_from_rfc3339(&v).map(|d| (k, d.with_timezone(&Utc))).map_err(serde::de::Error::custom))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_swift_pretty_printing() {
        let v = serde_json::json!({"b": [], "a": 20.0, "c": [1, {"x": "a/b"}], "d": {}});
        assert_eq!(to_swift_json(&v).unwrap(), "{\n  \"a\" : 20,\n  \"b\" : [\n\n  ],\n  \"c\" : [\n    1,\n    {\n      \"x\" : \"a/b\"\n    }\n  ],\n  \"d\" : {\n\n  }\n}");
    }
}
