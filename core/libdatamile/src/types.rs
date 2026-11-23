use serde_json::Value;

#[must_use]
pub fn parse_time(val: Option<&Value>) -> String {
    let now = chrono::Utc::now();
    let dt = match val {
        Some(Value::Number(n)) => n.as_i64().map_or(now, |i| {
            if i > 253_402_300_799 {
                chrono::DateTime::from_timestamp_millis(i).unwrap_or(now)
            } else {
                chrono::DateTime::from_timestamp(i, 0).unwrap_or(now)
            }
        }),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            trimmed.parse::<i64>().map_or_else(
                |_| chrono::DateTime::parse_from_rfc3339(trimmed).map_or(now, Into::into),
                |i| {
                    if i > 253_402_300_799 {
                        chrono::DateTime::from_timestamp_millis(i).unwrap_or(now)
                    } else {
                        chrono::DateTime::from_timestamp(i, 0).unwrap_or(now)
                    }
                },
            )
        }
        _ => now,
    };
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
