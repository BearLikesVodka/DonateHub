use donatehub_core::assets;
use donatehub_core::style::{default_style, load_presets, merge_style};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn default_style_matches_python() {
    assert_eq!(default_style(), fixture("default_style.json"));
    // Same key order as the Python dict, so the JSON the frontend gets is identical.
    assert_eq!(default_style().to_string(), fixture("default_style.json").to_string());
}

#[test]
fn presets_match_python() {
    assert_eq!(Value::Array(load_presets()), fixture("presets.json"));
}

#[test]
fn test_merge_style_rejects_bad_enum_and_css_injection() {
    let d = default_style();
    let s = merge_style(
        &d,
        &json!({"elements": {"title_pos": "sideways"},
                "fonts": {"title": {"color": "red;} body{display:none", "family": "Roboto"}}}),
    );
    assert_eq!(s["elements"]["title_pos"], d["elements"]["title_pos"]);
    assert_eq!(s["fonts"]["title"]["color"], d["fonts"]["title"]["color"]);
    assert_eq!(s["fonts"]["title"]["family"], "Roboto");
}

#[test]
fn test_merge_style_int_accepted_for_float() {
    let s = merge_style(&default_style(), &json!({"bar": {"height_vw": 5}}));
    assert_eq!(s["bar"]["height_vw"], json!(5));
}

#[test]
fn test_style_merge_rejects_unknown_and_bad_types() {
    let s = merge_style(&default_style(), &json!({"bar": {"height_vw": "x", "nope": 1, "radius_vw": 2}}));
    assert_eq!(s["bar"]["height_vw"], json!(7.0));
    assert_eq!(s["bar"]["radius_vw"], json!(2));
    assert!(s["bar"].get("nope").is_none());
}

#[test]
fn merge_style_rejects_bool_for_number_and_out_of_range() {
    let s = merge_style(&default_style(), &json!({"bar": {"height_vw": true, "radius_vw": 5000}}));
    assert_eq!(s["bar"]["height_vw"], json!(7.0));
    assert_eq!(s["bar"]["radius_vw"], json!(1.2));
}

#[test]
fn test_presets_are_complete_styles() {
    let presets = load_presets();
    assert!(presets.len() >= 5);
    let keys: Vec<_> = default_style().as_object().unwrap().keys().cloned().collect();
    for p in presets {
        assert!(!p["name"].as_str().unwrap().is_empty());
        let pk: Vec<_> = p["style"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(pk, keys);
    }
}

#[test]
fn test_text_align_option() {
    let d = default_style();
    assert_eq!(d["elements"]["text_align"], "left");
    assert_eq!(merge_style(&d, &json!({"elements": {"text_align": "center"}}))["elements"]["text_align"], "center");
    assert_eq!(merge_style(&d, &json!({"elements": {"text_align": "diagonal"}}))["elements"]["text_align"], "left");
}

#[test]
fn assets_served_with_mime() {
    let (body, mime) = assets::get("overlay.html").unwrap();
    assert!(mime.starts_with("text/html"));
    assert!(String::from_utf8_lossy(&body).contains("DonateHub"));
    assert!(assets::get("render.js").unwrap().1.contains("javascript"));
    assert!(assets::get("nope.txt").is_none());
}
