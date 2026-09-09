use percent_encoding::percent_decode_str;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;
use url::Url;

pub const EXTENSION_ASSET_SCHEME: &str = "muxy-ext";
pub const MAX_EXTENSION_ASSET_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionAsset {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub cache_control: &'static str,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExtensionAssetError {
    #[error("invalid extension asset URL")]
    InvalidUrl,
    #[error("extension asset URL host does not match '{0}'")]
    HostMismatch(String),
    #[error("extension asset path escapes its extension root")]
    OutsideRoot,
    #[error("extension asset does not exist")]
    NotFound,
    #[error("extension asset exceeds the {MAX_EXTENSION_ASSET_BYTES}-byte limit")]
    TooLarge,
    #[error("extension asset could not be read")]
    ReadFailed,
}

pub fn extension_asset_url(
    extension_id: &str,
    relative_path: &str,
) -> Result<Url, ExtensionAssetError> {
    let base = Url::parse(&format!("{EXTENSION_ASSET_SCHEME}://{extension_id}/"))
        .map_err(|_| ExtensionAssetError::InvalidUrl)?;
    base.join(relative_path.trim_start_matches('/'))
        .map_err(|_| ExtensionAssetError::InvalidUrl)
}

pub fn resolve_extension_asset(
    root: &Path,
    extension_id: &str,
    request_url: &str,
) -> Result<ExtensionAsset, ExtensionAssetError> {
    let url = Url::parse(request_url).map_err(|_| ExtensionAssetError::InvalidUrl)?;
    if url.scheme() != EXTENSION_ASSET_SCHEME {
        return Err(ExtensionAssetError::InvalidUrl);
    }
    if url.host_str() != Some(extension_id) {
        return Err(ExtensionAssetError::HostMismatch(extension_id.to_owned()));
    }
    let decoded = percent_decode_str(raw_request_path(request_url))
        .decode_utf8()
        .map_err(|_| ExtensionAssetError::InvalidUrl)?;
    let relative = decoded.strip_prefix('/').unwrap_or(&decoded);
    if relative.as_bytes().contains(&0) || relative.contains('\\') {
        return Err(ExtensionAssetError::InvalidUrl);
    }
    let base = std::fs::canonicalize(root).map_err(|_| ExtensionAssetError::NotFound)?;
    let candidate = root.join(relative);
    let resolved = std::fs::canonicalize(&candidate).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ExtensionAssetError::NotFound,
        std::io::ErrorKind::PermissionDenied => ExtensionAssetError::OutsideRoot,
        _ => ExtensionAssetError::ReadFailed,
    })?;
    if resolved != base && !resolved.starts_with(&base) {
        return Err(ExtensionAssetError::OutsideRoot);
    }
    let mut file = File::open(&resolved).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ExtensionAssetError::NotFound,
        std::io::ErrorKind::PermissionDenied => ExtensionAssetError::OutsideRoot,
        _ => ExtensionAssetError::ReadFailed,
    })?;
    let metadata = file
        .metadata()
        .map_err(|_| ExtensionAssetError::ReadFailed)?;
    if !metadata.is_file() {
        return Err(ExtensionAssetError::NotFound);
    }
    if metadata.len() > MAX_EXTENSION_ASSET_BYTES {
        return Err(ExtensionAssetError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.by_ref()
        .take(MAX_EXTENSION_ASSET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ExtensionAssetError::ReadFailed)?;
    if bytes.len() as u64 > MAX_EXTENSION_ASSET_BYTES {
        return Err(ExtensionAssetError::TooLarge);
    }
    Ok(ExtensionAsset {
        content_type: extension_asset_content_type(&resolved),
        path: resolved,
        bytes,
        cache_control: "no-store",
    })
}

fn raw_request_path(request_url: &str) -> &str {
    let Some((_, authority_and_path)) = request_url.split_once("://") else {
        return "";
    };
    let Some(path_offset) = authority_and_path.find('/') else {
        return "";
    };
    let path_and_suffix = &authority_and_path[path_offset..];
    let end = path_and_suffix
        .find(['?', '#'])
        .unwrap_or(path_and_suffix.len());
    &path_and_suffix[..end]
}

pub fn extension_asset_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("wasm") => "application/wasm",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

pub fn allows_extension_navigation(request_url: Option<&str>) -> bool {
    let Some(request_url) = request_url else {
        return true;
    };
    Url::parse(request_url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), EXTENSION_ASSET_SCHEME | "about"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolves_percent_decoded_assets_with_swift_response_metadata() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("assets")).unwrap();
        fs::write(
            directory.path().join("assets/main file.js"),
            b"window.ok = true",
        )
        .unwrap();

        let asset = resolve_extension_asset(
            directory.path(),
            "fixture",
            "muxy-ext://fixture/assets/main%20file.js?cache=no",
        )
        .unwrap();

        assert_eq!(asset.bytes, b"window.ok = true");
        assert_eq!(asset.content_type, "application/javascript; charset=utf-8");
        assert_eq!(asset.cache_control, "no-store");
    }

    #[test]
    fn rejects_wrong_schemes_hosts_and_traversal() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("index.html"), "fixture").unwrap();
        let parent_file = directory.path().parent().unwrap().join("outside.html");
        fs::write(&parent_file, "outside").unwrap();

        assert_eq!(
            resolve_extension_asset(directory.path(), "fixture", "https://fixture/index.html"),
            Err(ExtensionAssetError::InvalidUrl)
        );
        assert_eq!(
            resolve_extension_asset(directory.path(), "fixture", "muxy-ext://another/index.html"),
            Err(ExtensionAssetError::HostMismatch("fixture".to_owned()))
        );
        assert_eq!(
            resolve_extension_asset(
                directory.path(),
                "fixture",
                "muxy-ext://fixture/../outside.html"
            ),
            Err(ExtensionAssetError::OutsideRoot)
        );
        let _ = fs::remove_file(parent_file);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();

        assert_eq!(
            resolve_extension_asset(
                root.path(),
                "fixture",
                "muxy-ext://fixture/linked/secret.txt"
            ),
            Err(ExtensionAssetError::OutsideRoot)
        );
    }

    #[test]
    fn rejects_assets_over_the_swift_limit_without_reading_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.bin");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_EXTENSION_ASSET_BYTES + 1).unwrap();

        assert_eq!(
            resolve_extension_asset(directory.path(), "fixture", "muxy-ext://fixture/large.bin"),
            Err(ExtensionAssetError::TooLarge)
        );
    }

    #[test]
    fn mime_and_navigation_rules_match_the_swift_adapter() {
        let cases = [
            ("index.HTML", "text/html; charset=utf-8"),
            ("app.mjs", "application/javascript; charset=utf-8"),
            ("styles.css", "text/css; charset=utf-8"),
            ("data.json", "application/json; charset=utf-8"),
            ("icon.svg", "image/svg+xml"),
            ("image.png", "image/png"),
            ("photo.jpeg", "image/jpeg"),
            ("animation.gif", "image/gif"),
            ("image.webp", "image/webp"),
            ("module.wasm", "application/wasm"),
            ("favicon.ico", "image/x-icon"),
            ("README", "application/octet-stream"),
        ];
        for (path, expected) in cases {
            assert_eq!(extension_asset_content_type(Path::new(path)), expected);
        }
        assert!(allows_extension_navigation(None));
        assert!(allows_extension_navigation(Some("about:blank")));
        assert!(allows_extension_navigation(Some(
            "muxy-ext://fixture/index.html"
        )));
        assert!(!allows_extension_navigation(Some("https://example.com")));
        assert!(!allows_extension_navigation(Some("not a url")));
    }

    #[test]
    fn entry_urls_are_normalized_without_changing_the_extension_host() {
        assert_eq!(
            extension_asset_url("fixture", "/nested/index.html")
                .unwrap()
                .as_str(),
            "muxy-ext://fixture/nested/index.html"
        );
    }
}
