use std::borrow::Cow;

use rust_embed::Embed;

/// The frontend, compiled into the binary (same files the Python version serves).
#[derive(Embed)]
#[folder = "../../src/donatehub/static/"]
pub struct Static;

pub fn get(path: &str) -> Option<(Cow<'static, [u8]>, String)> {
    let file = Static::get(path)?;
    let mime = file.metadata.mimetype().to_string();
    Some((file.data, mime))
}
