//! Account directory only. Registry membership does not grant peer control permission.
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::Path as FilePath,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;

const SESSION_SECONDS: i64 = 30 * 24 * 60 * 60;
const ONLINE_SECONDS: i64 = 90;
const MAX_DEVICES: i64 = 100;
const MAX_SESSIONS: i64 = 100;

#[derive(Clone, Copy, Default)]
pub struct Config {
    pub signup_enabled: bool,
}

pub struct Server {
    db: Mutex<Connection>,
    config: Config,
    password_workers: Arc<Semaphore>,
    auth_attempts: Mutex<HashMap<IpAddr, (Instant, u32)>>,
    // Equal password work for unknown and known accounts.
    dummy_hash: String,
}

impl Server {
    pub fn open(path: &FilePath, config: Config) -> anyhow::Result<Arc<Self>> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .mode(0o600)
                .open(path)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        anyhow::ensure!(version <= 1, "database schema is newer than this server");
        db.execute_batch(include_str!("schema.sql"))?;
        Ok(Arc::new(Self {
            db: Mutex::new(db),
            config,
            password_workers: Arc::new(Semaphore::new(2)),
            auth_attempts: Mutex::new(HashMap::new()),
            dummy_hash: password_hash(&random_token())?,
        }))
    }

    async fn work<T: Send + 'static>(
        self: Arc<Self>,
        action: impl FnOnce(&mut Connection) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        tokio::task::spawn_blocking(move || {
            let mut db = self.db.lock().map_err(|_| ApiError::internal())?;
            // Cascade expired sessions into their device credentials.
            db.execute("DELETE FROM sessions WHERE expires_at <= ?", [now()])?;
            action(&mut db)
        })
        .await
        .map_err(|_| ApiError::internal())?
    }
}

pub fn router(server: Arc<Server>) -> Router {
    let auth = Router::new()
        .route("/v1/auth/signup", post(signup))
        .route("/v1/auth/login", post(login))
        .route_layer(middleware::from_fn_with_state(
            server.clone(),
            throttle_auth,
        ));
    Router::new()
        .merge(auth)
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/v1/server", get(server_info))
        .route("/v1/account", get(account))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/devices", get(devices).post(register_device))
        .route("/v1/devices/{id}", delete(revoke_device))
        .route("/v1/devices/{id}/heartbeat", post(heartbeat))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(private_response))
        .with_state(server)
}

async fn private_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

async fn throttle_auth(
    State(server): State<Arc<Server>>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    {
        let mut attempts = server
            .auth_attempts
            .lock()
            .map_err(|_| ApiError::internal())?;
        attempts.retain(|_, (start, _)| start.elapsed() < Duration::from_secs(60));
        // Never trust forwarded headers. Behind a proxy this limit applies to the proxy.
        if attempts.len() >= 10_000 && !attempts.contains_key(&address.ip()) {
            return Err(ApiError::limited());
        }
        let (_, count) = attempts.entry(address.ip()).or_insert((Instant::now(), 0));
        if *count >= 20 {
            return Err(ApiError::limited());
        }
        *count += 1;
    }
    Ok(next.run(request).await)
}

async fn server_info(State(server): State<Arc<Server>>) -> Json<Value> {
    Json(
        json!({"api_version": 1, "signup_enabled": server.config.signup_enabled,
        "heartbeat_interval_seconds": 30, "online_timeout_seconds": ONLINE_SECONDS}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    email: String,
    password: String,
}

fn credentials(input: Credentials) -> Result<Credentials, ApiError> {
    let email = input.email.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err(ApiError::bad("Invalid email address"));
    };
    if email.len() > 254
        || local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !email.is_ascii()
        || email.chars().any(char::is_whitespace)
        || email.chars().any(char::is_control)
    {
        return Err(ApiError::bad("Invalid email address"));
    }
    if input.password.len() < 12 || input.password.len() > 1024 {
        return Err(ApiError::bad("Password must contain 12 to 1024 bytes"));
    }
    Ok(Credentials {
        email,
        password: input.password,
    })
}

fn password_hash(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| anyhow::anyhow!("password hashing failed"))
}

async fn signup(
    State(server): State<Arc<Server>>,
    Json(input): Json<Credentials>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !server.config.signup_enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Account registration is disabled",
        ));
    }
    let input = credentials(input)?;
    let permit = server
        .password_workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::limited())?;
    let hash = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        password_hash(&input.password)
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|_| ApiError::internal())?;
    let result = server
        .work(move |db| {
            let tx = db.transaction()?;
            let id = random_token();
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO accounts VALUES (?, ?, ?, ?)",
                params![id, input.email, hash, now()],
            )?;
            if inserted == 0 {
                return Err(ApiError(StatusCode::CONFLICT, "Account cannot be created"));
            }
            let response = new_session(&tx, &id, &input.email)?;
            tx.commit()?;
            Ok(response)
        })
        .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

async fn login(
    State(server): State<Arc<Server>>,
    Json(input): Json<Credentials>,
) -> Result<Json<Value>, ApiError> {
    let input = credentials(input)?;
    let email = input.email.clone();
    let record = server
        .clone()
        .work(move |db| {
            Ok(db
                .query_row(
                    "SELECT id, password_hash FROM accounts WHERE email = ?",
                    [email],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?)
        })
        .await?;
    let hash = record
        .as_ref()
        .map(|(_, h)| h.clone())
        .unwrap_or_else(|| server.dummy_hash.clone());
    let permit = server
        .password_workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::limited())?;
    let valid = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        PasswordHash::new(&hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(input.password.as_bytes(), &hash)
                .is_ok()
        })
    })
    .await
    .map_err(|_| ApiError::internal())?;
    let Some((id, _)) = record.filter(|_| valid) else {
        return Err(ApiError::unauthorized());
    };
    server
        .work(move |db| new_session(db, &id, &input.email))
        .await
        .map(Json)
}

fn new_session(db: &Connection, account: &str, email: &str) -> Result<Value, ApiError> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM sessions WHERE account_id = ?",
        [account],
        |r| r.get(0),
    )?;
    if count >= MAX_SESSIONS {
        return Err(ApiError::limited());
    }
    let token = random_token();
    let expires = now() + SESSION_SECONDS;
    db.execute(
        "INSERT INTO sessions VALUES (?, ?, ?)",
        params![token_hash(&token), account, expires],
    )?;
    Ok(json!({"account": {"id": account, "email": email}, "token": token, "expires_at": expires}))
}

fn bearer(headers: &HeaderMap) -> Result<String, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::unauthorized());
    }
    Ok(token_hash(token))
}

fn owner(db: &Connection, session: &str) -> Result<String, ApiError> {
    db.query_row(
        "SELECT account_id FROM sessions WHERE token_hash = ? AND expires_at > ?",
        params![session, now()],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(ApiError::unauthorized)
}

async fn account(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    server
        .work(move |db| {
            let id = owner(db, &session)?;
            let email: String =
                db.query_row("SELECT email FROM accounts WHERE id = ?", [&id], |r| {
                    r.get(0)
                })?;
            Ok(json!({"id": id, "email": email}))
        })
        .await
        .map(Json)
}

async fn logout(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let session = bearer(&headers)?;
    server
        .work(move |db| {
            owner(db, &session)?;
            db.execute("DELETE FROM sessions WHERE token_hash = ?", [session])?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    public_key: String,
    name: String,
    platform: String,
}

#[derive(Serialize)]
struct Device {
    id: String,
    fingerprint: String,
    name: String,
    platform: String,
    created_at: i64,
    last_seen: Option<i64>,
    online: bool,
}

async fn register_device(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<Registration>,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    let key = hex::decode(&input.public_key).map_err(|_| ApiError::bad("Invalid public key"))?;
    if key.len() != 32 || key.iter().all(|b| *b == 0) {
        return Err(ApiError::bad("Invalid public key"));
    }
    let fingerprint = hex::encode(Sha256::digest(&key));
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(ApiError::bad(
            "Device name must contain 1 to 128 bytes without control characters",
        ));
    }
    if !["macos", "windows", "linux"].contains(&input.platform.as_str()) {
        return Err(ApiError::bad("Unsupported platform"));
    }
    server.work(move |db| {
        let tx = db.transaction()?;
        let account = owner(&tx, &session)?;
        let existing: Option<String> = tx.query_row(
            "SELECT id FROM devices WHERE account_id = ? AND fingerprint = ?",
            params![account, fingerprint], |r| r.get(0)).optional()?;
        let id = match existing {
            Some(id) => {
                tx.execute("UPDATE devices SET name = ?, platform = ? WHERE id = ?", params![name, input.platform, id])?;
                id
            }
            None => {
                let count: i64 = tx.query_row("SELECT COUNT(*) FROM devices WHERE account_id = ?", [&account], |r| r.get(0))?;
                if count >= MAX_DEVICES { return Err(ApiError::limited()); }
                let id = random_token();
                tx.execute("INSERT INTO devices VALUES (?, ?, ?, ?, ?, ?)",
                    params![id, account, fingerprint, name, input.platform, now()])?;
                id
            }
        };
        let token = random_token();
        tx.execute("INSERT INTO device_sessions VALUES (?, ?, ?, NULL)
            ON CONFLICT(session_hash, device_id) DO UPDATE SET token_hash = excluded.token_hash, last_seen = NULL",
            params![token_hash(&token), session, id])?;
        tx.commit()?;
        // Heartbeat credentials can only update this device, never list or revoke others.
        Ok(json!({"id": id, "fingerprint": fingerprint, "device_token": token}))
    }).await.map(Json)
}

async fn devices(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    server
        .work(move |db| {
            let account = owner(db, &session)?;
            let mut query = db.prepare(
                "SELECT d.id, d.fingerprint, d.name, d.platform, d.created_at,
            MAX(ds.last_seen) FROM devices d
            LEFT JOIN device_sessions ds ON ds.device_id = d.id
            WHERE d.account_id = ? GROUP BY d.id ORDER BY d.created_at, d.id",
            )?;
            let timestamp = now();
            let devices = query
                .query_map([account], |r| {
                    let last_seen: Option<i64> = r.get(5)?;
                    Ok(Device {
                        id: r.get(0)?,
                        fingerprint: r.get(1)?,
                        name: r.get(2)?,
                        platform: r.get(3)?,
                        created_at: r.get(4)?,
                        last_seen,
                        online: last_seen.is_some_and(|at| at > timestamp - ONLINE_SECONDS),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"devices": devices}))
        })
        .await
        .map(Json)
}

async fn heartbeat(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let credential = bearer(&headers)?;
    server
        .work(move |db| {
            let changed = db.execute(
                "UPDATE device_sessions SET last_seen = ? WHERE token_hash = ? AND device_id = ?",
                params![now(), credential, id],
            )?;
            if changed == 0 {
                return Err(ApiError::unauthorized());
            }
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

async fn revoke_device(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session = bearer(&headers)?;
    server
        .work(move |db| {
            let account = owner(db, &session)?;
            let changed = db.execute(
                "DELETE FROM devices WHERE id = ? AND account_id = ?",
                params![id, account],
            )?;
            if changed == 0 {
                return Err(ApiError(StatusCode::NOT_FOUND, "Device not found"));
            }
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before Unix epoch")
        .as_secs() as i64
}
fn random_token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub struct ApiError(StatusCode, &'static str);
impl ApiError {
    fn bad(message: &'static str) -> Self {
        Self(StatusCode::BAD_REQUEST, message)
    }
    fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "Invalid or expired credentials")
    }
    fn internal() -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
    }
    fn limited() -> Self {
        Self(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests; try again later",
        )
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(_: rusqlite::Error) -> Self {
        Self::internal()
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.0, Json(json!({"error": self.1}))).into_response();
        if self.0 == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
        }
        response
    }
}

#[cfg(test)]
mod tests;
