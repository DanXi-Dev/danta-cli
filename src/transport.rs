//! Shared forum transport. Auto mode probes direct access before using a saved WebVPN session.
use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use reqwest::{header, redirect, Client, Method, RequestBuilder, Response, Url};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::webvpn::WebVpnSession;

const WEBVPN_ORIGIN: &str = "https://webvpn.fudan.edu.cn";
const DIRECT_PROBE_URL: &str = "https://forum.fduhole.com";
const DIRECT_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const LOGIN_HINT: &str = "Run `danta webvpn login -u <UIS_ID>` to renew the WebVPN session.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ConnectionMode {
    #[default]
    Auto,
    Direct,
    Webvpn,
}

impl ConnectionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Direct => "direct",
            Self::Webvpn => "webvpn",
        }
    }
}

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
    gateway: Option<Gateway>,
    use_gateway: Arc<AtomicBool>,
    can_fall_back: bool,
    #[cfg(test)]
    test_base: Option<String>,
}

#[derive(Clone)]
enum Gateway {
    Ready {
        client: Client,
        session: WebVpnSession,
    },
    Unavailable(String),
}

impl Gateway {
    fn saved() -> Option<Self> {
        match WebVpnSession::load().and_then(|session| {
            session
                .map(|session| {
                    let client = session.client()?;
                    Ok(Self::Ready { client, session })
                })
                .transpose()
        }) {
            Ok(gateway) => gateway,
            Err(error) => Some(Self::Unavailable(error.to_string())),
        }
    }

    fn client(&self) -> Result<&Client> {
        match self {
            Self::Ready { client, .. } => Ok(client),
            Self::Unavailable(message) => bail!("{message}"),
        }
    }
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
    pub async fn load(mode: ConnectionMode) -> Result<Self> {
        let gateway = (mode != ConnectionMode::Direct)
            .then(Gateway::saved)
            .flatten();
        Self::select(mode, direct_client()?, gateway, DIRECT_PROBE_URL).await
    }

    async fn select(
        mode: ConnectionMode,
        direct: Client,
        gateway: Option<Gateway>,
        probe_url: &str,
    ) -> Result<Self> {
        let can_fall_back = mode == ConnectionMode::Auto && gateway.is_some();
        let use_gateway = match mode {
            ConnectionMode::Webvpn => {
                gateway
                    .as_ref()
                    .context(format!("No saved WebVPN session. {LOGIN_HINT}"))?
                    .client()?;
                true
            }
            ConnectionMode::Auto if can_fall_back => {
                // Any HTTP response shows the direct route is reachable. The probe
                // carries no forum credentials and never follows a redirect.
                direct
                    .get(probe_url)
                    .timeout(DIRECT_PROBE_TIMEOUT)
                    .send()
                    .await
                    .is_err()
            }
            _ => false,
        };
        Ok(Self {
            direct,
            public_images: public_image_client()?,
            gateway,
            use_gateway: Arc::new(AtomicBool::new(use_gateway)),
            can_fall_back,
            #[cfg(test)]
            test_base: None,
        })
    }

    pub fn connection_mode(&self) -> &'static str {
        if self.use_gateway.load(Ordering::Relaxed) {
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
            use_gateway: Arc::new(AtomicBool::new(false)),
            can_fall_back: false,
            test_base: Some(base),
        })
    }

    fn request(&self, method: Method, raw: &str) -> TransportRequest {
        let mapped = self
            .use_gateway
            .load(Ordering::Relaxed)
            .then(|| proxy_url(raw))
            .flatten();
        let proxied = mapped.is_some();
        let url = mapped.unwrap_or_else(|| raw.to_owned());
        #[cfg(test)]
        let url = match &self.test_base {
            Some(base) if proxied => url.replacen(WEBVPN_ORIGIN, base, 1),
            Some(base) if is_forum_origin(raw) => {
                format!("{}{}", base, resource_path(&Url::parse(raw).unwrap()))
            }
            _ => url,
        };
        let (client, session) = match (proxied, &self.gateway) {
            (true, Some(gateway @ Gateway::Ready { session, .. })) => {
                (gateway.client(), Some(session.clone()))
            }
            (true, Some(gateway)) => (gateway.client(), None),
            _ => (Ok(&self.direct), None),
        };
        TransportRequest {
            inner: client.map(|client| client.request(method, url)),
            proxied,
            session,
            fall_back: (!proxied && self.can_fall_back && is_forum_origin(raw))
                .then(|| Arc::clone(&self.use_gateway)),
        }
    }

    /// External images may follow CDN redirects using a client with no credentials.
    pub fn image(&self, url: &str) -> TransportRequest {
        if is_forum_origin(url) {
            self.get(url)
        } else {
            TransportRequest {
                inner: Ok(self.public_images.get(url)),
                proxied: false,
                session: None,
                fall_back: None,
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
    inner: Result<RequestBuilder>,
    proxied: bool,
    session: Option<WebVpnSession>,
    fall_back: Option<Arc<AtomicBool>>,
}

impl TransportRequest {
    pub fn header(mut self, name: &str, value: impl AsRef<str>) -> Self {
        self.inner = self.inner.map(|inner| inner.header(name, value.as_ref()));
        self
    }

    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.inner = self.inner.map(|inner| inner.json(value));
        self
    }

    pub async fn send(self) -> Result<Response> {
        let response = self.inner?.send().await.map_err(|error| {
            if let Some(next) = self.fall_back.filter(|_| error.is_connect() || error.is_timeout()) {
                // Like Flutter, switch later requests. The current request may have
                // reached the server, so never send it again on a different route.
                next.store(true, Ordering::Relaxed);
                return network_error("Direct forum request failed; subsequent requests in this process will use WebVPN", &error);
            }
            let phase = if self.proxied { "WebVPN forum request failed" } else { "Request failed" };
            network_error(phase, &error)
        })?;
        if self.proxied {
            let response = validate_webvpn_response(response)?;
            if response.headers().contains_key(header::SET_COOKIE)
                && (response.status().is_success()
                    || response.status() == reqwest::StatusCode::UNAUTHORIZED)
            {
                if let Some(session) = self.session {
                    // A disk write failure must not turn an already successful
                    // forum write into an error that could cause duplicate posts.
                    if let Err(error) = session.persist_cookie_updates() {
                        eprintln!("Warning: could not persist updated WebVPN session: {error}");
                    }
                }
            }
            Ok(response)
        } else {
            Ok(response)
        }
    }
}

pub(crate) fn network_error(phase: &str, error: &reqwest::Error) -> anyhow::Error {
    let reason = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not establish a connection (DNS, TLS, or proxy)"
    } else if error.is_body() || error.is_decode() {
        "could not read the response"
    } else {
        "HTTP request failed"
    };
    anyhow::anyhow!("{phase}: {reason}")
}

pub(crate) fn validate_webvpn_response(response: Response) -> Result<Response> {
    if response.status().is_redirection() {
        let requested = response.url();
        let target = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| requested.join(location).ok());
        let is_login = target.as_ref().is_some_and(|url| {
            (url.origin() == requested.origin() && matches!(url.path(), "/login" | "/login/"))
                || (url.scheme() == "https"
                    && url.port().is_none()
                    && matches!(url.host_str(), Some("id.fudan.edu.cn" | "uis.fudan.edu.cn")))
        });
        if is_login {
            bail!(
                "WebVPN redirected the request to authentication (HTTP {}). {LOGIN_HINT}",
                response.status().as_u16()
            );
        }
        // Never print or follow Location. It can contain a university login ticket.
        bail!(
            "Unexpected redirect from WebVPN (HTTP {}); the request was not followed",
            response.status().as_u16()
        );
    }
    if response.status().is_server_error() {
        bail!(
            "WebVPN or the upstream forum returned HTTP {}",
            response.status().as_u16()
        );
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        bail!(
            "WebVPN returned HTML instead of forum data (HTTP {})",
            response.status().as_u16()
        );
    }
    if response.status() == reqwest::StatusCode::UNAUTHORIZED && !content_type.contains("json") {
        bail!("WebVPN returned HTTP 401 without a forum JSON response. {LOGIN_HINT}");
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
            gateway: Some(Gateway::Ready {
                client: gateway,
                session: WebVpnSession::empty(),
            }),
            use_gateway: Arc::new(AtomicBool::new(true)),
            can_fall_back: false,
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
        for (status, content_type, extra, body, message) in [
            (
                "302 Found",
                "text/html",
                "Location: /login?ticket=private-test-value\r\n",
                "",
                "danta webvpn login",
            ),
            (
                "307 Temporary Redirect",
                "text/html",
                "Location: /login\r\n",
                "",
                "danta webvpn login",
            ),
            (
                "302 Found",
                "text/html",
                "Location: /other?ticket=private-test-value\r\n",
                "",
                "Unexpected redirect from WebVPN (HTTP 302)",
            ),
            (
                "200 OK",
                "text/html; charset=utf-8",
                "",
                "<html>UIS login</html>",
                "returned HTML instead of forum data",
            ),
            (
                "401 Unauthorized",
                "text/plain",
                "",
                "login required",
                "danta webvpn login",
            ),
            (
                "500 Internal Server Error",
                "text/html",
                "",
                "<html>error</html>",
                "returned HTTP 500",
            ),
        ] {
            let (base, recorded) = mock_once(status, content_type, extra, body).await;
            let error = gateway_transport(base)
                .post("https://forum.fduhole.com/api/divisions/1/holes")
                .header("Authorization", "Bearer secret")
                .send()
                .await
                .unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
            assert!(!error.to_string().contains("private-test-value"));
            assert!(recorded.await.unwrap().starts_with("POST "));
        }
    }

    #[tokio::test]
    async fn auto_prefers_direct_even_with_an_unusable_saved_session() {
        let (probe, recorded) = mock_once("401 Unauthorized", "application/json", "", "{}").await;
        let transport = Transport::select(
            ConnectionMode::Auto,
            direct_client().unwrap(),
            Some(Gateway::Unavailable("expired session".into())),
            &probe,
        )
        .await
        .unwrap();
        assert_eq!(transport.connection_mode(), "direct");
        let actual = recorded.await.unwrap();
        assert!(actual.starts_with("GET / HTTP/1.1"));
        assert!(!actual.contains("authorization:") && !actual.contains("cookie:"));
    }

    #[tokio::test]
    async fn auto_uses_webvpn_when_the_direct_probe_cannot_connect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let probe = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let (base, recorded) = mock_once("200 OK", "application/json", "", "[]").await;
        let gateway = gateway_transport(base.clone()).gateway;
        let mut transport = Transport::select(
            ConnectionMode::Auto,
            direct_client().unwrap(),
            gateway,
            &probe,
        )
        .await
        .unwrap();
        transport.test_base = Some(base);
        assert_eq!(transport.connection_mode(), "webvpn");
        transport
            .get("https://forum.fduhole.com/api/divisions")
            .send()
            .await
            .unwrap();
        assert!(recorded.await.unwrap().starts_with("GET /https/"));
    }

    #[tokio::test]
    async fn a_direct_write_failure_switches_future_requests_without_replaying_the_write() {
        let (probe, recorded) = mock_once("200 OK", "application/json", "", "[]").await;
        let (base, gateway_received) = mock_once("200 OK", "application/json", "", "[]").await;
        let gateway = gateway_transport(base.clone()).gateway;
        let mut transport = Transport::select(
            ConnectionMode::Auto,
            direct_client().unwrap(),
            gateway,
            &probe,
        )
        .await
        .unwrap();
        recorded.await.unwrap();
        transport.test_base = Some(probe); // The direct listener closed after the probe.
        let error = transport
            .post("https://forum.fduhole.com/api/holes")
            .json(&serde_json::json!({"test": true}))
            .send()
            .await
            .unwrap_err();
        assert!(error.to_string().contains("subsequent requests"));
        assert_eq!(transport.connection_mode(), "webvpn");
        transport.test_base = Some(base);
        transport
            .get("https://forum.fduhole.com/api/divisions")
            .send()
            .await
            .unwrap();
        assert!(gateway_received.await.unwrap().starts_with("GET /https/"));
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
