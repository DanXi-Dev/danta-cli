use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::api::DantaClient;
use crate::auth;

#[derive(Parser)]
#[command(name = "danta", about = "FDU Hole (树洞) CLI/TUI client", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output results as JSON (for agent/programmatic use)
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Login to FDU Hole
    Login {
        #[arg(short, long)]
        email: String,
        #[arg(short, long)]
        password: String,
    },
    /// Show current user info
    Me,
    /// List divisions
    Divisions,
    /// List holes in a division
    Holes {
        /// Division ID (default: 1 = 茶楼)
        #[arg(short, long, default_value = "1")]
        division: i64,
        /// Number of holes to fetch (max 10)
        #[arg(short, long, default_value = "10")]
        limit: u32,
        /// Sort order: time_updated or time_created
        #[arg(short, long, default_value = "time_updated")]
        order: String,
        /// Pagination offset (start_time cursor)
        #[arg(long)]
        offset: Option<String>,
    },
    /// View a specific hole and its floors
    View {
        hole_id: i64,
        /// Number of floors to fetch
        #[arg(short, long, default_value = "30")]
        limit: u32,
        /// Offset (for pagination)
        #[arg(short, long, default_value = "0")]
        offset: u32,
        /// Sort by: id (or omit for default)
        #[arg(long)]
        order: Option<String>,
        /// Reverse order (show latest floors first)
        #[arg(short, long)]
        reverse: bool,
    },
    /// Search floors
    Search {
        query: String,
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },
    /// Create a new hole (post)
    Post {
        /// Content text
        content: String,
        /// Division ID (default: 1 = 茶楼)
        #[arg(short, long, default_value = "1")]
        division: i64,
        /// Tags (comma-separated, e.g. "提问,求助")
        #[arg(short, long, default_value = "")]
        tags: String,
    },
    /// Reply to a hole
    Reply {
        /// Hole ID to reply to
        hole_id: i64,
        /// Reply content
        content: String,
    },
    /// Like a floor (+1) or dislike (-1)
    Like {
        /// Floor ID
        floor_id: i64,
        /// 1 for like, -1 for dislike, 0 to cancel
        #[arg(short, long, default_value = "1")]
        value: i32,
    },
    /// Delete a floor you own
    DeleteFloor {
        floor_id: i64,
        #[arg(short, long, default_value = "")]
        reason: String,
    },
    /// View edit history of a floor
    History {
        floor_id: i64,
    },
    /// Add a hole to favorites
    Fav {
        hole_id: i64,
    },
    /// Remove a hole from favorites
    Unfav {
        hole_id: i64,
    },
    /// List favorite hole IDs
    Favs,
    /// Subscribe to a hole
    Sub {
        hole_id: i64,
    },
    /// Unsubscribe from a hole
    Unsub {
        hole_id: i64,
    },
    /// List subscribed hole IDs
    Subs,
    /// List messages (notifications)
    Messages {
        /// Only show unread
        #[arg(short, long)]
        unread: bool,
    },
    /// Clear all messages
    ClearMessages,
    /// Report a floor
    Report {
        floor_id: i64,
        reason: String,
    },
    /// List all tags
    Tags,
    /// List your own holes
    MyHoles {
        #[arg(short, long, default_value = "10")]
        limit: u32,
    },
    /// List your own floors (replies)
    MyFloors {
        #[arg(short, long, default_value = "10")]
        limit: u32,
    },
    /// Edit a floor you own
    EditFloor {
        floor_id: i64,
        /// New content
        content: String,
    },
    /// View a single floor by ID
    Floor {
        floor_id: i64,
    },
    /// List your punishments
    Punishments,
    /// Open TUI mode
    Tui,
    /// Start a telnet server exposing the TUI
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "2323")]
        port: u16,
        /// Address to bind to
        #[arg(short, long, default_value = "127.0.0.1")]
        bind: String,
    },
}

// Helper struct for JSON output of View command
#[derive(Serialize)]
struct ViewOutput {
    hole: crate::models::Hole,
    floors: Vec<crate::models::Floor>,
}

// Helper for simple status JSON responses
#[derive(Serialize)]
struct StatusOutput {
    status: &'static str,
    message: String,
}

fn json_status(msg: impl Into<String>) {
    let out = StatusOutput {
        status: "ok",
        message: msg.into(),
    };
    println!("{}", serde_json::to_string(&out).unwrap());
}

fn json_out<T: Serialize>(data: &T) {
    println!("{}", serde_json::to_string(data).unwrap());
}

pub async fn run_cli(cmd: Commands, json: bool) -> Result<()> {
    match cmd {
        Commands::Login { email, password } => {
            let mut client = DantaClient::new();
            client.login(&email, &password).await?;
            auth::save_token(client.token().unwrap())?;
            if json {
                json_status("Login successful");
            } else {
                println!("Login successful! Token saved.");
            }
        }
        Commands::Me => {
            let client = get_client().await?;
            let user = client.get_me().await?;
            if json {
                json_out(&user);
            } else {
                println!("User ID:  {}", user.user_id);
                println!("Nickname: {}", user.nickname);
                println!("Admin:    {}", user.is_admin);
                println!("Joined:   {}", user.joined_time);
            }
        }
        Commands::Divisions => {
            let client = get_client().await?;
            let divs = client.get_divisions().await?;
            if json {
                json_out(&divs);
            } else {
                for d in divs {
                    println!("[{}] {} - {}", d.id, d.name, d.description);
                }
            }
        }
        Commands::Holes {
            division,
            limit,
            order,
            offset,
        } => {
            let client = get_client().await?;
            let holes = client
                .get_holes(division, offset.as_deref(), limit, &order)
                .await?;
            if json {
                json_out(&holes);
            } else {
                for h in holes {
                    let tags: Vec<_> = h.tags.iter().map(|t| t.name.as_str()).collect();
                    let preview = h
                        .floors
                        .as_ref()
                        .and_then(|f| f.first_floor.as_ref())
                        .map(|f| truncate_str(&f.content.replace('\n', " "), 40))
                        .unwrap_or_default();
                    println!(
                        "#{} [{}] {} | {}回复 {}浏览",
                        h.id,
                        tags.join(", "),
                        preview,
                        h.reply,
                        h.view,
                    );
                }
            }
        }
        Commands::View {
            hole_id,
            limit,
            offset,
            order,
            reverse,
        } => {
            let client = get_client().await?;
            let hole = client.get_hole(hole_id).await?;
            let mut floors = client
                .get_floors(hole_id, offset, limit, order.as_deref())
                .await?;
            if reverse {
                floors.reverse();
            }
            if json {
                json_out(&ViewOutput { hole, floors });
            } else {
                let tags: Vec<_> = hole.tags.iter().map(|t| t.name.as_str()).collect();
                println!("═══ Hole #{} [{}] ═══", hole.id, tags.join(", "));
                println!(
                    "{}回复 | {}浏览 | {}",
                    hole.reply, hole.view, hole.time_created
                );
                println!();

                for (i, f) in floors.iter().enumerate() {
                    let prefix = if i == 0 { "OP" } else { &format!("#{}", i) };
                    println!(
                        "── {} ({}) [floor:{}] {} ──",
                        prefix, f.anonyname, f.id, f.time_created
                    );
                    println!("{}", f.content);
                    if f.like > 0 || f.dislike > 0 {
                        print!("  +{} -{}", f.like, f.dislike);
                    }
                    if f.is_me {
                        print!("  [mine]");
                    }
                    if f.like > 0 || f.dislike > 0 || f.is_me {
                        println!();
                    }
                    println!();
                }
            }
        }
        Commands::Search { query, limit } => {
            let client = get_client().await?;
            let floors = client.search_floors(&query, 0, limit).await?;
            if json {
                json_out(&floors);
            } else if floors.is_empty() {
                println!("No results found.");
            } else {
                for f in floors {
                    let preview = truncate_str(&f.content.replace('\n', " "), 60);
                    println!(
                        "Hole #{} | {} ({}): {}",
                        f.hole_id, f.anonyname, f.time_created, preview
                    );
                }
            }
        }
        Commands::Post {
            content,
            division,
            tags,
        } => {
            let client = get_client().await?;
            let tag_names: Vec<String> = if tags.is_empty() {
                vec![]
            } else {
                tags.split(',').map(|s| s.trim().to_string()).collect()
            };
            let hole = client.create_hole(division, &content, &tag_names).await?;
            if json {
                json_out(&hole);
            } else {
                println!("Created hole #{}", hole.id);
            }
        }
        Commands::Reply { hole_id, content } => {
            let client = get_client().await?;
            let floor = client.reply_to_hole(hole_id, &content).await?;
            if json {
                json_out(&floor);
            } else {
                println!("Replied as floor #{} ({})", floor.id, floor.anonyname);
            }
        }
        Commands::Like { floor_id, value } => {
            let client = get_client().await?;
            let floor = client.like_floor(floor_id, value).await?;
            if json {
                json_out(&floor);
            } else {
                let action = match value {
                    1 => "Liked",
                    -1 => "Disliked",
                    _ => "Reset",
                };
                println!(
                    "{} floor #{} (now +{} -{})",
                    action, floor.id, floor.like, floor.dislike
                );
            }
        }
        Commands::DeleteFloor { floor_id, reason } => {
            let client = get_client().await?;
            client.delete_floor(floor_id, &reason).await?;
            if json {
                json_status(format!("Deleted floor #{}", floor_id));
            } else {
                println!("Deleted floor #{}", floor_id);
            }
        }
        Commands::History { floor_id } => {
            let client = get_client().await?;
            let history = client.get_floor_history(floor_id).await?;
            if json {
                json_out(&history);
            } else if history.is_empty() {
                println!("No edit history for floor #{}.", floor_id);
            } else {
                for (i, h) in history.iter().enumerate() {
                    println!("── Version {} ({}) ──", i + 1, h.time_updated);
                    println!("{}", h.content);
                    println!();
                }
            }
        }
        Commands::Fav { hole_id } => {
            let client = get_client().await?;
            client.add_favorite(hole_id).await?;
            if json {
                json_status(format!("Added hole #{} to favorites", hole_id));
            } else {
                println!("Added hole #{} to favorites.", hole_id);
            }
        }
        Commands::Unfav { hole_id } => {
            let client = get_client().await?;
            client.remove_favorite(hole_id).await?;
            if json {
                json_status(format!("Removed hole #{} from favorites", hole_id));
            } else {
                println!("Removed hole #{} from favorites.", hole_id);
            }
        }
        Commands::Favs => {
            let client = get_client().await?;
            let ids = client.get_favorite_ids().await?;
            if json {
                json_out(&ids);
            } else if ids.is_empty() {
                println!("No favorites.");
            } else {
                println!("Favorites ({}):", ids.len());
                for id in &ids {
                    println!("  #{}", id);
                }
            }
        }
        Commands::Sub { hole_id } => {
            let client = get_client().await?;
            client.add_subscription(hole_id).await?;
            if json {
                json_status(format!("Subscribed to hole #{}", hole_id));
            } else {
                println!("Subscribed to hole #{}.", hole_id);
            }
        }
        Commands::Unsub { hole_id } => {
            let client = get_client().await?;
            client.remove_subscription(hole_id).await?;
            if json {
                json_status(format!("Unsubscribed from hole #{}", hole_id));
            } else {
                println!("Unsubscribed from hole #{}.", hole_id);
            }
        }
        Commands::Subs => {
            let client = get_client().await?;
            let ids = client.get_subscription_ids().await?;
            if json {
                json_out(&ids);
            } else if ids.is_empty() {
                println!("No subscriptions.");
            } else {
                println!("Subscriptions ({}):", ids.len());
                for id in &ids {
                    println!("  #{}", id);
                }
            }
        }
        Commands::Messages { unread } => {
            let client = get_client().await?;
            let msgs = client.get_messages(unread).await?;
            if json {
                json_out(&msgs);
            } else if msgs.is_empty() {
                println!("No messages.");
            } else {
                for m in msgs {
                    let read_mark = if m.has_read { " " } else { "*" };
                    println!(
                        "{} [{}] {} - {}",
                        read_mark, m.message_id, m.time_created, m.description
                    );
                    if !m.message.is_empty() {
                        println!("  {}", m.message);
                    }
                }
            }
        }
        Commands::ClearMessages => {
            let client = get_client().await?;
            client.clear_messages().await?;
            if json {
                json_status("All messages cleared");
            } else {
                println!("All messages cleared.");
            }
        }
        Commands::Report { floor_id, reason } => {
            let client = get_client().await?;
            client.report_floor(floor_id, &reason).await?;
            if json {
                json_status(format!("Reported floor #{}", floor_id));
            } else {
                println!("Reported floor #{}.", floor_id);
            }
        }
        Commands::Tags => {
            let client = get_client().await?;
            let tags = client.get_tags().await?;
            if json {
                json_out(&tags);
            } else {
                for t in tags {
                    println!("[{}] {} (temperature: {})", t.id, t.name, t.temperature);
                }
            }
        }
        Commands::MyHoles { limit } => {
            let client = get_client().await?;
            let holes = client.get_my_holes(None, limit).await?;
            if json {
                json_out(&holes);
            } else if holes.is_empty() {
                println!("No holes posted.");
            } else {
                for h in holes {
                    let preview = h
                        .floors
                        .as_ref()
                        .and_then(|f| f.first_floor.as_ref())
                        .map(|f| truncate_str(&f.content.replace('\n', " "), 40))
                        .unwrap_or_default();
                    println!("#{} {} | {}回复", h.id, preview, h.reply);
                }
            }
        }
        Commands::MyFloors { limit } => {
            let client = get_client().await?;
            let floors = client.get_my_floors(0, limit).await?;
            if json {
                json_out(&floors);
            } else if floors.is_empty() {
                println!("No floors posted.");
            } else {
                for f in floors {
                    let preview = truncate_str(&f.content.replace('\n', " "), 50);
                    println!("Floor #{} in Hole #{}: {}", f.id, f.hole_id, preview);
                }
            }
        }
        Commands::EditFloor { floor_id, content } => {
            let client = get_client().await?;
            let floor = client.edit_floor(floor_id, &content).await?;
            if json {
                json_out(&floor);
            } else {
                println!("Edited floor #{}", floor.id);
            }
        }
        Commands::Floor { floor_id } => {
            let client = get_client().await?;
            let f = client.get_floor(floor_id).await?;
            if json {
                json_out(&f);
            } else {
                println!(
                    "Floor #{} in Hole #{} ({}) {}",
                    f.id, f.hole_id, f.anonyname, f.time_created
                );
                println!("{}", f.content);
                if f.like > 0 || f.dislike > 0 {
                    println!("  +{} -{}", f.like, f.dislike);
                }
            }
        }
        Commands::Punishments => {
            let client = get_client().await?;
            let puns = client.get_my_punishments().await?;
            if json {
                json_out(&puns);
            } else if puns.is_empty() {
                println!("No punishments.");
            } else {
                for p in puns {
                    println!(
                        "[{}] {} days: {} ({} ~ {})",
                        p.id, p.duration, p.reason, p.start_time, p.end_time
                    );
                }
            }
        }
        Commands::Tui | Commands::Serve { .. } => unreachable!(),
    }
    Ok(())
}

async fn get_client() -> Result<DantaClient> {
    let token = auth::load_token()?;
    let mut client = DantaClient::with_token(token);
    // Auto-refresh expired token
    if let Ok(true) = client.ensure_token().await {
        auth::save_token(client.token().unwrap())?;
    }
    Ok(client)
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}
