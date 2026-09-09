use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let output = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is missing"))?;
    let mut source = String::from("static ASSETS: &[(&str, &[u8])] = &[\n");
    for directory in ["icons", "themes"] {
        let mut names = Vec::new();
        for entry in fs::read_dir(format!("assets/{directory}"))? {
            let entry = entry?;
            if entry.file_type()?.is_file() && !entry.file_name().to_string_lossy().starts_with('.')
            {
                names.push(entry.file_name());
            }
        }
        names.sort();
        for name in names {
            let name = name
                .to_str()
                .ok_or_else(|| io::Error::other("non-UTF-8 asset name"))?;
            let key = format!("{directory}/{name}");
            writeln!(
                source,
                "({key:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {path:?}))),",
                path = format!("/assets/{key}")
            )
            .map_err(io::Error::other)?;
        }
    }
    source.push_str("];\n");
    fs::write(PathBuf::from(output).join("assets.rs"), source)?;
    writeln!(io::stdout(), "cargo:rerun-if-changed=assets")
}
