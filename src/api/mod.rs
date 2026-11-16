#![allow(dead_code)]
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;

use crate::models::*;
use crate::transport::Transport;

const AUTH_BASE: &str = "https://auth.fduhole.com/api";
const FORUM_BASE: &str = "https://forum.fduhole.com/api";
const CURRICULUM_BASE: &str = "https://danke.fduhole.com/api";

#[derive(Clone)]
pub struct DantaClient {
    http: Transport,
    token: Option<JWToken>,
}

/// Helper: check response status and return body on error.
async fn check(resp: reqwest::Response, action: &str) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        Ok(resp)
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("{} failed ({}): {}", action, status, body);
    }
}

impl DantaClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: Transport::load()?,
            token: None,
        })
    }

    pub fn with_token(token: JWToken) -> Result<Self> {
        Ok(Self {
            http: Transport::load()?,
            token: Some(token),
        })
    }

    pub fn token(&self) -> Option<&JWToken> {
        self.token.as_ref()
    }

    fn access_token(&self) -> Result<&str> {
        self.token
            .as_ref()
            .map(|t| t.access.as_str())
            .context("Not logged in")
    }

    fn auth_header(&self) -> Result<(&str, String)> {
        Ok(("Authorization", format!("Bearer {}", self.access_token()?)))
    }

    pub fn http(&self) -> &Transport {
        &self.http
    }

    pub fn connection_mode(&self) -> &'static str {
        self.http.connection_mode()
    }

    pub fn auth_value(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {}", t.access))
    }

    // ═══════════════════════════════════════
    // Auth
    // ═══════════════════════════════════════

    pub async fn login(&mut self, email: &str, password: &str) -> Result<&JWToken> {
        let resp = self
            .http
            .post(format!("{AUTH_BASE}/login"))
            .json(&LoginRequest {
                email: email.to_string(),
                password: password.to_string(),
            })
            .send()
            .await?;
        let resp = check(resp, "Login").await?;
        let token: JWToken = resp.json().await?;
        self.token = Some(token);
        Ok(self.token.as_ref().unwrap())
    }

    pub async fn refresh_token(&mut self) -> Result<()> {
        let refresh = self
            .token
            .as_ref()
            .map(|t| t.refresh.clone())
            .context("No refresh token")?;

        let resp = self
            .http
            .post(format!("{AUTH_BASE}/refresh"))
            .header("Authorization", format!("Bearer {}", refresh))
            .send()
            .await?;
        let resp = check(resp, "Token refresh").await?;
        let token: JWToken = resp.json().await?;
        self.token = Some(token);
        Ok(())
    }

    /// Try to ensure a valid token: refresh if current access token is expired.
    /// Returns true if token was refreshed (caller should save the new token).
    pub async fn ensure_token(&mut self) -> Result<bool> {
        // Try a lightweight request to see if token works
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/divisions"))
            .header(k, v)
            .send()
            .await?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            check(resp, "Check forum session").await?;
            return Ok(false);
        }
        // Only a forum 401 means the forum token needs refreshing.
        // WebVPN session failures are reported by the transport before this point.
        self.refresh_token().await?;
        Ok(true)
    }

    // ═══════════════════════════════════════
    // Divisions
    // ═══════════════════════════════════════

    pub async fn get_divisions(&self) -> Result<Vec<Division>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/divisions"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get divisions").await?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════
    // Holes
    // ═══════════════════════════════════════

    pub async fn get_holes(
        &self,
        division_id: i64,
        start_time: Option<&str>,
        length: u32,
        order: &str,
    ) -> Result<Vec<Hole>> {
        let (k, v) = self.auth_header()?;
        let mut url = format!(
            "{FORUM_BASE}/holes?division_id={}&length={}&order={}",
            division_id, length, order
        );
        if let Some(st) = start_time {
            url.push_str(&format!("&start_time={}", urlencoding::encode(st)));
        }
        let resp = self.http.get(&url).header(k, v).send().await?;
        let resp = check(resp, "Get holes").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_hole(&self, hole_id: i64) -> Result<Hole> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/holes/{}", hole_id))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get hole").await?;
        Ok(resp.json().await?)
    }

    pub async fn create_hole(
        &self,
        division_id: i64,
        content: &str,
        tag_names: &[String],
    ) -> Result<Hole> {
        let (k, v) = self.auth_header()?;
        let tags: Vec<_> = if tag_names.is_empty() {
            vec![serde_json::json!({"name": "默认"})]
        } else {
            tag_names
                .iter()
                .map(|n| serde_json::json!({"name": n}))
                .collect()
        };
        let body = serde_json::json!({
            "content": content,
            "tags": tags,
        });
        let resp = self
            .http
            .post(format!("{FORUM_BASE}/divisions/{}/holes", division_id))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        let resp = check(resp, "Create hole").await?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════
    // Floors
    // ═══════════════════════════════════════

    pub async fn get_floors(
        &self,
        hole_id: i64,
        offset: u32,
        size: u32,
        order: Option<&str>,
    ) -> Result<Vec<Floor>> {
        let (k, v) = self.auth_header()?;
        let mut url = format!(
            "{FORUM_BASE}/holes/{}/floors?offset={}&size={}",
            hole_id, offset, size
        );
        if let Some(ord) = order {
            url.push_str(&format!("&order={}", ord));
        }
        let resp = self.http.get(&url).header(k, v).send().await?;
        let resp = check(resp, "Get floors").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_floor(&self, floor_id: i64) -> Result<Floor> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/floors/{}", floor_id))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get floor").await?;
        Ok(resp.json().await?)
    }

    pub async fn reply_to_hole(&self, hole_id: i64, content: &str) -> Result<Floor> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "content": content });
        let resp = self
            .http
            .post(format!("{FORUM_BASE}/holes/{}/floors", hole_id))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        let resp = check(resp, "Reply").await?;
        Ok(resp.json().await?)
    }

    pub async fn edit_floor(&self, floor_id: i64, content: &str) -> Result<Floor> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "content": content });
        let resp = self
            .http
            .patch(format!("{FORUM_BASE}/floors/{}/_webvpn", floor_id))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        let resp = check(resp, "Edit floor").await?;
        Ok(resp.json().await?)
    }

    pub async fn delete_floor(&self, floor_id: i64, reason: &str) -> Result<Floor> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "delete_reason": reason });
        let resp = self
            .http
            .delete(format!("{FORUM_BASE}/floors/{}", floor_id))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        let resp = check(resp, "Delete floor").await?;
        Ok(resp.json().await?)
    }

    pub async fn like_floor(&self, floor_id: i64, like_value: i32) -> Result<Floor> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .post(format!(
                "{FORUM_BASE}/floors/{}/like/{}",
                floor_id, like_value
            ))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Like floor").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_floor_history(&self, floor_id: i64) -> Result<Vec<FloorHistory>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/floors/{}/history", floor_id))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get floor history").await?;
        Ok(resp.json().await?)
    }

    pub async fn search_floors(&self, query: &str, offset: u32, size: u32) -> Result<Vec<Floor>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!(
                "{FORUM_BASE}/floors/search?search={}&offset={}&size={}",
                urlencoding::encode(query),
                offset,
                size
            ))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Search").await?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════
    // Favorites
    // ═══════════════════════════════════════

    pub async fn get_favorite_ids(&self) -> Result<Vec<i64>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/user/favorites?plain=true"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get favorites").await?;
        let ids: FavoriteIds = resp.json().await?;
        Ok(ids.data)
    }

    pub async fn add_favorite(&self, hole_id: i64) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "hole_id": hole_id });
        let resp = self
            .http
            .post(format!("{FORUM_BASE}/user/favorites"))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Add favorite").await?;
        Ok(())
    }

    pub async fn remove_favorite(&self, hole_id: i64) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "hole_id": hole_id });
        let resp = self
            .http
            .delete(format!("{FORUM_BASE}/user/favorites"))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Remove favorite").await?;
        Ok(())
    }

    // ═══════════════════════════════════════
    // Subscriptions
    // ═══════════════════════════════════════

    pub async fn get_subscription_ids(&self) -> Result<Vec<i64>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/users/subscriptions?plain=true"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get subscriptions").await?;
        let ids: FavoriteIds = resp.json().await?;
        Ok(ids.data)
    }

    pub async fn add_subscription(&self, hole_id: i64) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "hole_id": hole_id });
        let resp = self
            .http
            .post(format!("{FORUM_BASE}/users/subscriptions"))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Subscribe").await?;
        Ok(())
    }

    pub async fn remove_subscription(&self, hole_id: i64) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "hole_id": hole_id });
        let resp = self
            .http
            .delete(format!("{FORUM_BASE}/users/subscriptions"))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Unsubscribe").await?;
        Ok(())
    }

    // ═══════════════════════════════════════
    // Messages
    // ═══════════════════════════════════════

    pub async fn get_messages(&self, not_read: bool) -> Result<Vec<Message>> {
        let (k, v) = self.auth_header()?;
        let url = if not_read {
            format!("{FORUM_BASE}/messages?not_read=true")
        } else {
            format!("{FORUM_BASE}/messages")
        };
        let resp = self.http.get(&url).header(k, v).send().await?;
        let resp = check(resp, "Get messages").await?;
        Ok(resp.json().await?)
    }

    pub async fn mark_message_read(&self, message_id: i64) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "has_read": true });
        let resp = self
            .http
            .delete(format!("{FORUM_BASE}/messages/{}", message_id))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Mark read").await?;
        Ok(())
    }

    pub async fn clear_messages(&self) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "clear_all": true });
        let resp = self
            .http
            .patch(format!("{FORUM_BASE}/messages/_webvpn"))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Clear messages").await?;
        Ok(())
    }

    // ═══════════════════════════════════════
    // Reports
    // ═══════════════════════════════════════

    pub async fn report_floor(&self, floor_id: i64, reason: &str) -> Result<()> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({ "floor_id": floor_id, "reason": reason });
        let resp = self
            .http
            .post(format!("{FORUM_BASE}/reports"))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        check(resp, "Report").await?;
        Ok(())
    }

    // ═══════════════════════════════════════
    // Tags
    // ═══════════════════════════════════════

    pub async fn get_tags(&self) -> Result<Vec<Tag>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/tags"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get tags").await?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════
    // User
    // ═══════════════════════════════════════

    pub async fn get_me(&self) -> Result<User> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/users/me"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get user").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_my_holes(&self, offset: Option<&str>, size: u32) -> Result<Vec<Hole>> {
        let (k, v) = self.auth_header()?;
        let mut url = format!("{FORUM_BASE}/users/me/holes?size={}&order=time_created", size);
        if let Some(ts) = offset {
            url.push_str(&format!("&offset={}", ts));
        }
        let resp = self.http.get(&url).header(k, v).send().await?;
        let resp = check(resp, "Get my holes").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_my_punishments(&self) -> Result<Vec<Punishment>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{FORUM_BASE}/users/me/punishments"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get punishments").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_favorite_holes(&self, length: u32) -> Result<Vec<Hole>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!(
                "{FORUM_BASE}/user/favorites?length={}&prefetch_length=0",
                length
            ))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get favorite holes").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_subscription_holes(&self, length: u32) -> Result<Vec<Hole>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!(
                "{FORUM_BASE}/users/subscriptions?length={}&prefetch_length=0",
                length
            ))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get subscription holes").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_my_floors(&self, offset: u32, size: u32) -> Result<Vec<Floor>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!(
                "{FORUM_BASE}/users/me/floors?offset={}&size={}&sort=desc",
                offset, size
            ))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get my floors").await?;
        Ok(resp.json().await?)
    }

    // ═══════════════════════════════════════
    // Curriculum (课评 / Danke)
    // ═══════════════════════════════════════

    pub async fn list_course_groups(&self) -> Result<Vec<CourseGroup>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{CURRICULUM_BASE}/courses"))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "List course groups").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_course_group(&self, group_id: i64) -> Result<CourseGroup> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{CURRICULUM_BASE}/group/{}", group_id))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get course group").await?;
        Ok(resp.json().await?)
    }

    pub async fn get_course(&self, course_id: i64) -> Result<Course> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!("{CURRICULUM_BASE}/courses/{}", course_id))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "Get course").await?;
        Ok(resp.json().await?)
    }

    pub async fn list_reviews(&self, course_id: i64) -> Result<Vec<Review>> {
        let (k, v) = self.auth_header()?;
        let resp = self
            .http
            .get(format!(
                "{CURRICULUM_BASE}/courses/{}/reviews",
                course_id
            ))
            .header(k, v)
            .send()
            .await?;
        let resp = check(resp, "List reviews").await?;
        Ok(resp.json().await?)
    }

    pub async fn create_review(
        &self,
        course_id: i64,
        title: &str,
        content: &str,
        rank: &Rank,
    ) -> Result<Review> {
        let (k, v) = self.auth_header()?;
        let body = serde_json::json!({
            "title": title,
            "content": content,
            "rank": {
                "overall": rank.overall,
                "content": rank.content,
                "workload": rank.workload,
                "assessment": rank.assessment,
            }
        });
        let resp = self
            .http
            .post(format!(
                "{CURRICULUM_BASE}/courses/{}/reviews",
                course_id
            ))
            .header(k, v)
            .json(&body)
            .send()
            .await?;
        let resp = check(resp, "Create review").await?;
        Ok(resp.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    async fn mock_client(
        division_status: u16,
    ) -> (DantaClient, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 1024];
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let size = stream.read(&mut buffer).await.unwrap();
                    if size == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..size]);
                }
                let request = String::from_utf8(bytes).unwrap();
                let refreshing = request.starts_with("POST /api/refresh ");
                recorded.lock().unwrap().push(request);
                let (status, body) = if refreshing {
                    (200, r#"{"access":"new-access","refresh":"new-refresh"}"#)
                } else {
                    (division_status, "[]")
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = DantaClient {
            http: Transport::for_test(base).unwrap(),
            token: Some(JWToken {
                access: "old-access".into(),
                refresh: "old-refresh".into(),
            }),
        };
        (client, requests, task)
    }

    #[tokio::test]
    async fn valid_forum_session_does_not_refresh() {
        let (mut client, requests, task) = mock_client(200).await;
        let result = client.ensure_token().await;
        task.abort();

        assert!(!result.unwrap());
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /api/divisions "));
        assert_eq!(client.token().unwrap().access, "old-access");
    }

    #[tokio::test]
    async fn forum_401_refreshes_once_with_refresh_token() {
        let (mut client, requests, task) = mock_client(401).await;
        let result = client.ensure_token().await;
        task.abort();

        assert!(result.unwrap());
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /api/divisions "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer old-access\r\n"));
        assert!(requests[1].starts_with("POST /api/refresh "));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("authorization: bearer old-refresh\r\n"));
        assert_eq!(client.token().unwrap().access, "new-access");
        assert_eq!(client.token().unwrap().refresh, "new-refresh");
    }

    #[tokio::test]
    async fn non_auth_failures_are_reported_without_refreshing() {
        for status in [403, 500, 503] {
            let (mut client, requests, task) = mock_client(status).await;
            let result = client.ensure_token().await;
            task.abort();

            let error = result.unwrap_err().to_string();
            assert!(error.contains("Check forum session failed"), "{error}");
            assert!(error.contains(&status.to_string()), "{error}");
            assert_eq!(requests.lock().unwrap().len(), 1);
            assert_eq!(client.token().unwrap().access, "old-access");
        }
    }
}
