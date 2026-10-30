use ratatui::layout::Rect;
use serde_json::Value;
#[derive(Clone, Debug)]
pub struct Pane {
    pub id: String,
    pub terminal: Option<String>,
    pub description: String,
    pub area: Rect,
}
pub fn panes(root: &Value, area: Rect, focused: Option<&str>, zoomed: Option<&str>) -> Vec<Pane> {
    let mut result = vec![];
    if let Some(id) = zoomed
        && let Some(node) = find(root, id)
    {
        collect(node, area, focused, &mut result);
        return result;
    }
    collect(root, area, focused, &mut result);
    result
}
pub fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|n| find(n, id))
}
pub fn first(node: &Value) -> Option<&str> {
    if node["type"] == "block" {
        node["id"].as_str()
    } else {
        node["children"].as_array()?.iter().find_map(first)
    }
}
fn collect(node: &Value, area: Rect, focused: Option<&str>, result: &mut Vec<Pane>) {
    if area.is_empty() {
        return;
    }
    let Some(children) = node["children"].as_array() else {
        result.push(Pane {
            id: node["id"].as_str().unwrap_or("unknown").into(),
            terminal: node["content"]["terminal"].as_str().map(str::to_owned),
            description: node["content"]["mod"]
                .as_str()
                .unwrap_or("Unsupported pane")
                .into(),
            area,
        });
        return;
    };
    if node["type"] == "stack" {
        if let Some(child) = focused
            .and_then(|id| children.iter().find(|c| find(c, id).is_some()))
            .or_else(|| {
                node["active"]
                    .as_str()
                    .and_then(|id| children.iter().find(|child| find(child, id).is_some()))
            })
            .or_else(|| {
                node["active"]
                    .as_u64()
                    .and_then(|i| children.get(i as usize))
            })
            .or_else(|| children.first())
        {
            collect(child, area, focused, result);
        }
        return;
    }
    let vertical = node["type"] == "column";
    let total = if vertical { area.height } else { area.width };
    let weights: Vec<_> = children
        .iter()
        .map(|c| {
            c["size"]["weight"]
                .as_f64()
                .filter(|w| w.is_finite() && *w > 0.)
                .unwrap_or(1.)
        })
        .collect();
    let sum: f64 = weights.iter().sum();
    let mut consumed = 0;
    let mut cumulative = 0.;
    for (index, child) in children.iter().enumerate() {
        cumulative += weights[index];
        let end = if index + 1 == children.len() {
            total
        } else {
            ((cumulative / sum) * f64::from(total)).round() as u16
        };
        let child_area = if vertical {
            Rect::new(
                area.x,
                area.y + consumed,
                area.width,
                end.saturating_sub(consumed),
            )
        } else {
            Rect::new(
                area.x + consumed,
                area.y,
                end.saturating_sub(consumed),
                area.height,
            )
        };
        collect(child, child_area, focused, result);
        consumed = end;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn weighted_layout_and_private_stack_focus() {
        let root = json!({"type":"row","children":[{"type":"block","id":"a","size":{"weight":1}},{"type":"stack","size":{"weight":2},"children":[{"type":"block","id":"b"},{"type":"block","id":"c"}]}]});
        let p = panes(&root, Rect::new(0, 0, 90, 20), Some("c"), None);
        assert_eq!(p[0].area.width, 30);
        assert_eq!(p[1].id, "c");
        assert_eq!(p[1].area.width, 60);
        assert_eq!(
            panes(&root, Rect::new(0, 0, 90, 20), Some("c"), Some("c"))[0]
                .area
                .width,
            90
        );
    }
}
