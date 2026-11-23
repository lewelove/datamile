#[must_use]
pub fn calculate_total_discs(tracks: &[serde_json::Value]) -> u32 {
    let mut discs = std::collections::HashSet::new();
    for t in tracks {
        let val = match t.get("discnumber") {
            Some(serde_json::Value::Number(n)) => {
                n.as_u64().and_then(|i| u32::try_from(i).ok()).unwrap_or(0)
            }
            Some(serde_json::Value::String(s)) => s
                .split('/')
                .next()
                .unwrap_or("0")
                .trim()
                .parse::<u32>()
                .unwrap_or(0),
            _ => 0,
        };
        if val > 0 {
            discs.insert(val);
        }
    }
    if discs.is_empty() {
        1
    } else {
        u32::try_from(discs.len()).unwrap_or(u32::MAX)
    }
}

#[must_use]
pub fn format_ms(ms: u64) -> String {
    let s = (ms / 1000) % 60;
    let m = (ms / (1000 * 60)) % 60;
    let h = ms / (1000 * 60 * 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
