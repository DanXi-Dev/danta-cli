use serde::{Deserialize, Serialize};

// ── Auth ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JWToken {
    pub access: String,
    pub refresh: String,
}

// ── Division ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Division {
    pub id: i64,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub pinned: Vec<Hole>,
}

// ── Tag ──

// API returns both "id" and "tag_id" with the same value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tag {
    pub id: i64,
    #[serde(rename = "tag_id")]
    pub _tag_id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub temperature: i64,
    #[serde(default)]
    pub nsfw: bool,
}

// ── Hole ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hole {
    pub id: i64,
    #[serde(default)]
    pub hole_id: Option<i64>,
    pub division_id: i64,
    pub time_created: String,
    pub time_updated: String,
    #[serde(default)]
    pub time_deleted: Option<String>,
    #[serde(default)]
    pub view: i64,
    #[serde(default)]
    pub reply: i64,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub good: bool,
    #[serde(default)]
    pub no_purge: bool,
    #[serde(default)]
    pub frozen: bool,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub floors: Option<Floors>,
    #[serde(default)]
    pub favorite_count: i64,
    #[serde(default)]
    pub subscription_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Floors {
    pub first_floor: Option<Floor>,
    pub last_floor: Option<Floor>,
}

// ── Floor ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Floor {
    pub id: i64,
    #[serde(default)]
    pub floor_id: Option<i64>,
    pub hole_id: i64,
    pub content: String,
    #[serde(default)]
    pub anonyname: String,
    pub time_created: String,
    pub time_updated: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub is_me: bool,
    #[serde(default)]
    pub like: i64,
    #[serde(default)]
    pub dislike: i64,
    #[serde(default)]
    pub liked: bool,
    #[serde(default)]
    pub disliked: bool,
    #[serde(default)]
    pub fold: Vec<String>,
    #[serde(default)]
    pub special_tag: String,
    #[serde(default)]
    pub mention: Vec<Floor>,
    #[serde(default)]
    pub modified: i64,
}

// ── Floor History ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloorHistory {
    pub content: String,
    #[serde(default)]
    pub user_id: i64,
    #[serde(default)]
    pub time_updated: String,
}

// ── Message ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub time_created: String,
    #[serde(default)]
    pub has_read: bool,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

// ── Favorites response ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavoriteIds {
    pub data: Vec<i64>,
}

// ── Punishment ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Punishment {
    pub id: i64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub start_time: String,
    #[serde(default)]
    pub end_time: String,
}

// ── User ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub user_id: i64,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub joined_time: String,
}
