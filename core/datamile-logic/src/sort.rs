use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Forward,
    Reverse,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SortKey {
    Number(i64),
    Float(f64),
    String(String),
    Tuple(Vec<Self>),
}

impl Eq for SortKey {}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Float(a), Self::Float(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Self::Number(a), Self::Float(b)) => {
                let a_f = a.to_string().parse::<f64>().unwrap_or(0.0);
                a_f.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Self::Float(a), Self::Number(b)) => {
                let b_f = b.to_string().parse::<f64>().unwrap_or(0.0);
                a.partial_cmp(&b_f).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Self::String(a), Self::String(b)) => alphanumeric_sort::compare_str(a, b),
            (Self::Tuple(a), Self::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    let res = x.cmp(y);
                    if res != std::cmp::Ordering::Equal {
                        return res;
                    }
                }
                a.len().cmp(&b.len())
            }
            (Self::Number(_) | Self::Float(_), _) | (Self::String(_), Self::Tuple(_)) => {
                std::cmp::Ordering::Less
            }
            (_, Self::Number(_) | Self::Float(_)) | (Self::Tuple(_), Self::String(_)) => {
                std::cmp::Ordering::Greater
            }
        }
    }
}

pub fn value_to_sort_key(val: &Value) -> SortKey {
    match val {
        Value::Number(n) => n.as_i64().map_or_else(
            || n.as_f64().map_or(SortKey::Number(0), SortKey::Float),
            SortKey::Number,
        ),
        Value::String(s) => SortKey::String(s.clone()),
        Value::Array(arr) => SortKey::Tuple(arr.iter().map(value_to_sort_key).collect()),
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(k1, _), (k2, _)| {
                let n1 = k1.parse::<usize>().ok();
                let n2 = k2.parse::<usize>().ok();
                match (n1, n2) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => k1.cmp(k2),
                }
            });
            SortKey::Tuple(
                entries
                    .into_iter()
                    .map(|(_, v)| value_to_sort_key(v))
                    .collect(),
            )
        }
        Value::Bool(b) => SortKey::Number(i64::from(*b)),
        Value::Null => SortKey::String(String::new()),
    }
}
