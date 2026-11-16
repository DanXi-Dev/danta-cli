//! Fudan WebVPN authentication and a separately persisted gateway session.
//!
//! Protocol references: DanXi-Dev/DanXi at 3c1f5f026fbc7ca50e151865f813cdb381807abe,
//! lib/repository/fdu/neo_login_tool.dart and lib/util/webvpn_proxy.dart.
//! UIS credentials and identity-provider cookies are never written to disk.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::rngs::OsRng;
use reqwest::{header, redirect::Policy, Client, Response, StatusCode, Url};
use reqwest_cookie_store::{CookieStore, CookieStoreMutex};
use rsa::{pkcs8::DecodePublicKey, Pkcs1v15Encrypt, RsaPublicKey};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::transport::{network_error, validate_webvpn_response};

// Desktop default used by the Flutter DanXi UIS client.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.51 Safari/537.36";
const WEBVPN_HOST: &str = "webvpn.fudan.edu.cn";
const LOGIN_AGAIN: &str = "Run `danta webvpn login -u <UIS_ID>` to renew the WebVPN session.";

#[derive(Clone)]
pub struct WebVpnSession {
    jar: Arc<CookieStoreMutex>,
    saved: Option<Arc<Mutex<SessionFile>>>,
}

struct SessionFile {
    path: PathBuf,
    gateway: String,
    last_saved: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct SavedSession {
    version: u32,
    cookies: Vec<Value>,
}

struct Endpoints {
    gateway: Url,
    identity: Url,
    legacy: Url,
    login: Url,
    verify: Url,
    #[cfg(test)]
    address: Option<std::net::SocketAddr>,
}

impl Endpoints {
    fn production() -> Self {
        Self {
            gateway: Url::parse("https://webvpn.fudan.edu.cn/").unwrap(),
            identity: Url::parse("https://id.fudan.edu.cn/").unwrap(),
            legacy: Url::parse("https://uis.fudan.edu.cn/").unwrap(),
            login: Url::parse("https://webvpn.fudan.edu.cn/login?cas_login=true").unwrap(),
            verify: Url::parse(
                &crate::transport::proxy_url("https://forum.fduhole.com/api/divisions")
                    .expect("forum is WebVPN-compatible"),
            )
            .unwrap(),
            #[cfg(test)]
            address: None,
        }
    }

    fn validate(&self, url: &Url) -> Result<()> {
        let secure = url.scheme() == "https";
        #[cfg(test)]
        let secure = secure || (self.address.is_some() && url.scheme() == "http");
        if !secure
            || !url.username().is_empty()
            || url.password().is_some()
            || ![&self.gateway, &self.identity, &self.legacy]
                .iter()
                .any(|origin| origin.origin() == url.origin())
        {
            bail!("Refusing an unexpected authentication redirect");
        }
        Ok(())
    }
}

impl WebVpnSession {
    pub(crate) fn empty() -> Self {
        Self {
            jar: Arc::new(CookieStoreMutex::new(CookieStore::default())),
            saved: None,
        }
    }

    pub async fn login(username: &str, password: &str) -> Result<Self> {
        Self::login_at(username, password, &Endpoints::production()).await
    }

    async fn login_at(username: &str, password: &str, endpoints: &Endpoints) -> Result<Self> {
        if username.trim().is_empty() || password.is_empty() {
            bail!("UIS username and password are required");
        }
        let session = Self::empty();
        let client = session.client_at(endpoints)?;
        let page = get_page(
            &client,
            endpoints,
            endpoints.login.clone(),
            "WebVPN login entry",
        )
        .await?;
        let final_page = if let Some(ticket_url) = ticket_url(&page)? {
            get_page(&client, endpoints, ticket_url, "WebVPN ticket callback").await?
        } else if page.url.origin() == endpoints.identity.origin() {
            modern_login(&client, endpoints, page, username, password).await?
        } else if page.url.origin() == endpoints.legacy.origin() {
            legacy_login(&client, endpoints, page, username, password).await?
        } else {
            page
        };
        check_login_error(&final_page.body)?;
        if final_page.url.origin() != endpoints.gateway.origin()
            || final_page.url.path().starts_with("/login")
        {
            bail!("WebVPN authentication did not reach the gateway; complete any verification in a browser and try again");
        }
        // Verify the same gateway-only cookies that later CLI processes restore.
        let store = session.gateway_store(endpoints.gateway.host_str().unwrap())?;
        let session = Self {
            jar: Arc::new(CookieStoreMutex::new(store)),
            saved: None,
        };
        session.check_at(endpoints).await?;
        Ok(session)
    }

    pub fn client(&self) -> Result<Client> {
        self.client_at(&Endpoints::production())
    }

    fn client_at(&self, endpoints: &Endpoints) -> Result<Client> {
        let builder = Client::builder()
            .cookie_provider(self.jar.clone())
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT);
        #[cfg(test)]
        let builder = if let Some(address) = endpoints.address {
            builder
                .no_proxy()
                .resolve(endpoints.gateway.host_str().unwrap(), address)
                .resolve(endpoints.identity.host_str().unwrap(), address)
                .resolve(endpoints.legacy.host_str().unwrap(), address)
        } else {
            builder
        };
        #[cfg(not(test))]
        let _ = endpoints;
        Ok(builder.build()?)
    }

    pub async fn check(&self) -> Result<()> {
        self.check_at(&Endpoints::production()).await
    }

    async fn check_at(&self, endpoints: &Endpoints) -> Result<()> {
        {
            let store = self
                .jar
                .lock()
                .map_err(|_| anyhow!("WebVPN cookie storage unavailable"))?;
            if store.get_request_values(&endpoints.verify).next().is_none() {
                bail!("No usable WebVPN cookie for the forum gateway. {LOGIN_AGAIN}");
            }
        }
        let response = self
            .client_at(endpoints)?
            .get(endpoints.verify.clone())
            .send()
            .await
            .map_err(|error| network_error("WebVPN session check", &error))?;
        let response = validate_webvpn_response(response)?;
        let status = response.status();
        if !(status.is_success() || status == StatusCode::UNAUTHORIZED) {
            bail!("WebVPN check failed ({status})");
        }
        let updated = response.headers().contains_key(header::SET_COOKIE);
        let value: Value = response.json().await.map_err(|error| {
            if error.is_decode() {
                anyhow!(
                    "WebVPN session check received invalid forum JSON (HTTP {})",
                    status.as_u16()
                )
            } else {
                network_error("WebVPN session check", &error)
            }
        })?;
        // JSON 401 is the forum asking for its separate token: the gateway worked.
        if !value.is_object() && !value.is_array() {
            bail!(
                "WebVPN session check received an unexpected forum response (HTTP {})",
                status.as_u16()
            );
        }
        if updated {
            self.persist_cookie_updates()?;
        }
        Ok(())
    }

    pub fn load() -> Result<Option<Self>> {
        Self::load_from(&session_path()?, WEBVPN_HOST)
    }

    fn load_from(path: &Path, gateway_host: &str) -> Result<Option<Self>> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Unable to read WebVPN session"),
        };
        let saved: SavedSession = serde_json::from_slice(&bytes)
            .context("Invalid WebVPN session file; run `danta webvpn login` again")?;
        if saved.version != 1 {
            bail!("Unsupported WebVPN session file version");
        }
        let store =
            CookieStore::from_cookies(saved.cookies.into_iter().map(serde_json::from_value), false)
                .context("Invalid WebVPN cookies; run `danta webvpn login` again")?;
        let session = Self {
            jar: Arc::new(CookieStoreMutex::new(store)),
            saved: None,
        };
        let filtered = session.gateway_store(gateway_host)?;
        if filtered.iter_unexpired().next().is_none() {
            bail!("Saved WebVPN cookies have expired. {LOGIN_AGAIN}");
        }
        Ok(Some(Self {
            jar: Arc::new(CookieStoreMutex::new(filtered)),
            saved: Some(Arc::new(Mutex::new(SessionFile {
                path: path.to_path_buf(),
                gateway: gateway_host.to_owned(),
                last_saved: bytes,
            }))),
        }))
    }

    fn gateway_store(&self, gateway_host: &str) -> Result<CookieStore> {
        let store = self
            .jar
            .lock()
            .map_err(|_| anyhow!("WebVPN cookie storage unavailable"))?;
        // Exact domain only: never persist .fudan.edu.cn SSO cookies.
        let cookies = store
            .iter_unexpired()
            .filter(|cookie| String::from(&cookie.domain) == gateway_host)
            .cloned()
            .map(Ok::<_, std::convert::Infallible>);
        Ok(CookieStore::from_cookies(cookies, false).unwrap())
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&session_path()?, WEBVPN_HOST)
    }

    fn encoded_cookies(&self, gateway_host: &str) -> Result<Option<Vec<u8>>> {
        let store = self.gateway_store(gateway_host)?;
        let mut cookies = store
            .iter_unexpired()
            .map(|cookie| {
                Ok::<_, serde_json::Error>((
                    cookie.name(),
                    String::from(&cookie.path),
                    serde_json::to_value(cookie)?,
                ))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if cookies.is_empty() {
            return Ok(None);
        }
        cookies.sort_unstable_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        Ok(Some(serde_json::to_vec(&SavedSession {
            version: 1,
            cookies: cookies.into_iter().map(|(_, _, cookie)| cookie).collect(),
        })?))
    }

    fn save_to(&self, path: &Path, gateway_host: &str) -> Result<()> {
        let bytes = self.encoded_cookies(gateway_host)?.with_context(|| {
            format!("WebVPN returned no session cookies to save. {LOGIN_AGAIN}")
        })?;
        let dir = path.parent().context("Invalid WebVPN session path")?;
        fs::create_dir_all(dir).with_context(|| {
            format!(
                "Unable to create WebVPN configuration directory {}",
                dir.display()
            )
        })?;
        let _lock = lock_session_file(path)?;
        write_session_file(path, &bytes)
    }

    pub(crate) fn persist_cookie_updates(&self) -> Result<()> {
        let Some(saved) = &self.saved else {
            return Ok(());
        };
        let mut saved = saved
            .lock()
            .map_err(|_| anyhow!("WebVPN session storage unavailable"))?;
        let _lock = lock_session_file(&saved.path)?;
        match fs::read(&saved.path) {
            Ok(current) if current == saved.last_saved => (),
            Ok(_) => return Ok(()), // A different process updated the file.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()), // Logged out.
            Err(error) => return Err(error).context("Unable to read saved WebVPN session"),
        }
        if let Some(bytes) = self.encoded_cookies(&saved.gateway)? {
            if bytes != saved.last_saved {
                write_session_file(&saved.path, &bytes)?;
                saved.last_saved = bytes;
            }
        }
        Ok(())
    }

    pub fn clear() -> Result<()> {
        let path = session_path()?;
        if !path.try_exists()? {
            return Ok(());
        }
        let _lock = lock_session_file(&path)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("Unable to remove WebVPN session"),
        }
    }
}

fn lock_session_file(path: &Path) -> Result<fs::File> {
    // A separate lock file still protects the path after a rename or logout.
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path.with_extension("lock"))
        .with_context(|| format!("Unable to open WebVPN session lock for {}", path.display()))?;
    file.lock().context("Unable to lock WebVPN session file")?;
    Ok(file)
}

fn write_session_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("Invalid WebVPN session path")?;
    let temp = dir.join(format!(
        ".webvpn-{}-{:016x}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("Unable to save WebVPN session to {}", path.display()))
}

fn session_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Cannot find config directory")?
        .join("danta-cli")
        .join("webvpn.json"))
}

struct Page {
    url: Url,
    body: String,
}

async fn get_page(client: &Client, endpoints: &Endpoints, url: Url, step: &str) -> Result<Page> {
    endpoints.validate(&url)?;
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| auth_network_error(step, &url, &error))?;
    follow_redirects(client, endpoints, response, url, false, step).await
}

fn auth_network_error(step: &str, url: &Url, error: &reqwest::Error) -> anyhow::Error {
    // Ticket and auth URLs can contain secrets. Only report the verified host.
    network_error(
        &format!("{step} ({})", url.host_str().unwrap_or("Fudan")),
        error,
    )
}

async fn follow_redirects(
    client: &Client,
    endpoints: &Endpoints,
    mut response: Response,
    mut url: Url,
    mut was_post: bool,
    step: &str,
) -> Result<Page> {
    for _ in 0..12 {
        if !response.status().is_redirection() {
            if !response.status().is_success() {
                bail!(
                    "{step} ({}) returned HTTP {}",
                    url.host_str().unwrap_or("Fudan"),
                    response.status().as_u16()
                );
            }
            let body = response
                .text()
                .await
                .map_err(|error| auth_network_error(step, &url, &error))?;
            return Ok(Page { url, body });
        }
        // Never replay a credential-bearing POST body, even for 307/308.
        if was_post
            && matches!(
                response.status(),
                StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
            )
        {
            bail!("Authentication requested an unsupported POST redirect");
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .context("Authentication redirect has no destination")?
            .to_str()?;
        url = url
            .join(location)
            .context("Invalid authentication redirect")?;
        endpoints.validate(&url)?;
        response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|error| auth_network_error(step, &url, &error))?;
        was_post = false;
    }
    bail!("Too many Fudan authentication redirects")
}

async fn post_json(
    client: &Client,
    endpoints: &Endpoints,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = endpoints.identity.join(path)?;
    endpoints.validate(&url)?;
    let request = client.post(url.clone());
    let request = if let Some(body) = body {
        request.json(body)
    } else {
        request
    };
    let response = request
        .send()
        .await
        .map_err(|error| auth_network_error(&format!("UIS {path}"), &url, &error))?;
    if !response.status().is_success() {
        bail!("UIS {path} returned HTTP {}", response.status().as_u16());
    }
    response
        .json()
        .await
        .context("Fudan identity service returned an unexpected response")
}

async fn modern_login(
    client: &Client,
    endpoints: &Endpoints,
    page: Page,
    username: &str,
    password: &str,
) -> Result<Page> {
    let fragment = page
        .url
        .fragment()
        .context("Fudan login page is missing authentication parameters")?;
    let params_url = Url::parse(&format!(
        "https://id.fudan.edu.cn/{}",
        fragment.trim_start_matches('/')
    ))?;
    let params = params_url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let lck = params
        .get("lck")
        .context("Fudan login page is missing lck")?;
    let entity_id = params
        .get("entityId")
        .context("Fudan login page is missing entityId")?;
    let methods = post_json(
        client,
        endpoints,
        "/idp/authn/queryAuthMethods",
        Some(&json!({"lck": lck, "entityId": entity_id})),
    )
    .await?;
    if methods["second"].as_bool() == Some(true) {
        bail!("Fudan requires additional verification; automatic WebVPN login cannot complete this challenge. Use the university WebVPN in a browser");
    }
    let chain = methods["data"]
        .as_array()
        .and_then(|methods| {
            methods
                .iter()
                .find(|method| method["moduleCode"] == "userAndPwd")
        })
        .and_then(|method| method["authChainCode"].as_str())
        .context(
            "Fudan did not offer password authentication; complete verification in a browser",
        )?;
    let key = post_json(client, endpoints, "/idp/authn/getJsPublicKey", None).await?;
    let key = key["data"]
        .as_str()
        .context("Missing Fudan authentication public key")?;
    let key = RsaPublicKey::from_public_key_der(
        &STANDARD.decode(key).context("Invalid Fudan public key")?,
    )
    .context("Invalid Fudan RSA public key")?;
    let encrypted = STANDARD.encode(
        key.encrypt(&mut OsRng, Pkcs1v15Encrypt, password.as_bytes())
            .context("Unable to encode UIS password")?,
    );
    let response = post_json(
        client,
        endpoints,
        "/idp/authn/authExecute",
        Some(&json!({
            "authModuleCode": "userAndPwd", "authChainCode": chain, "entityId": entity_id,
            "requestType": "chain_type", "lck": lck,
            "authPara": {"loginName": username, "password": encrypted, "verifyCode": ""}
        })),
    )
    .await?;
    if let Some(message) = response["message"].as_str() {
        check_login_error(message)?;
    }
    if response["second"].as_bool() == Some(true) {
        bail!("Fudan requires additional verification; automatic WebVPN login cannot complete this challenge. Use the university WebVPN in a browser");
    }
    let login_token = response["loginToken"].as_str().filter(|token| !token.is_empty())
        .context("Fudan requires additional verification; automatic WebVPN login cannot complete this challenge. Use the university WebVPN in a browser")?;
    let url = endpoints.identity.join("/idp/authCenter/authnEngine")?;
    let step = "UIS login token exchange";
    let response = client
        .post(url.clone())
        .form(&[("loginToken", login_token)])
        .send()
        .await
        .map_err(|error| auth_network_error(step, &url, &error))?;
    let page = follow_redirects(client, endpoints, response, url, true, step).await?;
    if page.url.origin() == endpoints.gateway.origin() && !page.url.path().starts_with("/login") {
        return Ok(page);
    }
    let url = ticket_url(&page)?.context("Fudan did not return a service ticket")?;
    get_page(client, endpoints, url, "WebVPN ticket callback").await
}

async fn legacy_login(
    client: &Client,
    endpoints: &Endpoints,
    page: Page,
    username: &str,
    password: &str,
) -> Result<Page> {
    let fields = {
        let document = Html::parse_document(&page.body);
        let selector = Selector::parse("input[name]").unwrap();
        let mut fields = document
            .select(&selector)
            .filter(|input| !matches!(input.value().attr("type"), Some("button" | "submit")))
            .map(|input| {
                (
                    input.value().attr("name").unwrap().to_owned(),
                    input.value().attr("value").unwrap_or("").to_owned(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        if !fields.contains_key("password") || !fields.contains_key("username") {
            bail!("Unsupported UIS login page; complete verification in a browser");
        }
        fields.insert("username".into(), username.into());
        fields.insert("password".into(), password.into());
        fields
    };
    endpoints.validate(&page.url)?;
    let step = "UIS password authentication";
    let response = client
        .post(page.url.clone())
        .form(&fields)
        .send()
        .await
        .map_err(|error| auth_network_error(step, &page.url, &error))?;
    let page = follow_redirects(client, endpoints, response, page.url, true, step).await?;
    check_login_error(&page.body)?;
    if let Some(url) = ticket_url(&page)? {
        get_page(client, endpoints, url, "WebVPN ticket callback").await
    } else {
        Ok(page)
    }
}

fn ticket_url(page: &Page) -> Result<Option<Url>> {
    let document = Html::parse_document(&page.body);
    let form = Selector::parse("form#logon").unwrap();
    let ticket = Selector::parse("input#ticket").unwrap();
    let Some(form) = document.select(&form).next() else {
        return Ok(None);
    };
    let action = form
        .value()
        .attr("action")
        .context("Missing service-ticket destination")?;
    let ticket = form
        .select(&ticket)
        .next()
        .and_then(|input| input.value().attr("value"))
        .filter(|ticket| !ticket.is_empty())
        .context("Missing service ticket")?;
    let mut url = page.url.join(action)?;
    let query = url
        .query_pairs()
        .filter(|(key, _)| key != "ticket")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(query)
        .append_pair("ticket", ticket);
    Ok(Some(url))
}

fn check_login_error(body: &str) -> Result<()> {
    if body.contains("用户名或密码错误") || body.contains("密码有误") {
        bail!("UIS username or password is incorrect");
    }
    if body.contains("请输入验证码") {
        bail!("Fudan requires a CAPTCHA; automatic WebVPN login cannot complete this challenge. Use the university WebVPN in a browser");
    }
    if body.contains("弱密码提示") {
        bail!("UIS requires a password update; complete it on the university login page");
    }
    if body.contains("网络维护中 | Under Maintenance") {
        bail!("Fudan authentication is under maintenance");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::{pkcs8::EncodePublicKey, RsaPrivateKey};
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener},
        thread,
    };

    struct Request {
        method: String,
        path: String,
        headers: String,
        body: String,
    }
    type Reply = (u16, Vec<(String, String)>, String);

    fn mock_server(
        count: usize,
        handler: impl Fn(usize, Request, SocketAddr) -> Reply + Send + 'static,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            for index in 0..count {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "mock request {index} timed out"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 1024];
                    let len = stream.read(&mut buffer).unwrap();
                    assert!(len > 0);
                    bytes.extend_from_slice(&buffer[..len]);
                    if let Some(position) =
                        bytes.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        break position + 4;
                    }
                };
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 1024];
                    let len = stream.read(&mut buffer).unwrap();
                    assert!(len > 0);
                    bytes.extend_from_slice(&buffer[..len]);
                }
                let first = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .collect::<Vec<_>>();
                let request = Request {
                    method: first[0].into(),
                    path: first[1].into(),
                    body: String::from_utf8(bytes[header_end..header_end + length].to_vec())
                        .unwrap(),
                    headers,
                };
                let (status, headers, body) = handler(index, request, address);
                let headers = headers
                    .into_iter()
                    .map(|(key, value)| format!("{key}: {value}\r\n"))
                    .collect::<String>();
                write!(stream, "HTTP/1.1 {status} Mock\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len()).unwrap();
            }
        });
        (address, handle)
    }

    fn endpoints(address: SocketAddr) -> Endpoints {
        let gateway = Url::parse(&format!("http://webvpn.test:{}/", address.port())).unwrap();
        Endpoints {
            identity: Url::parse(&format!("http://id.test:{}/", address.port())).unwrap(),
            legacy: Url::parse(&format!("http://uis.test:{}/", address.port())).unwrap(),
            login: gateway.join("login?cas_login=true").unwrap(),
            verify: gateway.join("proxy/forum/api/divisions").unwrap(),
            gateway,
            address: Some(address),
        }
    }

    fn reply(status: u16, headers: &[(&str, &str)], body: &str) -> Reply {
        (
            status,
            headers
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
            body.into(),
        )
    }

    async fn mocked_modern_login(require_second_factor: bool) -> Result<WebVpnSession> {
        let private = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let key = STANDARD.encode(
            private
                .to_public_key()
                .to_public_key_der()
                .unwrap()
                .as_bytes(),
        );
        let request_count = if require_second_factor { 5 } else { 9 };
        let (address, server) = mock_server(request_count, move |index, request, address| {
            match index {
                0 => {
                    assert_eq!(request.path, "/login?cas_login=true");
                    reply(
                        302,
                        &[
                            (
                                "Location",
                                &format!(
                                    "http://id.test:{}/idp/login#/login?lck=L&entityId=E",
                                    address.port()
                                ),
                            ),
                            ("Set-Cookie", "gateway_bootstrap=1; Path=/"),
                        ],
                        "",
                    )
                }
                1 => {
                    assert_eq!(request.path, "/idp/login");
                    reply(
                        200,
                        &[("Set-Cookie", "CASTGC=identity-session; Path=/")],
                        "<html>Login</html>",
                    )
                }
                2 => {
                    assert_eq!(request.method, "POST");
                    assert_eq!(request.path, "/idp/authn/queryAuthMethods");
                    assert_eq!(
                        serde_json::from_str::<Value>(&request.body).unwrap(),
                        json!({"lck":"L", "entityId":"E"})
                    );
                    assert!(request.headers.contains("CASTGC=identity-session"));
                    reply(
                        200,
                        &[("Content-Type", "application/json")],
                        r#"{"data":[{"moduleCode":"userAndPwd","authChainCode":"C"}]}"#,
                    )
                }
                3 => {
                    assert_eq!(request.path, "/idp/authn/getJsPublicKey");
                    reply(
                        200,
                        &[("Content-Type", "application/json")],
                        &json!({"data":key}).to_string(),
                    )
                }
                4 => {
                    assert_eq!(request.path, "/idp/authn/authExecute");
                    let body: Value = serde_json::from_str(&request.body).unwrap();
                    assert_eq!(body["authChainCode"], "C");
                    assert_eq!(body["authPara"]["loginName"], "student");
                    assert_eq!(body["authPara"]["verifyCode"], "");
                    let encrypted = STANDARD
                        .decode(body["authPara"]["password"].as_str().unwrap())
                        .unwrap();
                    assert_eq!(
                        private.decrypt(Pkcs1v15Encrypt, &encrypted).unwrap(),
                        b"test-password"
                    );
                    assert!(!request.body.contains("test-password"));
                    reply(
                        200,
                        &[("Content-Type", "application/json")],
                        &json!({"loginToken": "identity-token", "second": require_second_factor})
                            .to_string(),
                    )
                }
                5 => {
                    assert_eq!(request.path, "/idp/authCenter/authnEngine");
                    assert_eq!(request.body, "loginToken=identity-token");
                    reply(
                        200,
                        &[],
                        &format!(
                            r#"<form id="logon" action="http://webvpn.test:{}/callback?keep=1"><input id="ticket" value="T&amp;1"></form>"#,
                            address.port()
                        ),
                    )
                }
                6 => {
                    assert_eq!(request.method, "GET");
                    assert_eq!(request.path, "/callback?keep=1&ticket=T%261");
                    assert!(!request.headers.contains("CASTGC"));
                    reply(
                        302,
                        &[
                            ("Location", "/portal"),
                            (
                                "Set-Cookie",
                                "wrdvpn_ticket=authenticated; Path=/; HttpOnly",
                            ),
                        ],
                        "",
                    )
                }
                7 => reply(200, &[], "<html>WebVPN</html>"),
                8 => {
                    assert_eq!(request.path, "/proxy/forum/api/divisions");
                    assert!(request.headers.contains("wrdvpn_ticket=authenticated"));
                    reply(
                        401,
                        &[("Content-Type", "application/json")],
                        r#"{"message":"forum token required"}"#,
                    )
                }
                _ => unreachable!(),
            }
        });
        let result = WebVpnSession::login_at("student", "test-password", &endpoints(address)).await;
        server.join().unwrap();
        result
    }

    #[tokio::test]
    async fn modern_login_exchanges_rsa_password_ticket_and_gateway_cookie() {
        let session = mocked_modern_login(false).await.unwrap();
        let store = session.jar.lock().unwrap();
        assert!(store
            .iter_any()
            .all(|cookie| String::from(&cookie.domain) == "webvpn.test"));
        assert!(store.iter_any().all(|cookie| cookie.name() != "CASTGC"));
    }

    #[tokio::test]
    async fn second_factor_stops_before_token_exchange_even_when_token_is_present() {
        let error = mocked_modern_login(true).await.err().unwrap().to_string();
        assert!(error.contains("additional verification"));
        assert!(!error.contains("identity-token"));
    }

    #[tokio::test]
    async fn legacy_login_preserves_hidden_fields_and_cookie_redirects() {
        let (address, server) = mock_server(5, |index, request, address| match index {
            0 => reply(
                302,
                &[(
                    "Location",
                    &format!("http://uis.test:{}/authserver/login", address.port()),
                )],
                "",
            ),
            1 => reply(
                200,
                &[("Set-Cookie", "JSESSIONID=uis; Path=/")],
                r#"<input name="username"><input name="password"><input type="hidden" name="execution" value="opaque&amp;value"><input type="hidden" name="_eventId" value="submit"><input placeholder="请输入验证码">"#,
            ),
            2 => {
                assert_eq!(request.method, "POST");
                assert_eq!(request.path, "/authserver/login");
                let values = Url::parse(&format!("https://example.test/?{}", request.body))
                    .unwrap()
                    .query_pairs()
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect::<std::collections::HashMap<_, _>>();
                assert_eq!(values["execution"], "opaque&value");
                assert_eq!(values["username"], "student");
                assert_eq!(values["password"], "test-password");
                assert!(request.headers.contains("JSESSIONID=uis"));
                reply(
                    302,
                    &[(
                        "Location",
                        &format!("http://webvpn.test:{}/portal", address.port()),
                    )],
                    "",
                )
            }
            3 => reply(
                200,
                &[("Set-Cookie", "wrdvpn_ticket=authenticated; Path=/")],
                "<html>Gateway</html>",
            ),
            4 => reply(200, &[("Content-Type", "application/json")], "[]"),
            _ => unreachable!(),
        });
        WebVpnSession::login_at("student", "test-password", &endpoints(address))
            .await
            .unwrap();
        server.join().unwrap();
    }

    #[tokio::test]
    async fn authentication_never_follows_an_unexpected_host() {
        let (address, server) = mock_server(1, |_, _, _| {
            reply(302, &[("Location", "https://example.invalid/steal")], "")
        });
        let error = WebVpnSession::login_at("student", "test-password", &endpoints(address))
            .await
            .err()
            .unwrap();
        assert!(error
            .to_string()
            .contains("unexpected authentication redirect"));
        server.join().unwrap();
    }

    #[test]
    fn session_persists_session_cookies_but_no_identity_cookies() {
        let session = WebVpnSession::empty();
        {
            let mut store = session.jar.lock().unwrap();
            store
                .parse(
                    "wrdvpn_ticket=session-value; Path=/; Secure; HttpOnly",
                    &Url::parse("https://webvpn.fudan.edu.cn/").unwrap(),
                )
                .unwrap();
            store
                .parse(
                    "CASTGC=identity-secret; Path=/; Secure",
                    &Url::parse("https://id.fudan.edu.cn/").unwrap(),
                )
                .unwrap();
            store
                .parse(
                    "broad_sso=identity-wide; Domain=fudan.edu.cn; Path=/; Secure",
                    &Url::parse("https://id.fudan.edu.cn/").unwrap(),
                )
                .unwrap();
        }
        let dir =
            std::env::temp_dir().join(format!("danta-webvpn-test-{:016x}", rand::random::<u64>()));
        let path = dir.join("webvpn.json");
        session.save_to(&path, WEBVPN_HOST).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("session-value"));
        assert!(!raw.contains("identity-secret"));
        assert!(!raw.contains("identity-wide"));
        let restored = WebVpnSession::load_from(&path, WEBVPN_HOST)
            .unwrap()
            .unwrap();
        assert_eq!(
            restored
                .jar
                .lock()
                .unwrap()
                .get_request_values(&Url::parse("https://webvpn.fudan.edu.cn/").unwrap())
                .collect::<Vec<_>>(),
            vec![("wrdvpn_ticket", "session-value")]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cookie_updates_survive_reload_without_overwriting_a_new_login_or_logout() {
        let dir = std::env::temp_dir().join(format!(
            "danta-webvpn-rotate-{:016x}",
            rand::random::<u64>()
        ));
        let path = dir.join("webvpn.json");
        let gateway = Url::parse("https://webvpn.fudan.edu.cn/").unwrap();
        let set_ticket = |session: &WebVpnSession, value: &str| {
            session
                .jar
                .lock()
                .unwrap()
                .parse(
                    &format!("wrdvpn_ticket={value}; Path=/; Secure; HttpOnly"),
                    &gateway,
                )
                .unwrap();
        };
        let read_ticket = || {
            let session = WebVpnSession::load_from(&path, WEBVPN_HOST)
                .unwrap()
                .unwrap();
            let store = session.jar.lock().unwrap();
            let value = store
                .get_request_values(&gateway)
                .next()
                .unwrap()
                .1
                .to_owned();
            value
        };

        let login = WebVpnSession::empty();
        set_ticket(&login, "initial");
        login.save_to(&path, WEBVPN_HOST).unwrap();
        let active = WebVpnSession::load_from(&path, WEBVPN_HOST)
            .unwrap()
            .unwrap();
        set_ticket(&active, "renewed-by-gateway");
        active.persist_cookie_updates().unwrap();
        assert_eq!(read_ticket(), "renewed-by-gateway");

        set_ticket(&login, "new-login");
        login.save_to(&path, WEBVPN_HOST).unwrap();
        set_ticket(&active, "delayed-update");
        active.persist_cookie_updates().unwrap();
        assert_eq!(read_ticket(), "new-login");

        fs::remove_file(&path).unwrap();
        active.persist_cookie_updates().unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn authentication_errors_are_actionable_and_do_not_include_server_secrets() {
        for (input, expected) in [
            ("用户名或密码错误 secret", "incorrect"),
            ("请输入验证码 secret", "CAPTCHA"),
            ("弱密码提示 secret", "password update"),
            ("网络维护中 | Under Maintenance secret", "maintenance"),
        ] {
            let error = check_login_error(input).unwrap_err().to_string();
            assert!(error.contains(expected));
            assert!(!error.contains("secret"));
        }
    }
}
