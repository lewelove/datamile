use serde_json::{Map, Value};

/// Truncates a literal lock tree for `?depth=<n>`.
///
/// `None` leaves the file verbatim, `0` drops the root's `nodes`, and `N`
/// keeps `N` levels of children before dropping their `nodes`.
pub fn apply_depth(value: &mut Value, media_type: &str, depth: Option<usize>) {
    let Some(depth) = depth else {
        return;
    };

    let Some(root) = value.get_mut(media_type).and_then(Value::as_object_mut) else {
        return;
    };

    trim_node(root, depth);
}

fn trim_node(node: &mut Map<String, Value>, depth: usize) {
    if depth == 0 {
        node.remove("nodes");
        return;
    }

    let Some(nodes) = node.get_mut("nodes").and_then(Value::as_object_mut) else {
        return;
    };

    for children in nodes.values_mut() {
        let Some(children) = children.as_array_mut() else {
            continue;
        };

        for child in children {
            if let Some(child) = child.as_object_mut() {
                trim_node(child, depth - 1);
            }
        }
    }
}
