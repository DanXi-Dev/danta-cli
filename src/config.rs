use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot find config directory"))?
        .join("danta-cli");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("config.toml"))
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BorderStyle {
    Rounded,
    Double,
    Thick,
}

impl BorderStyle {
    pub fn next(self) -> Self {
        match self {
            Self::Rounded => Self::Double,
            Self::Double => Self::Thick,
            Self::Thick => Self::Rounded,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Rounded => Self::Thick,
            Self::Double => Self::Rounded,
            Self::Thick => Self::Double,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rounded => "Rounded",
            Self::Double => "Double",
            Self::Thick => "Thick",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[serde(rename = "time_updated")]
    TimeUpdated,
    #[serde(rename = "time_created")]
    TimeCreated,
}

impl SortOrder {
    pub fn next(self) -> Self {
        match self {
            Self::TimeUpdated => Self::TimeCreated,
            Self::TimeCreated => Self::TimeUpdated,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TimeUpdated => "time_updated",
            Self::TimeCreated => "time_created",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Self::TimeUpdated => "Last Updated",
            Self::TimeCreated => "Newest First",
        }
    }
}

// ── Image Protocol ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageProtocol {
    Auto,
    Kitty,
    Iterm2,
    Sixel,
}

impl Default for ImageProtocol {
    fn default() -> Self { Self::Auto }
}

impl ImageProtocol {
    pub fn all() -> &'static [Self] {
        &[Self::Auto, Self::Kitty, Self::Iterm2, Self::Sixel]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Kitty => "Kitty",
            Self::Iterm2 => "iTerm2",
            Self::Sixel => "Sixel",
        }
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|&s| s == self).unwrap();
        all[(idx + 1) % all.len()]
    }

    pub fn prev(self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|&s| s == self).unwrap();
        all[(idx + all.len() - 1) % all.len()]
    }
}

// ── Thumbnail Mode ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThumbnailMode {
    Auto,
    Off,
    ForceColor,
    ForceGrayscale,
}

impl Default for ThumbnailMode {
    fn default() -> Self { Self::Auto }
}

impl ThumbnailMode {
    pub fn all() -> &'static [Self] {
        &[Self::Auto, Self::Off, Self::ForceColor, Self::ForceGrayscale]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Off => "Off",
            Self::ForceColor => "Force: Color",
            Self::ForceGrayscale => "Force: Grayscale",
        }
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|&s| s == self).unwrap();
        all[(idx + 1) % all.len()]
    }

    pub fn prev(self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|&s| s == self).unwrap();
        all[(idx + all.len() - 1) % all.len()]
    }

    pub fn render_mode(self) -> ThumbnailRenderMode {
        match self {
            Self::Auto => {
                if detect_truecolor_support() {
                    ThumbnailRenderMode::Auto
                } else {
                    ThumbnailRenderMode::Grayscale
                }
            }
            Self::Off => ThumbnailRenderMode::Off,
            Self::ForceColor => ThumbnailRenderMode::ColoredHalf,
            Self::ForceGrayscale => ThumbnailRenderMode::Grayscale,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailRenderMode {
    Auto,
    ColoredHalf,
    Grayscale,
    Off,
}

pub fn detect_truecolor_support() -> bool {
    if let Ok(ct) = env::var("COLORTERM") {
        if ct.contains("truecolor") || ct.contains("24bit") {
            return true;
        }
    }
    if let Ok(term) = env::var("TERM") {
        if term.contains("truecolor") || term.contains("24bit") {
            return true;
        }
        if term.starts_with("xterm-256color") {
            return true;
        }
    }
    false
}

// ── Config ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DantaConfig {
    #[serde(default = "default_border")]
    pub border_style: BorderStyle,
    #[serde(default = "default_true")]
    pub show_help_bar: bool,
    #[serde(default = "default_division")]
    pub default_division: i64,
    #[serde(default = "default_sort")]
    pub sort_order: SortOrder,
    #[serde(default = "default_floors_per_page")]
    pub floors_per_page: u32,
    #[serde(default = "default_search_results")]
    pub search_page_size: u32,
    #[serde(default = "default_true")]
    pub show_ascii_art: bool,
    #[serde(default)]
    pub thumbnail_mode: ThumbnailMode,
    #[serde(default)]
    pub image_protocol: ImageProtocol,
}

fn default_border() -> BorderStyle { BorderStyle::Rounded }
fn default_true() -> bool { true }
fn default_division() -> i64 { 1 }
fn default_sort() -> SortOrder { SortOrder::TimeUpdated }
fn default_floors_per_page() -> u32 { 50 }
fn default_search_results() -> u32 { 20 }

impl Default for DantaConfig {
    fn default() -> Self {
        Self {
            border_style: default_border(),
            show_help_bar: true,
            default_division: 1,
            sort_order: default_sort(),
            floors_per_page: 50,
            search_page_size: 20,
            show_ascii_art: true,
            thumbnail_mode: ThumbnailMode::default(),
            image_protocol: ImageProtocol::default(),
        }
    }
}

impl DantaConfig {
    pub fn load() -> Self {
        let path = match config_path() {
            Ok(p) => p,
            Err(_) => return Self::default(),
        };
        if !path.exists() {
            return Self::default();
        }
        match fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        let raw = toml::to_string_pretty(self)?;
        fs::write(&path, raw)?;
        Ok(())
    }
}
