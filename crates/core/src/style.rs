use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::assets::Static;

fn font(size: f64, weight: i64) -> Value {
    json!({"family": "Fira Sans Condensed", "size_vw": size, "weight": weight, "italic": false,
           "color": "#ffffff", "stroke_vw": 0.0, "stroke_color": "#000000", "shadow": true})
}

static DEFAULT_STYLE: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "elements": {
            "title_pos": "top",
            "progress_pos": "inside",
            "progress_format": "amount_target_percent",
            "time_pos": "bottom",
            "show_bounds": false,
            "backdrop": false,
            "backdrop_color": "#00ff00",
            "text_align": "left",
        },
        "bar": {
            "height_vw": 7.0,
            "radius_vw": 1.2,
            "border_vw": 0.0,
            "border_color": "#ffffff",
            "bg": {"mode": "color", "color1": "rgba(43,43,43,0.91)", "color2": "rgba(20,20,20,0.91)", "angle": 0},
            "fill": {"mode": "gradient", "color1": "#009e05", "color2": "#00de07", "angle": 0},
        },
        "fonts": {
            "title": font(4.0, 800),
            "progress": font(4.0, 800),
            "time": font(2.5, 600),
        },
    })
});

fn enum_values(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "title_pos" => &["top", "bottom", "hidden"],
        "progress_pos" => &["inside", "top", "bottom", "hidden"],
        "progress_format" => &["percent", "amount", "amount_percent", "amount_target", "amount_target_percent"],
        "time_pos" => &["top", "bottom", "hidden"],
        "mode" => &["color", "gradient"],
        "text_align" => &["left", "center"],
        _ => return None,
    })
}

// Values end up inside CSS; allow colors and font names but nothing that can break out of a declaration.
static SAFE_STR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w\s#(),.%'\-+]{0,100}$").unwrap());

pub fn default_style() -> Value {
    DEFAULT_STYLE.clone()
}

fn valid(key: &str, default: &Value, value: &Value) -> bool {
    match (default, value) {
        (Value::Bool(_), Value::Bool(_)) => true,
        (Value::Number(_), Value::Number(n)) => n.as_f64().is_some_and(|x| (-1000.0..=1000.0).contains(&x)),
        (Value::String(_), Value::String(s)) => match enum_values(key) {
            Some(allowed) => allowed.contains(&s.as_str()),
            None => SAFE_STR.is_match(s),
        },
        _ => false,
    }
}

/// `base` updated with the valid parts of `patch`; shape and types follow the default style.
pub fn merge_style(base: &Value, patch: &Value) -> Value {
    merge(&DEFAULT_STYLE, base, patch)
}

fn merge(schema: &Value, base: &Value, patch: &Value) -> Value {
    let empty = Map::new();
    let schema = schema.as_object().unwrap_or(&empty);
    let patch = patch.as_object().unwrap_or(&empty);
    let mut out = Map::new();
    for (key, default) in schema {
        let current = base.as_object().and_then(|b| b.get(key)).unwrap_or(default);
        let value = if default.is_object() {
            let current = if current.is_object() { current } else { default };
            merge(default, current, patch.get(key).unwrap_or(&Value::Null))
        } else if let Some(p) = patch.get(key).filter(|p| valid(key, default, p)) {
            p.clone()
        } else if valid(key, default, current) {
            current.clone()
        } else {
            default.clone()
        };
        out.insert(key.clone(), value);
    }
    Value::Object(out)
}

/// `presets/*.json` sorted by file name, each as `{"name", "style"}` with a complete style.
pub fn load_presets() -> Vec<Value> {
    let mut names: Vec<_> =
        Static::iter().filter(|p| p.starts_with("presets/") && p.ends_with(".json")).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|p| {
            let data: Value = serde_json::from_slice(&Static::get(&p)?.data).ok()?;
            let style = merge_style(&DEFAULT_STYLE, data.get("style").unwrap_or(&Value::Null));
            Some(json!({"name": data.get("name")?.clone(), "style": style}))
        })
        .collect()
}
