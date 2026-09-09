use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

#[derive(Debug)]
pub struct Assets;

impl Assets {
    pub fn themes() -> impl Iterator<Item = (&'static str, &'static str)> {
        ASSETS.iter().filter_map(|(path, bytes)| {
            Some((
                theme_name(path.strip_prefix("themes/")?),
                std::str::from_utf8(bytes).ok()?,
            ))
        })
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .iter()
            .find(|(key, _)| *key == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(key, _)| key.starts_with(path))
            .map(|(key, _)| (*key).into())
            .collect())
    }
}

fn theme_name(path: &str) -> &str {
    path.strip_suffix(".conf")
        .or_else(|| path.strip_suffix(".theme"))
        .unwrap_or(path)
}
