use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Serialize;
use std::io::{self, BufRead, IsTerminal};

use crate::api::DantaClient;
use crate::auth;
use crate::transport::ConnectionMode;
use crate::webvpn::WebVpnSession;

#[derive(Parser)]
#[command(name = "danta", about = "FDU Hole (树洞) CLI/TUI client", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output results as JSON (for agent/programmatic use)
    #[arg(long, global = true)]
    pub json: bool,

    /// Choose how to connect to the forum
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub connection: ConnectionMode,
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
    /// Manage UIS authentication for WebVPN access to the forum
    Webvpn {
        #[command(subcommand)]
        command: WebVpnCommand,
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
        /// Number of holes to fetch
        #[arg(short, long, default_value = "20")]
        limit: u32,
        /// Sort order: time_updated or time_created
        #[arg(short, long, default_value = "time_updated")]
        order: String,
        /// Pagination offset (start_time cursor)
        #[arg(long)]
        offset: Option<String>,
        /// Filter by tag name (client-side)
        #[arg(long)]
        tag: Option<String>,
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
    /// Extract image URLs from all floors of a hole
    Images {
        hole_id: i64,
    },
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
    /// Check authentication status
    Status,
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
    /// List course groups (课评)
    Courses {
        /// Group ID to view (omit to list all groups)
        #[arg(short, long)]
        group: Option<i64>,
        /// Filter courses by name keyword (client-side)
        #[arg(short, long)]
        search: Option<String>,
    },
    /// View a specific course and its reviews
    Course {
        course_id: i64,
    },
    /// List reviews for a course
    Reviews {
        course_id: i64,
    },
}

#[derive(Subcommand)]
pub enum WebVpnCommand {
    /// Log in with UIS and save a WebVPN session for forum access
    Login {
        /// UIS student or staff ID
        #[arg(short, long)]
        username: String,
        /// Read the UIS password from the first line of standard input
        #[arg(long)]
        password_stdin: bool,
    },
    /// Check the saved WebVPN session with a read-only forum request
    Status,
    /// Remove the local WebVPN session and return to direct access
    Logout,
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

pub async fn run_cli(cmd: Commands, json: bool, connection: ConnectionMode) -> Result<()> {
    match cmd {
        Commands::Login { email, password } => {
            let mut client = DantaClient::new(connection).await?;
            client.login(&email, &password).await?;
            auth::save_token(client.token().unwrap())?;
            if json {
                json_status("Login successful");
            } else {
                println!("Login successful! Token saved.");
            }
        }
        Commands::Webvpn { command } => run_webvpn(command, json).await?,
        Commands::Me => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
            tag,
        } => {
            let client = get_client(connection).await?;
            let holes = client
                .get_holes(division, offset.as_deref(), limit, &order)
                .await?;
            let holes: Vec<_> = match &tag {
                Some(filter) => holes
                    .into_iter()
                    .filter(|h| h.tags.iter().any(|t| &t.name == filter))
                    .collect(),
                None => holes,
            };
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
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
            let floor = client.reply_to_hole(hole_id, &content).await?;
            if json {
                json_out(&floor);
            } else {
                println!("Replied as floor #{} ({})", floor.id, floor.anonyname);
            }
        }
        Commands::Like { floor_id, value } => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
            client.delete_floor(floor_id, &reason).await?;
            if json {
                json_status(format!("Deleted floor #{}", floor_id));
            } else {
                println!("Deleted floor #{}", floor_id);
            }
        }
        Commands::History { floor_id } => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
            client.add_favorite(hole_id).await?;
            if json {
                json_status(format!("Added hole #{} to favorites", hole_id));
            } else {
                println!("Added hole #{} to favorites.", hole_id);
            }
        }
        Commands::Unfav { hole_id } => {
            let client = get_client(connection).await?;
            client.remove_favorite(hole_id).await?;
            if json {
                json_status(format!("Removed hole #{} from favorites", hole_id));
            } else {
                println!("Removed hole #{} from favorites.", hole_id);
            }
        }
        Commands::Favs => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
            client.add_subscription(hole_id).await?;
            if json {
                json_status(format!("Subscribed to hole #{}", hole_id));
            } else {
                println!("Subscribed to hole #{}.", hole_id);
            }
        }
        Commands::Unsub { hole_id } => {
            let client = get_client(connection).await?;
            client.remove_subscription(hole_id).await?;
            if json {
                json_status(format!("Unsubscribed from hole #{}", hole_id));
            } else {
                println!("Unsubscribed from hole #{}.", hole_id);
            }
        }
        Commands::Subs => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
            client.clear_messages().await?;
            if json {
                json_status("All messages cleared");
            } else {
                println!("All messages cleared.");
            }
        }
        Commands::Report { floor_id, reason } => {
            let client = get_client(connection).await?;
            client.report_floor(floor_id, &reason).await?;
            if json {
                json_status(format!("Reported floor #{}", floor_id));
            } else {
                println!("Reported floor #{}.", floor_id);
            }
        }
        Commands::Tags => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
            let floor = client.edit_floor(floor_id, &content).await?;
            if json {
                json_out(&floor);
            } else {
                println!("Edited floor #{}", floor.id);
            }
        }
        Commands::Floor { floor_id } => {
            let client = get_client(connection).await?;
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
            let client = get_client(connection).await?;
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
        Commands::Status => {
            let client = get_client(connection).await?;
            let user = client.get_me().await?;
            if json {
                json_out(&serde_json::json!({
                    "status": "ok",
                    "connection": client.connection_mode(),
                    "user_id": user.user_id,
                    "nickname": user.nickname,
                }));
            } else {
                println!("Forum session valid ({})", client.connection_mode());
                println!("User ID:  {}", user.user_id);
                println!("Nickname: {}", user.nickname);
            }
        }
        Commands::Images { hole_id } => {
            let client = get_client(connection).await?;
            let hole = client.get_hole(hole_id).await?;
            let total = hole.reply as u32 + 1;
            let floors = client.get_floors(hole_id, 0, total.max(100), None).await?;

            let md_img = Regex::new(r"!\[[^\]]*\]\((https?://[^)]+)\)").unwrap();
            let bare_url =
                Regex::new(r"https?://\S+\.(?:jpg|jpeg|png|gif|webp)(?:\?\S*)?").unwrap();

            let urls: Vec<String> = floors
                .iter()
                .flat_map(|f| {
                    let mut found: Vec<String> = Vec::new();
                    for cap in md_img.captures_iter(&f.content) {
                        found.push(cap[1].to_string());
                    }
                    for cap in bare_url.captures_iter(&f.content) {
                        let url = cap[0].to_string();
                        if !found.contains(&url) {
                            found.push(url);
                        }
                    }
                    found
                })
                .filter(|url| {
                    let filename = url.rsplit('/').next().unwrap_or("");
                    let filename = filename.split('?').next().unwrap_or(filename);
                    !filename.starts_with("dx_")
                })
                .collect();

            if json {
                json_out(&urls);
            } else {
                for url in &urls {
                    println!("{}", url);
                }
            }
        }
        Commands::Tui | Commands::Serve { .. } => unreachable!(),
        Commands::Courses { group, search } => {
            let client = get_client(connection).await?;
            if let Some(gid) = group {
                let cg = client.get_course_group(gid).await?;
                let courses = apply_search(&cg.courses, &search);
                output_courses(&cg, &courses, json);
            } else {
                let groups = client.list_course_groups().await?;
                if json {
                    json_out(&groups);
                } else {
                    for g in &groups {
                        let count = g.courses.len();
                        println!(
                            "[{}] {} ({}) — {} courses",
                            g.id, g.name, g.code, count
                        );
                    }
                }
            }
        }
        Commands::Course { course_id } => {
            let client = get_client(connection).await?;
            let course = client.get_course(course_id).await?;
            let reviews = client.list_reviews(course_id).await?;
            if json {
                #[derive(Serialize)]
                struct CourseOutput {
                    course: crate::models::Course,
                    reviews: Vec<crate::models::Review>,
                }
                json_out(&CourseOutput { course, reviews });
            } else {
                print_course(&course, &reviews);
            }
        }
        Commands::Reviews { course_id } => {
            let client = get_client(connection).await?;
            let reviews = client.list_reviews(course_id).await?;
            if json {
                json_out(&reviews);
            } else {
                print_reviews(&reviews);
            }
        }
    }
    Ok(())
}

fn apply_search<'a>(courses: &'a [crate::models::Course], keyword: &Option<String>) -> Vec<&'a crate::models::Course> {
    match keyword {
        Some(kw) if !kw.is_empty() => courses
            .iter()
            .filter(|c| {
                c.name.contains(kw.as_str())
                    || c.code.contains(kw.as_str())
                    || c.teachers.contains(kw.as_str())
            })
            .collect(),
        _ => courses.iter().collect(),
    }
}

fn output_courses(
    group: &crate::models::CourseGroup,
    courses: &[&crate::models::Course],
    json: bool,
) {
    if json {
        json_out(&courses);
    } else {
        println!("── {} [{}] ──", group.name, group.code);
        if courses.is_empty() {
            println!("  (no courses found)");
        } else {
            for c in courses {
                let stars = if !c.reviews.is_empty() {
                    let avg: f64 = c.reviews.iter().map(|r| r.rank.overall).sum::<f64>()
                        / c.reviews.len() as f64;
                    format!(" ★{:.1} ({} reviews)", avg, c.reviews.len())
                } else {
                    String::new()
                };
                println!(
                    "  [#{}] {} ({}) | {} credits | {} | {}{}",
                    c.id,
                    c.name,
                    c.code,
                    c.credit,
                    c.teachers,
                    c.semester_name(),
                    stars,
                );
            }
        }
    }
}

fn print_course(course: &crate::models::Course, reviews: &[crate::models::Review]) {
    let avg_rank = if !reviews.is_empty() {
        let n = reviews.len() as f64;
        let overall: f64 = reviews.iter().map(|r| r.rank.overall).sum();
        let content: f64 = reviews.iter().map(|r| r.rank.content).sum();
        let workload: f64 = reviews.iter().map(|r| r.rank.workload).sum();
        let assessment: f64 = reviews.iter().map(|r| r.rank.assessment).sum();
        (overall / n, content / n, workload / n, assessment / n, n as i64)
    } else {
        (0.0, 0.0, 0.0, 0.0, 0)
    };

    println!("═══ {} [{}] ═══", course.name, course.code);
    println!("Teachers:    {}", course.teachers);
    println!("Credits:     {} | Hours: {} | Max students: {}", course.credit, course.week_hour, course.max_student);
    println!("Department:  {} | Campus: {}", course.department, course.campus);
    println!("Semester:    {}", course.semester_name());
    if avg_rank.4 > 0 {
        println!();
        println!("── Average Ratings ({reviews}) ──", reviews = avg_rank.4);
        println!("  Overall:    ★{:.1}/5", avg_rank.0);
        println!("  Content:    ★{:.1}/5", avg_rank.1);
        println!("  Workload:   ★{:.1}/5  (higher = lighter)", avg_rank.2);
        println!("  Assessment: ★{:.1}/5  (higher = fairer)", avg_rank.3);
    }
    if !reviews.is_empty() {
        println!();
        print_reviews(reviews);
    } else {
        println!();
        println!("(no reviews yet)");
    }
}

fn print_reviews(reviews: &[crate::models::Review]) {
    for (i, r) in reviews.iter().enumerate() {
        println!(
            "── Review #{} ({}↑) [{}] ──",
            i + 1, r.vote, r.time_created
        );
        println!("  Title: {}", r.title);
        println!("  Overall:★{:.1}  Content:★{:.1}  Workload:★{:.1}  Assessment:★{:.1}",
            r.rank.overall, r.rank.content, r.rank.workload, r.rank.assessment);
        println!("  {}", r.content.replace('\n', "\n  "));
        println!();
    }
}

async fn get_client(connection: ConnectionMode) -> Result<DantaClient> {
    let token = auth::load_token()?;
    let mut client = DantaClient::with_token(token, connection).await?;
    // Propagate connectivity and WebVPN errors without treating them as expired tokens.
    if client.ensure_token().await? {
        auth::save_token(client.token().unwrap())?;
    }
    Ok(client)
}

async fn run_webvpn(command: WebVpnCommand, json: bool) -> Result<()> {
    match command {
        WebVpnCommand::Login {
            username,
            password_stdin,
        } => {
            let password = read_uis_password(password_stdin)?;
            let session = WebVpnSession::login(&username, &password).await?;
            session.save()?;
            if json {
                json_status("WebVPN login successful. Session saved for forum access.");
            } else {
                println!("WebVPN login successful. Session saved for forum access.");
                println!("Your FDU Hole account is separate; use `danta login` if needed.");
            }
        }
        WebVpnCommand::Status => {
            let session = WebVpnSession::load()?.context(
                "WebVPN is not enabled. Run `danta webvpn login -u <UIS_ID>` to enable it.",
            )?;
            session.check().await?;
            if json {
                json_out(&serde_json::json!({
                    "status": "ok",
                    "connection": "webvpn",
                    "message": "WebVPN session valid; forum gateway reachable",
                }));
            } else {
                println!("WebVPN session valid; forum gateway reachable.");
            }
        }
        WebVpnCommand::Logout => {
            WebVpnSession::clear()?;
            if json {
                json_status("Local WebVPN session removed. Forum requests now use direct access.");
            } else {
                println!("Local WebVPN session removed. Forum requests now use direct access.");
            }
        }
    }
    Ok(())
}

fn read_uis_password(password_stdin: bool) -> Result<String> {
    let password = if password_stdin {
        let mut input = String::new();
        io::stdin()
            .lock()
            .read_line(&mut input)
            .context("Could not read UIS password from standard input")?;
        strip_password_line_ending(input)
    } else {
        if !io::stdin().is_terminal() {
            bail!("For non-interactive login, provide the UIS password with --password-stdin");
        }
        rpassword::prompt_password("UIS password: ").context("Could not read UIS password")?
    };
    if password.is_empty() {
        bail!("UIS password cannot be empty");
    }
    Ok(password)
}

fn strip_password_line_ending(mut input: String) -> String {
    if input.ends_with('\n') {
        input.pop();
        if input.ends_with('\r') {
            input.pop();
        }
    }
    input
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvpn_login_prompts_by_default() {
        let cli = Cli::try_parse_from(["danta", "webvpn", "login", "-u", "12345"]).unwrap();
        assert!(matches!(cli.command,
            Some(Commands::Webvpn { command: WebVpnCommand::Login { username, password_stdin: false } })
            if username == "12345"));
    }

    #[test]
    fn webvpn_login_supports_stdin_and_json() {
        let cli = Cli::try_parse_from([
            "danta",
            "webvpn",
            "login",
            "--username",
            "12345",
            "--password-stdin",
            "--json",
        ])
        .unwrap();
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Some(Commands::Webvpn {
                command: WebVpnCommand::Login {
                    password_stdin: true,
                    ..
                }
            })
        ));
    }

    #[test]
    fn webvpn_login_requires_username_and_rejects_password_argument() {
        assert!(Cli::try_parse_from(["danta", "webvpn", "login"]).is_err());
        assert!(Cli::try_parse_from([
            "danta",
            "webvpn",
            "login",
            "-u",
            "12345",
            "--password",
            "secret",
        ])
        .is_err());
    }

    #[test]
    fn webvpn_status_and_logout_need_no_forum_credentials() {
        for command in ["status", "logout"] {
            assert!(Cli::try_parse_from(["danta", "webvpn", command]).is_ok());
        }
    }

    #[test]
    fn stdin_password_preserves_spaces_and_removes_only_line_ending() {
        assert_eq!(
            strip_password_line_ending(" secret  \n".into()),
            " secret  "
        );
        assert_eq!(strip_password_line_ending("secret\r\n".into()), "secret");
        assert_eq!(strip_password_line_ending("secret".into()), "secret");
    }
}
