use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use tower::ServiceExt;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    data: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let mut request = builder.body(Body::from(data.to_string())).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
    ));
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let body = to_bytes(response.into_body(), 32_768).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn signup(app: &Router, email: &str) -> String {
    let (status, body) = request(
        app,
        "POST",
        "/v1/auth/signup",
        None,
        json!({"email": email, "password": "a sufficiently long password"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["token"].as_str().unwrap().to_owned()
}

async fn register(app: &Router, token: &str, key: u8) -> Value {
    let (status, body) = request(
        app,
        "POST",
        "/v1/devices",
        Some(token),
        json!({"name": "My Mac", "platform": "macos", "public_key": hex::encode([key; 32])}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn setup() -> (tempfile::TempDir, Arc<Server>, Router) {
    let dir = tempfile::tempdir().unwrap();
    let state = Server::open(
        &dir.path().join("server.sqlite3"),
        Config {
            signup_enabled: true,
            origin: Some("http://localhost:8080".into()),
        },
    )
    .unwrap();
    let app = router(state.clone());
    (dir, state, app)
}

#[tokio::test]
async fn devices_are_private_scoped_and_revocable() {
    let (_dir, _server, app) = setup();
    let alice = signup(&app, "alice@example.com").await;
    let bob = signup(&app, "bob@example.com").await;
    let device = register(&app, &alice, 1).await;
    let id = device["id"].as_str().unwrap();
    let token = device["device_token"].as_str().unwrap();
    let heartbeat = format!("/v1/devices/{id}/heartbeat");
    let endpoint = format!("/v1/devices/{id}");
    let (_, body) = request(&app, "GET", "/v1/devices", Some(&bob), Value::Null).await;
    assert_eq!(body["devices"], json!([]));
    assert_eq!(
        request(&app, "GET", "/v1/devices", None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "DELETE", &endpoint, Some(&bob), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "POST", &heartbeat, Some(&alice), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "GET", "/v1/devices", Some(token), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "POST", &heartbeat, Some(token), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (_, body) = request(&app, "GET", "/v1/devices", Some(&alice), Value::Null).await;
    assert_eq!(body["devices"][0]["online"], true);
    let second = register(&app, &alice, 2).await;
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/v1/devices/{}/heartbeat", second["id"].as_str().unwrap()),
            Some(token),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "DELETE", &endpoint, Some(&alice), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "POST", &heartbeat, Some(token), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn logout_expiry_and_stale_heartbeats_remove_presence() {
    let (_dir, server, app) = setup();
    let token = signup(&app, "alice@example.com").await;
    let device = register(&app, &token, 1).await;
    let endpoint = format!("/v1/devices/{}/heartbeat", device["id"].as_str().unwrap());
    let device_token = device["device_token"].as_str().unwrap();
    request(&app, "POST", &endpoint, Some(device_token), Value::Null).await;
    server
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE device_sessions SET last_seen = ?",
            [now() - ONLINE_SECONDS],
        )
        .unwrap();
    let (_, body) = request(&app, "GET", "/v1/devices", Some(&token), Value::Null).await;
    assert_eq!(body["devices"][0]["online"], false);
    let (status, login) = request(
        &app,
        "POST",
        "/v1/auth/login",
        None,
        json!({"email": "alice@example.com", "password": "a sufficiently long password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let other_session = login["token"].as_str().unwrap();
    assert_eq!(
        request(&app, "POST", "/v1/auth/logout", Some(&token), Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "POST", &endpoint, Some(device_token), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "GET", "/v1/account", Some(&token), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, body) = request(&app, "GET", "/v1/devices", Some(other_session), Value::Null).await;
    assert_eq!(body["devices"][0]["online"], false);
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);
    let active = register(&app, other_session, 1).await;
    assert_eq!(active["id"], device["id"]);
    server
        .db
        .lock()
        .unwrap()
        .execute("UPDATE sessions SET expires_at = ?", [now()])
        .unwrap();
    assert_eq!(
        request(
            &app,
            "POST",
            &endpoint,
            Some(active["device_token"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "GET", "/v1/account", Some(other_session), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn persistence_normalization_and_credential_storage() {
    let (dir, server, app) = setup();
    let token = signup(&app, "  Alice@Example.COM ").await;
    let device = register(&app, &token, 1).await;
    let renewed = register(&app, &token, 1).await;
    assert_eq!(renewed["id"], device["id"]);
    let endpoint = format!("/v1/devices/{}/heartbeat", device["id"].as_str().unwrap());
    assert_eq!(
        request(
            &app,
            "POST",
            &endpoint,
            Some(device["device_token"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    {
        let db = server.db.lock().unwrap();
        let stored: String = db
            .query_row("SELECT password_hash FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert!(stored.starts_with("$argon2id$"));
        let stored: String = db
            .query_row("SELECT token_hash FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, token_hash(&token));
        assert_ne!(stored, token);
        let stored: String = db
            .query_row("SELECT token_hash FROM device_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            stored,
            token_hash(renewed["device_token"].as_str().unwrap())
        );
    }
    drop(app);
    drop(server);
    let app = router(Server::open(&dir.path().join("server.sqlite3"), Config::default()).unwrap());
    let (status, account) = request(&app, "GET", "/v1/account", Some(&token), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account["email"], "alice@example.com");
    let (_, body) = request(&app, "GET", "/v1/devices", Some(&token), Value::Null).await;
    assert_eq!(body["devices"][0]["id"], device["id"]);
    assert_eq!(
        request(
            &app,
            "POST",
            &endpoint,
            Some(renewed["device_token"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/auth/signup",
            None,
            json!({"email": "new@example.com", "password": "a sufficiently long password"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn another_login_preserves_presence_and_cannot_claim_another_accounts_device() {
    let (_dir, _server, app) = setup();
    let first = signup(&app, "alice@example.com").await;
    let (_, login) = request(
        &app,
        "POST",
        "/v1/auth/login",
        None,
        json!({"email": "alice@example.com", "password": "a sufficiently long password"}),
    )
    .await;
    let second = login["token"].as_str().unwrap();
    let bob = signup(&app, "bob@example.com").await;
    let a = register(&app, &first, 1).await;
    let b = register(&app, second, 1).await;
    let c = register(&app, &bob, 1).await;
    assert_eq!(a["id"], b["id"]);
    assert_ne!(a["id"], c["id"]);
    let endpoint = format!("/v1/devices/{}/heartbeat", a["id"].as_str().unwrap());
    for device in [&a, &b] {
        assert_eq!(
            request(
                &app,
                "POST",
                &endpoint,
                Some(device["device_token"].as_str().unwrap()),
                Value::Null
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    request(&app, "POST", "/v1/auth/logout", Some(&first), Value::Null).await;
    let (_, devices) = request(&app, "GET", "/v1/devices", Some(second), Value::Null).await;
    assert_eq!(devices["devices"][0]["online"], true);
    assert_eq!(
        request(
            &app,
            "POST",
            &endpoint,
            Some(a["device_token"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &endpoint,
            Some(b["device_token"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn invalid_credentials_payloads_and_auth_flood_are_rejected() {
    let (_dir, _server, app) = setup();
    let token = signup(&app, "alice@example.com").await;
    for email in ["alice@example.com", "unknown@example.com"] {
        assert_eq!(
            request(
                &app,
                "POST",
                "/v1/auth/login",
                None,
                json!({"email": email, "password": "wrong long password"})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/auth/signup",
            None,
            json!({"email": "bad", "password": "short"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/devices",
            Some(&token),
            json!({"name": "mac", "platform": "macos", "public_key": "00"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/devices",
            Some(&token),
            json!({"name": "\n", "platform": "macos", "public_key": hex::encode([1; 32])})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/v1/auth/login",
            None,
            json!({"email": "alice@example.com", "password": "x".repeat(20_000)})
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for _ in 0..20 {
        request(
            &app,
            "POST",
            "/v1/auth/login",
            None,
            json!({"email": "bad", "password": "short"}),
        )
        .await;
    }
    assert_eq!(
        request(&app, "POST", "/v1/auth/login", None, Value::Null)
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}
