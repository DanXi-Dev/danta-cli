use regex::Regex;
use std::sync::LazyLock;

static IMAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap());

const STICKER_BASE: &str = "https://static.fduhole.com/stickers/";

/// Resolve a raw image reference from `![](ref)` to a full URL.
fn resolve_url(raw: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("{}{}.webp", STICKER_BASE, raw)
    }
}

/// Check if a raw reference is a sticker shortcode.
pub fn is_sticker(raw: &str) -> bool {
    !raw.starts_with("http://") && !raw.starts_with("https://")
}

/// Extract all image/sticker URLs from markdown content.
pub fn extract_image_urls(content: &str) -> Vec<(String, String)> {
    IMAGE_RE
        .captures_iter(content)
        .map(|cap| {
            let alt = cap.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            let raw = cap[2].to_string();
            let display = if alt.is_empty() {
                if is_sticker(&raw) { raw.clone() } else { String::new() }
            } else {
                alt
            };
            (display, resolve_url(&raw))
        })
        .collect()
}

/// A segment of floor content: either text or an image reference.
pub enum ContentSegment {
    Text(String),
    Image {
        is_sticker: bool,
        url: String,       // resolved URL
        label: String,     // display label (shortcode or alt text)
    },
}

/// Split floor content into text and image segments for inline rendering.
pub fn split_content(content: &str) -> Vec<ContentSegment> {
    let mut segments = Vec::new();
    let mut last_end = 0;

    for cap in IMAGE_RE.captures_iter(content) {
        let m = cap.get(0).unwrap();
        if m.start() > last_end {
            segments.push(ContentSegment::Text(content[last_end..m.start()].to_string()));
        }
        let alt = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let raw = &cap[2];
        let sticker = is_sticker(raw);
        let label = if alt.is_empty() {
            raw.to_string()
        } else {
            alt.to_string()
        };
        segments.push(ContentSegment::Image {
            is_sticker: sticker,
            url: resolve_url(raw),
            label,
        });
        last_end = m.end();
    }

    if last_end < content.len() {
        segments.push(ContentSegment::Text(content[last_end..].to_string()));
    }

    segments
}
