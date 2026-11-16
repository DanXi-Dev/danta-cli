mod api;
mod auth;
mod cli;
pub mod config;
mod models;
#[cfg(unix)]
mod serve;
mod transport;
mod tui;
mod webvpn;

use anyhow::{Context, Result};
use clap::Parser;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let json = cli.json;

    match cli.command {
        Some(Commands::Tui) | None => {
            let token = auth::load_token()?;
            let mut client = api::DantaClient::with_token(token)?;
            // Auto-refresh expired token before entering TUI
            if client
                .ensure_token()
                .await
                .context("Could not connect to the forum")?
            {
                auth::save_token(client.token().unwrap())?;
            }
            tui::run(client).await?;
        }
        #[cfg(unix)]
        Some(Commands::Serve { port, bind }) => {
            serve::run(&bind, port).await?;
        }
        #[cfg(not(unix))]
        Some(Commands::Serve { .. }) => {
            eprintln!("Serve mode is only supported on Unix systems.");
            std::process::exit(1);
        }
        Some(cmd) => {
            if let Err(e) = cli::run_cli(cmd, json).await {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"status": "error", "message": e.to_string()})
                    );
                    std::process::exit(1);
                } else {
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}
