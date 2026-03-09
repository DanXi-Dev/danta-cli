use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use crate::models::JWToken;

fn token_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("Cannot find config directory")?
        .join("danta-cli");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("token.json"))
}

pub fn save_token(token: &JWToken) -> Result<()> {
    let path = token_path()?;
    let json = serde_json::to_string_pretty(token)?;
    fs::write(&path, json)?;
    Ok(())
}

pub fn load_token() -> Result<JWToken> {
    let path = token_path()?;
    let json = fs::read_to_string(&path).context(
        "Not logged in. Run `danta login -e <email> -p <password>` first.",
    )?;
    let token: JWToken = serde_json::from_str(&json)?;
    Ok(token)
}
