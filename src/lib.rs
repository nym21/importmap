//! Generate import maps with hashed URLs for cache busting.

use std::{collections::BTreeMap, fs, io, path::Path};

use rapidhash::v3::rapidhash_v3;
use serde::Serialize;
use walkdir::WalkDir;

/// File extensions to include in the import map.
const EXTENSIONS: &[&str] = &["js", "mjs", "css"];

/// Hash length in the output (hex chars).
const HASH_LEN: usize = 8;

/// HTML marker name for import map section.
const MARKER: &str = "IMPORTMAP";

/// Import map structure matching the web standard.
#[derive(Debug, Serialize)]
pub struct ImportMap {
    pub imports: BTreeMap<String, String>,
}

impl ImportMap {
    /// Create an empty import map (useful for dev mode).
    pub fn empty() -> Self {
        Self {
            imports: BTreeMap::new(),
        }
    }

    /// Scan a directory and generate an import map.
    pub fn scan(dir: &Path, base_url: &str) -> io::Result<Self> {
        let mut imports = BTreeMap::new();
        let base_url = base_url.trim_end_matches('/');

        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !EXTENSIONS.contains(&ext) {
                continue;
            }

            let relative = path
                .strip_prefix(dir)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

            // Skip JS files at root (e.g. service-worker.js)
            if ext == "js" && relative.parent().is_none_or(|p| p == Path::new("")) {
                continue;
            }

            // Skip development builds
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains(".development.") || name.contains(".dev.") {
                continue;
            }

            let contents = fs::read(path)?;
            let hash = rapidhash_v3(&contents);
            let hash_hex = format!("{:016x}", hash);
            let short_hash = &hash_hex[..HASH_LEN];

            let original_url = format!("{}/{}", base_url, relative.display());

            // Insert hash before extension: foo.js -> foo.a1b2c3d4.js
            let hashed_url = if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let parent = relative
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                if parent.is_empty() {
                    format!("{}/{}.{}.{}", base_url, stem, short_hash, ext)
                } else {
                    format!("{}/{}/{}.{}.{}", base_url, parent, stem, short_hash, ext)
                }
            } else {
                format!("{}.{}", original_url, short_hash)
            };

            imports.insert(original_url, hashed_url);
        }

        Ok(Self { imports })
    }

    /// Update an HTML file in place between `<!-- IMPORTMAP -->` and `<!-- /IMPORTMAP -->` markers.
    /// Returns `Ok(true)` if the file was modified, `Ok(false)` if unchanged.
    pub fn update_html_file(&self, path: &Path) -> io::Result<bool> {
        let html = fs::read_to_string(path)?;
        match self.transform_html(&html) {
            Some(updated) if updated != html => {
                fs::write(path, updated)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Transform HTML content between `<!-- IMPORTMAP -->` and `<!-- /IMPORTMAP -->` markers.
    /// Inserts modulepreload links and the importmap script tag.
    /// If the import map is empty (dev mode), clears the content between markers.
    pub fn transform_html(&self, html: &str) -> Option<String> {
        // Dev mode: clear importmap section
        if self.imports.is_empty() {
            return Self::replace_between_markers(html, "");
        }

        let links: String = self
            .imports
            .values()
            .map(|url| format!(r#"<link rel="modulepreload" href="{url}">"#))
            .collect::<Vec<_>>()
            .join("\n");

        let json = serde_json::to_string_pretty(self).ok()?;
        let script = format!("<script type=\"importmap\">\n{json}\n</script>");

        let content = format!("{script}\n{links}");

        Self::replace_between_markers(html, &content)
    }

    fn replace_between_markers(html: &str, content: &str) -> Option<String> {
        let open = format!("<!-- {MARKER} -->");
        let close = format!("<!-- /{MARKER} -->");

        let start_pos = html.find(&open)?;
        let after_open = start_pos + open.len();
        let end_pos = html[after_open..].find(&close)? + after_open;

        // Detect indentation from the opening marker
        let line_start = html[..start_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent = &html[line_start..start_pos];

        // Indent content
        let indented: String = content
            .lines()
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("{indent}{line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        Some(format!(
            "{}\n{}\n{}{}",
            &html[..after_open],
            indented,
            indent,
            &html[end_pos..]
        ))
    }
}
