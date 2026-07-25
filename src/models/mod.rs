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

// ── Curriculum (课评 / Danke) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseGroup {
    pub id: i64,
    pub name: String,
    pub code: String,
    #[serde(default)]
    pub department: String,
    #[serde(default, rename = "campus_name")]
    pub campus: String,
    #[serde(default, rename = "course_list")]
    pub courses: Vec<Course>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Course {
    pub id: i64,
    #[serde(default)]
    pub credit: f64,
    pub name: String,
    pub code: String,
    #[serde(default, rename = "code_id")]
    pub code_id: String,
    #[serde(default)]
    pub department: String,
    #[serde(default, rename = "campus_name")]
    pub campus: String,
    #[serde(default)]
    pub teachers: String,
    #[serde(default, rename = "max_student")]
    pub max_student: i64,
    #[serde(default, rename = "week_hour")]
    pub week_hour: i64,
    #[serde(default)]
    pub year: i64,
    #[serde(default)]
    pub semester: i64,
    #[serde(default)]
    pub reviews: Vec<Review>,
}

impl Course {
    pub fn semester_name(&self) -> String {
        let term = if self.semester % 2 == 1 { "秋" } else { "春" };
        format!("{}-{} {}", self.year, self.year + 1, term)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: i64,
    #[serde(default, rename = "reviewer_id")]
    pub reviewer_id: i64,
    pub title: String,
    pub content: String,
    #[serde(default, rename = "time_created")]
    pub time_created: String,
    #[serde(default, rename = "time_updated")]
    pub time_updated: String,
    #[serde(default, rename = "is_me")]
    pub is_me: bool,
    #[serde(default)]
    pub remark: i64,
    #[serde(default)]
    pub vote: i64,
    pub rank: Rank,
    #[serde(default)]
    pub extra: CurriculumExtra,
    #[serde(default)]
    pub history: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CurriculumExtra {
    #[serde(default)]
    pub achievements: Vec<Achievement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Achievement {
    pub name: String,
    #[serde(default, rename = "obtainDate")]
    pub obtain_date: String,
    #[serde(default)]
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rank {
    pub overall: f64,
    pub content: f64,
    pub workload: f64,
    pub assessment: f64,
}
