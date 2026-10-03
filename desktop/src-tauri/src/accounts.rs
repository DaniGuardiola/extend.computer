//! Account credentials never cross the webview bridge. Local pairing remains independent.
use crate::runtime::Desktop;
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use url::Url;
use zeroize::Zeroizing;
const OFFICIAL: &str = "https://extend.computer";
#[derive(Clone, Default, Serialize)]
pub struct View {
    pub server: String,
    pub email: Option<String>,
    pub device_id: Option<String>,
    pub devices: Vec<Value>,
    pub pending: Option<Value>,
    pub browser_pending: bool,
    pub error: Option<String>,
}
#[derive(Deserialize, Serialize)]
struct Credential {
    token: String,
    email: String,
    device_id: Option<String>,
    device_token: Option<String>,
    registered_name: Option<String>,
    #[serde(default)]
    key_verified: bool,
}
struct Inner {
    view: View,
    credential: Option<Credential>,
    ticket: Option<String>,
    loaded: bool,
}
pub struct Accounts {
    desktop: Arc<Desktop>,
    client: Client,
    inner: Mutex<Inner>,
    generation: AtomicU64,
    settings: PathBuf,
}
fn origin(raw: &str) -> Result<String> {
    let url = Url::parse(raw.trim()).context("Enter a valid account server URL.")?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Use the server origin without a path, query, or credentials."
    );
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
        "Account servers require HTTPS; localhost HTTP is allowed for development."
    );
    Ok(url.origin().ascii_serialization())
}
fn random() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
impl Accounts {
    pub fn new(desktop: Arc<Desktop>) -> Result<Arc<Self>> {
        let settings = desktop.root.join("account-server.json");
        let server = std::fs::read(&settings)
            .ok()
            .and_then(|b| serde_json::from_slice::<String>(&b).ok())
            .and_then(|s| origin(&s).ok())
            .unwrap_or_else(|| OFFICIAL.into());
        Ok(Arc::new(Self {
            desktop,
            settings,
            client: Client::builder()
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            inner: Mutex::new(Inner {
                view: View {
                    server,
                    ..View::default()
                },
                credential: None,
                ticket: None,
                loaded: false,
            }),
            generation: AtomicU64::new(0),
        }))
    }
    fn entry(server: &str) -> Result<keyring::Entry> {
        #[cfg(not(test))]
        let service = if cfg!(feature = "dev-identity") {
            "computer.extend.accounts.development".to_owned()
        } else {
            "computer.extend.accounts".to_owned()
        };
        #[cfg(test)]
        let service = format!("computer.extend.accounts.tests.{}", std::process::id());
        Ok(keyring::Entry::new(&service, server)?)
    }
    fn load(inner: &mut Inner) -> Result<()> {
        if inner.loaded {
            return Ok(());
        }
        match Self::entry(&inner.view.server)?.get_password() {
            Ok(raw) => {
                let raw = Zeroizing::new(raw);
                inner.credential = Some(
                    serde_json::from_str(&raw)
                        .context("Saved login is damaged. Sign out and try again.")?,
                );
                inner.view.email = inner.credential.as_ref().map(|c| c.email.clone());
            }
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e).context("Could not read the system credential store."),
        }
        inner.loaded = true;
        Ok(())
    }
    fn save(inner: &Inner) -> Result<()> {
        if let Some(c) = &inner.credential {
            let raw = Zeroizing::new(serde_json::to_string(c)?);
            Self::entry(&inner.view.server)?
                .set_password(&raw)
                .context("Could not save login in the system credential store.")?;
        }
        Ok(())
    }
    fn clear(inner: &mut Inner) -> Result<()> {
        match Self::entry(&inner.view.server)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => {
                return Err(e).context("Could not remove login from the system credential store.")
            }
        }
        inner.credential = None;
        inner.ticket = None;
        inner.view.email = None;
        inner.view.device_id = None;
        inner.view.devices.clear();
        inner.view.pending = None;
        inner.loaded = true;
        Ok(())
    }
    fn request(
        &self,
        server: &str,
        path: &str,
        method: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> Result<Value> {
        let mut req = self
            .client
            .request(method.parse()?, format!("{server}/v1{path}"))
            .header("Origin", server)
            .header("X-Extend-Client", "desktop");
        if let Some(token) = token {
            req = req.bearer_auth(token)
        }
        if let Some(body) = body {
            req = req.json(&body)
        }
        let mut response = req.send().context("Could not reach your account server.")?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(256 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 256 * 1024,
            "Account server response is too large."
        );
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !status.is_success() {
            if status.as_u16() == 401 {
                bail!("Your login or code is invalid or expired. Please sign in again.")
            }
            bail!(
                "{}",
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Account request failed. Try again.")
            );
        }
        Ok(value)
    }
    pub fn view(&self) -> View {
        self.inner.lock().unwrap().view.clone()
    }
    pub fn configure(&self, server: String) -> Result<View> {
        let server = origin(&server)?;
        let mut inner = self.inner.lock().unwrap();
        ensure!(
            inner.credential.is_none() && !inner.view.browser_pending,
            "Sign out or cancel sign-in before changing servers."
        );
        self.generation.fetch_add(1, Ordering::SeqCst);
        std::fs::create_dir_all(&self.desktop.root)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.desktop.root)?;
        serde_json::to_writer(&mut file, &server)?;
        file.as_file().sync_all()?;
        file.persist(&self.settings)?;
        inner.view = View {
            server,
            ..View::default()
        };
        inner.loaded = false;
        inner.ticket = None;
        Self::load(&mut inner)?;
        Ok(inner.view.clone())
    }
    fn accept(inner: &mut Inner, value: Value) -> Result<()> {
        if value.get("mfa_required").and_then(Value::as_bool) == Some(true) {
            inner.ticket = Some(
                value
                    .get("ticket")
                    .and_then(Value::as_str)
                    .context("Missing second-factor ticket.")?
                    .into(),
            );
            inner.view.pending = Some(
                json!({"totp":value["totp"],"keys":value["keys"],"recovery":value["recovery"]}),
            );
            return Ok(());
        }
        let token = value
            .get("token")
            .and_then(Value::as_str)
            .context("Server did not return a desktop session.")?;
        ensure!(
            token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid session response."
        );
        let email = value["account"]["email"]
            .as_str()
            .context("Missing account email.")?
            .to_owned();
        let c = Credential {
            token: token.into(),
            email: email.clone(),
            device_id: None,
            device_token: None,
            registered_name: None,
            key_verified: false,
        };
        // Save before exposing a successful login. A failed save must not leave a memory-only login.
        let raw = Zeroizing::new(serde_json::to_string(&c)?);
        Self::entry(&inner.view.server)?
            .set_password(&raw)
            .context("Could not save login in the system credential store.")?;
        inner.credential = Some(c);
        inner.loaded = true;
        inner.ticket = None;
        inner.view.pending = None;
        inner.view.email = Some(email);
        inner.view.error = None;
        Ok(())
    }
    fn accept_or_revoke(&self, inner: &mut Inner, value: Value) -> Result<()> {
        let token = value
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Err(e) = Self::accept(inner, value) {
            if let Some(token) = token {
                let _ = self.request(
                    &inner.view.server,
                    "/auth/logout",
                    "POST",
                    Some(json!({})),
                    Some(&token),
                );
            }
            return Err(e);
        }
        Ok(())
    }
    pub fn login(&self, email: String, password: String) -> Result<View> {
        let password = Zeroizing::new(password);
        let mut inner = self.inner.lock().unwrap();
        Self::load(&mut inner)?;
        ensure!(
            inner.credential.is_none(),
            "Sign out before using another account."
        );
        self.generation.fetch_add(1, Ordering::SeqCst);
        inner.view.browser_pending = false;
        let value = self.request(
            &inner.view.server,
            "/auth/login",
            "POST",
            Some(json!({"email":email,"password":password.as_str()})),
            None,
        )?;
        self.accept_or_revoke(&mut inner, value)?;
        drop(inner);
        self.refresh()
    }
    pub fn verify(&self, code: String) -> Result<View> {
        let mut inner = self.inner.lock().unwrap();
        let ticket = inner.ticket.as_ref().context("Start sign-in again.")?;
        let value = self.request(
            &inner.view.server,
            "/auth/mfa/verify",
            "POST",
            Some(json!({"ticket":ticket,"code":code})),
            None,
        )?;
        self.accept_or_revoke(&mut inner, value)?;
        drop(inner);
        self.refresh()
    }
    pub fn refresh(&self) -> Result<View> {
        let mut inner = self.inner.lock().unwrap();
        Self::load(&mut inner)?;
        let server = inner.view.server.clone();
        let Some(c) = inner.credential.as_mut() else {
            self.desktop.clear_account_peers();
            return Ok(inner.view.clone());
        };
        let user = match self.request(&server, "/account", "GET", None, Some(&c.token)) {
            Ok(user) => user,
            Err(e) => {
                if e.to_string().starts_with("Your login") {
                    self.desktop.clear_account_peers();
                    Self::clear(&mut inner)?;
                }
                inner.view.error = Some(e.to_string());
                return Ok(inner.view.clone());
            }
        };
        c.email = user["email"]
            .as_str()
            .context("Missing account email.")?
            .into();
        let local = self.desktop.local_device_info()?;
        if c.device_token.is_none() || c.registered_name.as_deref() != Some(&local.name) {
            let registered=self.request(&server,"/devices","POST",Some(json!({"public_key":self.desktop.account_public_key()?,"name":local.name,"platform":std::env::consts::OS})),Some(&c.token))?;
            c.device_id = Some(
                registered["id"]
                    .as_str()
                    .context("Missing device ID.")?
                    .into(),
            );
            c.device_token = Some(
                registered["device_token"]
                    .as_str()
                    .context("Missing device credential.")?
                    .into(),
            );
            c.registered_name = Some(local.name);
            c.key_verified = false;
            Self::save(&inner)?;
        }
        let c = inner.credential.as_ref().unwrap();
        if !c.key_verified {
            let path = format!("/devices/{}/proof", c.device_id.as_ref().unwrap());
            let challenge = self.request(
                &server,
                &(path.clone() + "/options"),
                "POST",
                Some(json!({})),
                c.device_token.as_deref(),
            )?;
            let proof = self.desktop.account_key_proof(
                challenge["server_key"]
                    .as_str()
                    .context("Invalid server key")?,
                challenge["challenge"]
                    .as_str()
                    .context("Invalid challenge")?,
            )?;
            self.request(
                &server,
                &(path + "/verify"),
                "POST",
                Some(json!({"challenge":challenge["challenge"],"proof":proof})),
                c.device_token.as_deref(),
            )?;
            inner.credential.as_mut().unwrap().key_verified = true;
            Self::save(&inner)?;
        }
        let c = inner.credential.as_ref().unwrap();
        let heartbeat = self.request(
            &server,
            &format!("/devices/{}/heartbeat", c.device_id.as_ref().unwrap()),
            "POST",
            Some(json!({})),
            c.device_token.as_deref(),
        );
        if let Err(e) = heartbeat {
            if e.to_string().starts_with("Your login") {
                self.desktop.clear_account_peers();
                Self::clear(&mut inner)?;
            }
            inner.view.error = Some(e.to_string());
            return Ok(inner.view.clone());
        }
        let c = inner.credential.as_ref().unwrap();
        let list = self.request(&server, "/devices", "GET", None, Some(&c.token))?;
        let email = c.email.clone();
        let device_id = c.device_id.clone();
        inner.view.email = Some(email);
        inner.view.device_id = device_id;
        inner.view.devices = list["devices"]
            .as_array()
            .context("Invalid device list.")?
            .clone();
        self.desktop
            .account_relay
            .configure(Some(crate::relay::RelayCredentials {
                server: server.clone(),
                device: inner
                    .credential
                    .as_ref()
                    .unwrap()
                    .device_id
                    .clone()
                    .unwrap(),
                token: inner
                    .credential
                    .as_ref()
                    .unwrap()
                    .device_token
                    .clone()
                    .unwrap(),
            }));
        self.desktop.sync_account_peers(
            &format!("{}/{}", server, inner.view.email.as_deref().unwrap()),
            &inner.view.devices,
        )?;
        inner.view.error = None;
        Ok(inner.view.clone())
    }
    pub fn logout(&self) -> Result<View> {
        self.desktop.clear_account_peers();
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.inner.lock().unwrap();
        inner.view.browser_pending = false;
        Self::load(&mut inner)?;
        let result = if let Some(c) = &inner.credential {
            self.request(
                &inner.view.server,
                "/auth/logout",
                "POST",
                Some(json!({})),
                Some(&c.token),
            )
            .map(|_| ())
        } else {
            Ok(())
        };
        Self::clear(&mut inner)?;
        self.desktop.clear_account_peers();
        inner.view.error=result.err().map(|_|"Signed out locally. Server was unreachable; revoke this session in account settings.".into());
        Ok(inner.view.clone())
    }
    pub fn remove(&self, id: String) -> Result<View> {
        let inner = self.inner.lock().unwrap();
        ensure!(
            id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid device ID."
        );
        let c = inner.credential.as_ref().context("Sign in first.")?;
        self.request(
            &inner.view.server,
            &format!("/devices/{id}"),
            "DELETE",
            None,
            Some(&c.token),
        )?;
        if c.device_id.as_deref() == Some(&id) {
            drop(inner);
            return self.logout();
        }
        drop(inner);
        self.refresh()
    }
    pub fn cancel(&self) -> View {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.inner.lock().unwrap();
        inner.ticket = None;
        inner.view.pending = None;
        inner.view.browser_pending = false;
        inner.view.clone()
    }
    pub fn open_page(&self, page: String) -> Result<()> {
        ensure!(
            matches!(page.as_str(), "signup" | "account" | "recover"),
            "Unknown account page."
        );
        let server = self.inner.lock().unwrap().view.server.clone();
        if server != OFFICIAL {
            let info = self.request(&server, "/server", "GET", None, None)?;
            ensure!(info["desktop_browser_login"].as_bool()==Some(true), "This installation provides only an account API. Use its administrator's website for signup and account settings.");
        }
        open::that(format!("{server}/{page}"))?;
        Ok(())
    }
    pub fn browser(self: &Arc<Self>, on_signed_in: impl FnOnce() + Send + 'static) -> Result<View> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let server = self.inner.lock().unwrap().view.server.clone();
        let info = self.request(&server, "/server", "GET", None, None)?;
        ensure!(info["desktop_browser_login"].as_bool()==Some(true),"This server supports password and code login. Browser sign-in requires an account website.");
        let state = random();
        let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(hex::decode(random())?));
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut inner = self.inner.lock().unwrap();
        ensure!(
            inner.credential.is_none(),
            "Sign out before using another account."
        );
        let server = inner.view.server.clone();
        let mut url = Url::parse(&format!("{server}/desktop/connect"))?;
        url.query_pairs_mut()
            .append_pair("port", &port.to_string())
            .append_pair("state", &state)
            .append_pair("challenge", &challenge);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        open::that(url.as_str())?;
        inner.view.browser_pending = true;
        inner.view.error = None;
        let view = inner.view.clone();
        drop(inner);
        let this = self.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let deadline = Instant::now() + Duration::from_secs(300);
                while Instant::now() < deadline {
                    if this.generation.load(Ordering::SeqCst) != generation {
                        return Ok(());
                    }
                    let (mut socket, _) = match listener.accept() {
                        Ok(v) => v,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        Err(e) => return Err(e.into()),
                    };
                    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
                    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
                    let mut bytes = [0u8; 8192];
                    let n = socket.read(&mut bytes)?;
                    let text = std::str::from_utf8(&bytes[..n])?;
                    let target = text
                        .lines()
                        .next()
                        .and_then(|line| line.strip_prefix("GET "))
                        .and_then(|v| v.strip_suffix(" HTTP/1.1"));
                    let callback = target.and_then(|path| {
                        Url::parse(&format!("http://127.0.0.1:{port}{path}")).ok()
                    });
                    let valid = callback
                        .as_ref()
                        .filter(|u| u.path() == "/callback")
                        .and_then(|u| {
                            let p = u.query_pairs().collect::<std::collections::HashMap<_, _>>();
                            if p.get("state").map(|s| s.as_ref()) != Some(state.as_str()) {
                                return None;
                            }
                            p.get("code").map(|s| s.to_string())
                        });
                    let Some(code) =
                        valid.filter(|c| c.len() == 64 && c.bytes().all(|b| b.is_ascii_hexdigit()))
                    else {
                        let _=socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    };
                    let value = this.request(
                        &server,
                        "/auth/desktop/exchange",
                        "POST",
                        Some(json!({"code":code,"verifier":verifier.as_str()})),
                        None,
                    )?;
                    let mut inner = this.inner.lock().unwrap();
                    if this.generation.load(Ordering::SeqCst) != generation {
                        if let Some(token) = value.get("token").and_then(Value::as_str) {
                            let _ = this.request(
                                &server,
                                "/auth/logout",
                                "POST",
                                Some(json!({})),
                                Some(token),
                            );
                        }
                        return Ok(());
                    }
                    this.accept_or_revoke(&mut inner, value)?;
                    inner.view.browser_pending = false;
                    drop(inner);
                    // Fixed account-server destination; no credentials or callback
                    // parameters cross into the completion page or its referrer.
                    let response = browser_completion_response(&server);
                    let _ = socket.write_all(response.as_bytes());
                    on_signed_in();
                    this.refresh()?;
                    return Ok(());
                }
                bail!("Browser sign-in timed out. Start again.")
            })();
            if let Err(e) = result {
                let mut inner = this.inner.lock().unwrap();
                if this.generation.load(Ordering::SeqCst) == generation {
                    inner.view.browser_pending = false;
                    inner.view.error = Some(e.to_string());
                }
            }
        });
        Ok(view)
    }
    pub fn start(self: &Arc<Self>) {
        self.desktop.account_relay.start(&self.desktop);
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            let Some(this) = weak.upgrade() else { break };
            if let Err(e) = this.refresh() {
                this.inner.lock().unwrap().view.error = Some(e.to_string());
            }
            drop(this);
            std::thread::sleep(Duration::from_secs(30));
        });
    }
}
fn browser_completion_response(server: &str) -> String {
    format!("HTTP/1.1 303 See Other\r\nLocation: {server}/desktop/connected#close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_redirect_uses_account_origin_without_callback_secrets() {
        for server in [
            OFFICIAL,
            "https://accounts.example.com",
            "http://localhost:3010",
        ] {
            let response = browser_completion_response(server);
            assert!(response.starts_with("HTTP/1.1 303 See Other\r\n"));
            assert!(response.contains(&format!("Location: {server}/desktop/connected#close\r\n")));
            assert!(response.contains("Referrer-Policy: no-referrer\r\n"));
            assert!(response.contains("Cache-Control: no-store\r\n"));
            assert!(!response.contains("state=") && !response.contains("code="));
        }
    }
    #[test]
    fn server_origin_rejects_credentials_and_insecure_remote() {
        assert!(origin("https://extend.computer").is_ok());
        assert!(origin("http://localhost:8080").is_ok());
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/path",
            "https://example.com?x=1",
        ] {
            assert!(origin(url).is_err());
        }
    }
}

#[cfg(all(test, feature = "dev-identity"))]
mod integration {
    use super::*;
    #[test]
    #[ignore = "Requires isolated local website; writes only a temporary test Keychain entry"]
    fn native_login_restore_presence_mfa_and_logout() {
        let server = std::env::var("EXTEND_ACCOUNT_TEST_URL").expect("EXTEND_ACCOUNT_TEST_URL");
        assert!(server.starts_with("http://localhost:"));
        let temp = tempfile::tempdir().unwrap();
        let desktop = Desktop::new(
            temp.path().join("profile"),
            temp.path().join("unused-helper"),
        )
        .unwrap();
        let accounts = Accounts::new(desktop.clone()).unwrap();
        accounts.configure(server.clone()).unwrap();
        let email = format!("native-{}@example.invalid", random());
        let password = random();
        let signup = accounts
            .request(
                &server,
                "/auth/signup",
                "POST",
                Some(json!({"email":email,"password":password})),
                None,
            )
            .unwrap();
        let id = signup["account"]["id"].as_str().unwrap().to_owned();
        struct Cleanup {
            server: String,
            id: String,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = Accounts::entry(&self.server)
                    .and_then(|e| e.delete_credential().map_err(Into::into));
                let _ = sql(&format!("DELETE FROM accounts WHERE id='{}';", self.id));
            }
        }
        let _cleanup = Cleanup {
            server: server.clone(),
            id: id.clone(),
        };
        accounts
            .request(
                &server,
                "/auth/logout",
                "POST",
                Some(json!({})),
                signup["token"].as_str(),
            )
            .unwrap();
        let signed = accounts.login(email.clone(), password.clone()).unwrap();
        assert_eq!(signed.email.as_deref(), Some(email.as_str()));
        assert!(signed.device_id.is_some());
        let own = signed
            .devices
            .iter()
            .find(|d| d["id"] == signed.device_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(own["online"], true);
        let stored = Accounts::entry(&server).unwrap().get_password().unwrap();
        let credential: Credential = serde_json::from_str(&stored).unwrap();
        let bridge = serde_json::to_string(&signed).unwrap();
        assert!(!bridge.contains(&credential.token));
        assert!(!bridge.contains(credential.device_token.as_ref().unwrap()));
        drop(accounts);
        let restored = Accounts::new(desktop).unwrap();
        let view = restored.refresh().unwrap();
        assert_eq!(view.email, Some(email.clone()));
        assert_eq!(view.device_id, signed.device_id);
        assert_eq!(view.devices.len(), 1);
        restored.logout().unwrap();
        assert!(restored.view().email.is_none());
        assert!(matches!(
            Accounts::entry(&server).unwrap().get_password(),
            Err(keyring::Error::NoEntry)
        ));
        let code = random()[..32].to_owned();
        let digest = hex::encode(Sha256::digest(code.as_bytes()));
        sql(&format!("UPDATE accounts SET mfa_enabled=1 WHERE id='{id}'; INSERT INTO recovery_codes VALUES ('{id}','{digest}');")).unwrap();
        let pending = restored.login(email.clone(), password).unwrap();
        assert!(pending.pending.is_some());
        assert!(pending.email.is_none());
        assert!(restored.verify("000000".into()).is_err());
        let completed = restored.verify(code).unwrap();
        assert_eq!(completed.email, Some(email));
        assert!(completed.pending.is_none());
        restored.logout().unwrap();
    }
    fn sql(statement: &str) -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
        let result = std::process::Command::new("node")
            .args([
                "node_modules/wrangler/bin/wrangler.js",
                "d1",
                "execute",
                "extend-computer",
                "--local",
                "--command",
                statement,
            ])
            .current_dir(root)
            .env("WRANGLER_LOG_PATH", "/tmp/extend-native-account-test.log")
            .output()?;
        ensure!(
            result.status.success(),
            "Local fixture database command failed."
        );
        Ok(())
    }
}
