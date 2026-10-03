//! Session-bound, one-use X25519 possession proofs for automatic account pairing.
use super::*;
use subtle::ConstantTimeEq;
use x25519_dalek::{PublicKey, StaticSecret};

fn device_key(db: &Connection, credential: &str, id: &str) -> Result<String, ApiError> {
    db.query_row("SELECT d.public_key FROM devices d JOIN device_sessions ds ON ds.device_id=d.id JOIN sessions s ON s.token_hash=ds.session_hash JOIN accounts a ON a.id=s.account_id WHERE ds.token_hash=? AND d.id=? AND s.expires_at>? AND s.mfa_version=a.mfa_version", params![credential,id,now()], |r| r.get(0)).optional()?.ok_or_else(ApiError::unauthorized)
}

pub(super) async fn options(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let credential = bearer(&headers)?;
    server.work(move |db| {
        let key = device_key(db, &credential, &id)?;
        let key: [u8;32] = hex::decode(key).map_err(|_| ApiError::bad("Invalid public key"))?.try_into().map_err(|_| ApiError::bad("Invalid public key"))?;
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        let shared = secret.diffie_hellman(&PublicKey::from(key));
        if !shared.was_contributory() { return Err(ApiError::bad("Invalid public key")); }
        let challenge = random_token();
        let mut hash = Sha256::new();
        hash.update(b"extend.computer/device-proof/v1\0");
        hash.update(shared.as_bytes());
        hash.update(hex::decode(&challenge).unwrap());
        let expected = hex::encode(hash.finalize());
        db.execute("DELETE FROM device_proofs WHERE expires_at<=?", [now()])?;
        db.execute("INSERT INTO device_proofs VALUES (?, ?, ?, ?) ON CONFLICT(token_hash) DO UPDATE SET challenge=excluded.challenge,expected=excluded.expected,expires_at=excluded.expires_at", params![credential,challenge,expected,now()+120])?;
        Ok(json!({"server_key":hex::encode(public.as_bytes()),"challenge":challenge}))
    }).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Proof {
    challenge: String,
    proof: String,
}

pub(super) async fn verify(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Proof>,
) -> Result<StatusCode, ApiError> {
    let credential = bearer(&headers)?;
    server.work(move |db| {
        device_key(db, &credential, &id)?;
        let expected: Option<String> = db.query_row("SELECT expected FROM device_proofs WHERE token_hash=? AND challenge=? AND expires_at>?", params![credential,input.challenge,now()], |r| r.get(0)).optional()?;
        // A failed attempt consumes this challenge too.
        db.execute("DELETE FROM device_proofs WHERE token_hash=?", [&credential])?;
        if !expected.is_some_and(|p| p.as_bytes().ct_eq(input.proof.as_bytes()).into()) { return Err(ApiError::unauthorized()); }
        db.execute("UPDATE device_sessions SET key_verified=1 WHERE token_hash=? AND device_id=?", params![credential,id])?;
        Ok(StatusCode::NO_CONTENT)
    }).await
}
