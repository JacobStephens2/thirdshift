use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Deserializer, Serializer, de::Error};

pub fn serialize<S: Serializer, T: Display>(value: &T, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(value)
}

pub fn deserialize<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let text = match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => return Err(D::Error::custom("expected decimal integer or string")),
    };
    expand_decimal(&text)
        .map_err(D::Error::custom)?
        .parse()
        .map_err(D::Error::custom)
}

// Expand decimal/exponent notation with decimal digits, never through f64.
// The final target-type parse checks the exact signed/unsigned range.
fn expand_decimal(text: &str) -> Result<String, &'static str> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let exponent_digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (mantissa.contains('.') && fraction.is_empty())
        || exponent_digits.is_empty()
        || !exponent_digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("invalid decimal integer");
    }
    let mut digits = format!("{whole}{fraction}")
        .trim_start_matches('0')
        .to_owned();
    if digits.is_empty() {
        return Ok("0".into());
    }
    let exponent: i64 = exponent
        .parse()
        .map_err(|_| "integer exponent out of range")?;
    let fraction_len = i64::try_from(fraction.len()).map_err(|_| "integer out of range")?;
    let shift = exponent
        .checked_sub(fraction_len)
        .ok_or("integer exponent out of range")?;
    if shift < 0 {
        let remove = usize::try_from(shift.unsigned_abs()).map_err(|_| "fractional integer")?;
        let keep = digits
            .len()
            .checked_sub(remove)
            .ok_or("fractional integer")?;
        if digits.as_bytes()[keep..].iter().any(|b| *b != b'0') {
            return Err("fractional integer");
        }
        digits.truncate(keep);
    } else {
        let append = usize::try_from(shift).map_err(|_| "integer out of range")?;
        // OTLP's largest integer is u64: at most 20 decimal digits.
        if digits.len().saturating_add(append) > 20 {
            return Err("integer out of range");
        }
        digits.extend(std::iter::repeat_n('0', append));
    }
    if negative {
        digits.insert(0, '-');
    }
    Ok(digits)
}
