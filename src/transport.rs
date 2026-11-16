//! Shared forum transport. A saved WebVPN session explicitly selects WebVPN;
//! requests are never replayed or silently switched back to a direct connection.
use anyhow::{bail, Context, Result};
use reqwest::{header, redirect, Client, Method, RequestBuilder, Response, Url};
use serde::Serialize;
use std::time::Duration;

use crate::webvpn::WebVpnSession;

const WEBVPN_ORIGIN: &str = "https://webvpn.fudan.edu.cn";
const LOGIN_HINT: &str = "Run `danta webvpn login -u <UIS_ID>` to renew the WebVPN session.";

// Protocol routing identifiers used by DanXi-Dev/DanXi (3c1f5f0).
// Deliberately limited to forum services; other campus services are out of scope.
const FORUM_ROUTES: &[(&str, &str)] = &[
    (
        "www.fduhole.com",
        "77726476706e69737468656265737421e7e056d221347d5871048ce29b5a2e",
    ),
    (
        "auth.fduhole.com",
        "77726476706e69737468656265737421f1e2559469366c45760785a9d6562c38",
    ),
    (
        "forum.fduhole.com",
        "77726476706e69737468656265737421f6f853892a7e6e546b0086a09d1b203a46",
    ),
    (
        "image.fduhole.com",
        "77726476706e69737468656265737421f9fa409b227e6e546b0086a09d1b203ab8",
    ),
];

fn forum_route(url: &Url) -> Option<&'static str> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    FORUM_ROUTES
        .iter()
        .find_map(|(host, route)| (url.host_str() == Some(*host)).then_some(*route))
}

/// Only exact trusted HTTPS origins may receive a forum bearer token.
pub fn is_forum_origin(raw: &str) -> bool {
    Url::parse(raw)
        .ok()
        .as_ref()
        .and_then(forum_route)
        .is_some()
}

fn resource_path(url: &Url) -> String {
    let mut suffix = url.path().to_owned();
    if let Some(query) = url.query() {
        suffix.push('?');
        suffix.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        suffix.push('#');
        suffix.push_str(fragment);
    }
    suffix
}

pub fn proxy_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    let route = forum_route(&url)?;
    Some(format!(
        "{WEBVPN_ORIGIN}/https/{route}{}",
        resource_path(&url)
    ))
}

#[derive(Clone)]
pub struct Transport {
    direct: Client,
    public_images: Client,
    gateway: Option<Client>,
    #[cfg(test)]
    test_base: Option<String>,
}

fn direct_client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        // A forum bearer must never follow redirects into a gateway login page.
        .redirect(redirect::Policy::none())
        .user_agent(concat!("danta-cli/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn public_image_client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(redirect::Policy::limited(10))
        .build()?)
}

impl Transport {
    pub fn load() -> Result<Self> {
        let session = WebVpnSession::load()?;
        let gateway = session.as_ref().map(WebVpnSession::client).transpose()?;
        Ok(Self {
            direct: direct_client()?,
            public_images: public_image_client()?,
            gateway,
            #[cfg(test)]
            test_base: None,
        })
    }

    pub fn connection_mode(&self) -> &'static str {
        if self.gateway.is_some() {
            "webvpn"
        } else {
            "direct"
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(base: String) -> Result<Self> {
        Ok(Self {
            direct: direct_client()?,
            public_images: public_image_client()?,
            gateway: None,
            test_base: Some(base),
        })
    }

    fn request(&self, method: Method, raw: &str) -> TransportRequest {
        let mapped = self.gateway.as_ref().and_then(|_| proxy_url(raw));
        let proxied = mapped.is_some();
        let client = if proxied {
            self.gateway.as_ref().unwrap()
        } else {
            &self.direct
        };
        let url = mapped.unwrap_or_else(|| raw.to_owned());
        #[cfg(test)]
        let url = match &self.test_base {
            Some(base) if proxied => url.replacen(WEBVPN_ORIGIN, base, 1),
            Some(base) if is_forum_origin(raw) => {
                format!("{}{}", base, resource_path(&Url::parse(raw).unwrap()))
            }
            _ => url,
        };
        TransportRequest {
            inner: client.request(method, url),
            proxied,
        }
    }

    /// External images may follow CDN redirects using a client with no credentials.
    pub fn image(&self, url: &str) -> TransportRequest {
        if is_forum_origin(url) {
            self.get(url)
        } else {
            TransportRequest {
                inner: self.public_images.get(url),
                proxied: false,
            }
        }
    }

    pub fn get(&self, url: impl AsRef<str>) -> TransportRequest {
        self.request(Method::GET, url.as_ref())
    }

    pub fn post(&self, url: impl AsRef<str>) -> TransportRequest {
        self.request(Method::POST, url.as_ref())
    }

    pub fn patch(&self, url: impl AsRef<str>) -> TransportRequest {
        self.request(Method::PATCH, url.as_ref())
    }

    pub fn delete(&self, url: impl AsRef<str>) -> TransportRequest {
        self.request(Method::DELETE, url.as_ref())
    }
}

pub struct TransportRequest {
    inner: RequestBuilder,
    proxied: bool,
}

impl TransportRequest {
    pub fn header(mut self, name: &str, value: impl AsRef<str>) -> Self {
        self.inner = self.inner.header(name, value.as_ref());
        self
    }

    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.inner = self.inner.json(value);
        self
    }

    pub async fn send(self) -> Result<Response> {
        let response = self
            .inner
            .send()
            .await
            .map_err(|e| e.without_url())
            .context(if self.proxied {
                "WebVPN request failed"
            } else {
                "Request failed"
            })?;
        if self.proxied {
            let response = validate_webvpn_response(response).await?;
            Ok(response)
        } else {
            Ok(response)
        }
    }
}

pub async fn validate_webvpn_response(response: Response) -> Result<Response> {
    if response.status().is_redirection() {
        bail!("WebVPN redirected the request to authentication. {LOGIN_HINT}");
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        bail!("WebVPN returned an HTML page instead of forum data. {LOGIN_HINT}");
    }
    if response.status() == reqwest::StatusCode::UNAUTHORIZED && !content_type.contains("json") {
        bail!("WebVPN authentication is no longer valid. {LOGIN_HINT}");
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn maps_only_exact_forum_origins_and_preserves_encoded_resource() {
        let input =
            "https://forum.fduhole.com/api/floors/search?search=%E4%BD%A0%20%26%2F&size=20#floor";
        assert_eq!(proxy_url(input).unwrap(), format!(
            "{WEBVPN_ORIGIN}/https/77726476706e69737468656265737421f6f853892a7e6e546b0086a09d1b203a46/api/floors/search?search=%E4%BD%A0%20%26%2F&size=20#floor"
        ));
        for (host, _) in FORUM_ROUTES {
            assert!(proxy_url(&format!("https://{host}/api")).is_some());
        }
        for url in [
            "https://forum.fduhole.com.evil.test/api",
            "https://forum.fduhole.com@evil.test/api",
            "https://user@forum.fduhole.com/api",
            "https://forum.fduhole.com:8443/api",
            "http://forum.fduhole.com/api",
            "https://danke.fduhole.com/api",
            "https://static.fduhole.com/stickers/example.png",
            "https://example.org/image.png",
            "https://webvpn.fudan.edu.cn/login",
            "not a URL",
        ] {
            assert!(!is_forum_origin(url), "{url}");
            assert!(proxy_url(url).is_none(), "{url}");
        }
    }

    async fn mock_once(
        status: &str,
        content_type: &str,
        extra_headers: &str,
        body: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buf = [0; 2048];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]);
                    let size: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(data).unwrap()
        });
        (base, task)
    }

    fn gateway_transport(base: String) -> Transport {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        jar.add_cookie_str("vpn_session=test; Path=/", &Url::parse(&base).unwrap());
        let gateway = Client::builder()
            .no_proxy()
            .cookie_provider(jar)
            .redirect(redirect::Policy::none())
            .build()
            .unwrap();
        Transport {
            direct: direct_client().unwrap(),
            public_images: public_image_client().unwrap(),
            gateway: Some(gateway),
            test_base: Some(base),
        }
    }

    #[tokio::test]
    async fn proxy_preserves_methods_body_bearer_and_session() {
        for method in [Method::GET, Method::POST, Method::PATCH, Method::DELETE] {
            let (base, recorded) = mock_once("200 OK", "application/json", "", "{}").await;
            let transport = gateway_transport(base);
            let request = transport
                .request(
                    method.clone(),
                    "https://forum.fduhole.com/api/floors/7/_webvpn?q=a%26b",
                )
                .header("Authorization", "Bearer forum-token")
                .json(&serde_json::json!({"content":"hello"}));
            request.send().await.unwrap();
            let actual = recorded.await.unwrap();
            assert!(actual.starts_with(&format!("{} /https/77726476706e69737468656265737421f6f853892a7e6e546b0086a09d1b203a46/api/floors/7/_webvpn?q=a%26b HTTP/1.1", method)));
            assert!(actual.contains("authorization: Bearer forum-token\r\n"));
            assert!(actual.contains("cookie: vpn_session=test\r\n"));
            assert!(actual.ends_with(r#"{"content":"hello"}"#));
        }
    }

    #[tokio::test]
    async fn refuses_login_redirects_and_html_without_replaying_requests() {
        for (status, content_type, extra, body) in [
            ("302 Found", "text/html", "Location: /login\r\n", ""),
            (
                "307 Temporary Redirect",
                "text/html",
                "Location: /login\r\n",
                "",
            ),
            (
                "200 OK",
                "text/html; charset=utf-8",
                "",
                "<html>UIS login</html>",
            ),
            ("401 Unauthorized", "text/plain", "", "login required"),
        ] {
            let (base, recorded) = mock_once(status, content_type, extra, body).await;
            let error = gateway_transport(base)
                .post("https://forum.fduhole.com/api/divisions/1/holes")
                .header("Authorization", "Bearer secret")
                .send()
                .await
                .unwrap_err();
            assert!(error.to_string().contains("danta webvpn login"));
            assert!(recorded.await.unwrap().starts_with("POST "));
        }
    }

    #[tokio::test]
    async fn leaves_forum_401_to_token_refresh_and_external_images_cookie_free() {
        let (base, recorded) = mock_once("401 Unauthorized", "application/json", "", "{}").await;
        let response = gateway_transport(base)
            .get("https://auth.fduhole.com/api/login")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        recorded.await.unwrap();

        let (base, recorded) = mock_once("200 OK", "image/png", "", "image").await;
        let transport = gateway_transport(base.clone());
        transport
            .get(format!("{base}/external.png"))
            .send()
            .await
            .unwrap();
        let actual = recorded.await.unwrap();
        assert!(actual.starts_with("GET /external.png HTTP/1.1"));
        assert!(!actual.contains("cookie:"));
        assert!(!actual.contains("authorization:"));
    }
}
