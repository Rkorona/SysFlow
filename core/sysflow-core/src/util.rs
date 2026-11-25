use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse "90s", "5m", "24h", "7d". A bare number means seconds.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::invalid_arg("empty duration"));
    }
    let (num, unit) = match s.chars().last() {
        Some(c) if c.is_ascii_digit() => (s, 's'),
        Some(c) => (&s[..s.len() - c.len_utf8()], c),
        None => return Err(Error::invalid_arg("empty duration")),
    };
    let n: u64 = num
        .parse()
        .map_err(|_| Error::invalid_arg(format!("invalid duration: {s}")))?;
    let mult = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return Err(Error::invalid_arg(format!("invalid duration unit in: {s}"))),
    };
    Ok(Duration::from_secs(n.saturating_mul(mult)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units() {
        assert_eq!(parse_duration("90").unwrap().as_secs(), 90);
        assert_eq!(parse_duration("90s").unwrap().as_secs(), 90);
        assert_eq!(parse_duration("5m").unwrap().as_secs(), 300);
        assert_eq!(parse_duration("24h").unwrap().as_secs(), 86400);
        assert_eq!(parse_duration("7d").unwrap().as_secs(), 604800);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("h").is_err());
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("-5m").is_err());
    }
}
