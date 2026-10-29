use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error, ser::SerializeMap};

use super::{KeyValue, integer};

/// OTLP's AnyValue oneof, including an explicitly empty value.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AnyValue {
    #[default]
    Empty,
    String(String),
    Bool(bool),
    Int(i64),
    Double(f64),
    Array(Vec<AnyValue>),
    Map(Vec<KeyValue>),
    Bytes(Vec<u8>),
}

#[derive(Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de> + Default"))]
struct Values<T> {
    #[serde(default)]
    values: T,
}

impl Serialize for AnyValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        match self {
            Self::Empty => {}
            Self::String(v) => map.serialize_entry("stringValue", v)?,
            Self::Bool(v) => map.serialize_entry("boolValue", v)?,
            Self::Int(v) => map.serialize_entry("intValue", &v.to_string())?,
            Self::Double(v) if v.is_nan() => map.serialize_entry("doubleValue", "NaN")?,
            Self::Double(v) if *v == f64::INFINITY => {
                map.serialize_entry("doubleValue", "Infinity")?
            }
            Self::Double(v) if *v == f64::NEG_INFINITY => {
                map.serialize_entry("doubleValue", "-Infinity")?
            }
            Self::Double(v) => map.serialize_entry("doubleValue", v)?,
            Self::Array(v) => map.serialize_entry("arrayValue", &Values { values: v })?,
            Self::Map(v) => map.serialize_entry("kvlistValue", &Values { values: v })?,
            Self::Bytes(v) => map.serialize_entry("bytesValue", &encode_base64(v))?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AnyValue {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a double other than a JSON string goes to serde_json, to read as a number or reject"
    )]
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let fields = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let mut selected = None;
        for (key, value) in fields {
            if value.is_null() {
                continue;
            }
            let parsed = match key.as_str() {
                "stringValue" => {
                    Self::String(serde_json::from_value(value).map_err(D::Error::custom)?)
                }
                "boolValue" => Self::Bool(serde_json::from_value(value).map_err(D::Error::custom)?),
                "intValue" => Self::Int(integer::deserialize(value).map_err(D::Error::custom)?),
                "doubleValue" => Self::Double(match value {
                    serde_json::Value::String(s) => match s.as_str() {
                        "NaN" => f64::NAN,
                        "Infinity" => f64::INFINITY,
                        "-Infinity" => f64::NEG_INFINITY,
                        _ => s
                            .parse::<f64>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .ok_or_else(|| D::Error::custom("invalid double"))?,
                    },
                    v => serde_json::from_value(v).map_err(D::Error::custom)?,
                }),
                "arrayValue" => Self::Array(
                    serde_json::from_value::<Values<Vec<AnyValue>>>(value)
                        .map_err(D::Error::custom)?
                        .values,
                ),
                "kvlistValue" => Self::Map(
                    serde_json::from_value::<Values<Vec<KeyValue>>>(value)
                        .map_err(D::Error::custom)?
                        .values,
                ),
                "bytesValue" => {
                    let text: String = serde_json::from_value(value).map_err(D::Error::custom)?;
                    Self::Bytes(decode_base64(&text).map_err(D::Error::custom)?)
                }
                _ => continue, // OTLP receivers ignore unknown message fields.
            };
            if selected.replace(parsed).is_some() {
                return Err(D::Error::custom("multiple AnyValue variants"));
            }
        }
        Ok(selected.unwrap_or_default())
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(bytes: &[u8]) -> String {
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        text.push(BASE64[(a >> 2) as usize] as char);
        text.push(BASE64[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        text.push(if chunk.len() > 1 {
            BASE64[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        text.push(if chunk.len() > 2 {
            BASE64[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    text
}

fn decode_base64(text: &str) -> Result<Vec<u8>, &'static str> {
    // ProtoJSON accepts standard and URL-safe alphabets, with or without padding.
    let digits = text.trim_end_matches('=');
    let padding = text.len() - digits.len();
    if digits.len() % 4 == 1 || padding > 2 || (padding > 0 && !text.len().is_multiple_of(4)) {
        return Err("invalid base64 length or padding");
    }
    let mut output = Vec::new();
    let mut bits = 0;
    let mut pending = 0u32;
    for byte in digits.bytes() {
        let byte = match byte {
            b'-' => b'+',
            b'_' => b'/',
            b => b,
        };
        let digit = BASE64
            .iter()
            .position(|b| *b == byte)
            .ok_or("invalid base64 digit")? as u32;
        pending = (pending << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((pending >> bits) as u8);
            pending &= (1 << bits) - 1;
        }
    }
    if pending != 0 {
        return Err("nonzero base64 padding bits");
    }
    Ok(output)
}
