use super::*;
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
fn secret() -> String {
    let mut bytes = [0u8; 20];
    OsRng.fill_bytes(&mut bytes);
    base32(&bytes)
}
fn base32(bytes: &[u8]) -> String {
    let (mut bits, mut value) = (0u32, 0u32);
    let mut out = String::new();
    for &b in bytes {
        value = (value << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((value >> bits) & 31) as usize] as char)
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((value << (5 - bits)) & 31) as usize] as char)
    }
    out
}
fn decode(secret: &str) -> Result<Vec<u8>, ApiError> {
    let (mut bits, mut value) = (0u32, 0u32);
    let mut out = Vec::new();
    for c in secret.bytes() {
        let n = ALPHABET
            .iter()
            .position(|&v| v == c)
            .ok_or_else(ApiError::internal)?;
        value = (value << 5) | n as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((value >> bits) & 255) as u8)
        }
    }
    Ok(out)
}
pub(crate) fn otp(secret: &str, step: i64, digits: u32) -> Result<String, ApiError> {
    let mut mac =
        <Hmac<Sha1> as Mac>::new_from_slice(&decode(secret)?).map_err(|_| ApiError::internal())?;
    mac.update(&(step as u64).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[19] & 15) as usize;
    let n = u32::from_be_bytes(hash[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    Ok(format!(
        "{:0width$}",
        n % 10u32.pow(digits),
        width = digits as usize
    ))
}
fn matching(secret: &str, code: &str, last: i64) -> Result<i64, ApiError> {
    if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(ApiError::unauthorized());
    }
    let current = now() / 30;
    for step in [current, current - 1, current + 1] {
        if step > last
            && step >= 0
            && bool::from(otp(secret, step, 6)?.as_bytes().ct_eq(code.as_bytes()))
        {
            return Ok(step);
        }
    }
    Err(ApiError::unauthorized())
}
fn seal(secret: &str, key: &str, account: &str) -> Result<String, ApiError> {
    let bytes = hex::decode(key).map_err(|_| ApiError::internal())?;
    let cipher = Aes256Gcm::new_from_slice(&bytes).map_err(|_| ApiError::internal())?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret.as_bytes(),
                aad: account.as_bytes(),
            },
        )
        .map_err(|_| ApiError::internal())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&encrypted[encrypted.len() - 16..]);
    out.extend_from_slice(&encrypted[..encrypted.len() - 16]);
    Ok(URL_SAFE_NO_PAD.encode(out))
}
fn open(value: &str, key: &str, account: &str) -> Result<String, ApiError> {
    let bytes = hex::decode(key).map_err(|_| ApiError::internal())?;
    let cipher = Aes256Gcm::new_from_slice(&bytes).map_err(|_| ApiError::internal())?;
    let value = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ApiError::internal())?;
    if value.len() < 28 {
        return Err(ApiError::internal());
    }
    let mut encrypted = value[28..].to_vec();
    encrypted.extend_from_slice(&value[12..28]);
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&value[..12]),
            Payload {
                msg: &encrypted,
                aad: account.as_bytes(),
            },
        )
        .map_err(|_| ApiError::internal())?;
    String::from_utf8(plain).map_err(|_| ApiError::internal())
}

pub(crate) struct Ticket {
    pub id: String,
    pub account: String,
    pub password: String,
    pub version: i64,
    pub purpose: String,
    pub session: Option<String>,
}
pub(crate) fn ticket(db: &Connection, raw: &str) -> Result<Ticket, ApiError> {
    if raw.len() != 64 || !raw.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::unauthorized());
    }
    db.query_row("SELECT t.id,t.account_id,t.password_version,t.version,t.purpose,t.session_hash FROM mfa_tickets t JOIN accounts a ON a.id=t.account_id WHERE t.id=? AND t.expires_at>? AND t.attempts<5 AND a.password_hash=t.password_version AND a.mfa_version=t.version",params![token_hash(raw),now()],|r|Ok(Ticket{id:r.get(0)?,account:r.get(1)?,password:r.get(2)?,version:r.get(3)?,purpose:r.get(4)?,session:r.get(5)?})).optional()?.ok_or_else(ApiError::unauthorized)
}
pub(crate) fn attempt(db: &Connection, ticket: &Ticket) -> Result<(), ApiError> {
    if db.execute(
        "UPDATE mfa_tickets SET attempts=attempts+1 WHERE id=? AND attempts<5 AND expires_at>?",
        params![ticket.id, now()],
    )? == 0
    {
        return Err(ApiError::unauthorized());
    }
    let count:i64=db.query_row("INSERT INTO mfa_attempts VALUES (?,1,?) ON CONFLICT(account_id) DO UPDATE SET count=CASE WHEN until<=? THEN 1 ELSE count+1 END,until=CASE WHEN until<=? THEN excluded.until ELSE until END RETURNING count",params![ticket.account,now()+60,now(),now()],|r|r.get(0))?;
    if count > 10 {
        return Err(ApiError::limited());
    }
    Ok(())
}
pub(crate) fn pending(
    db: &Connection,
    account: &str,
    purpose: &str,
    session: Option<&str>,
) -> Result<Value, ApiError> {
    db.execute("DELETE FROM mfa_tickets WHERE expires_at<=?", [now()])?;
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM mfa_tickets WHERE account_id=?",
        [account],
        |r| r.get(0),
    )?;
    if count >= 20 {
        return Err(ApiError::limited());
    }
    let raw = random_token();
    db.execute("INSERT INTO mfa_tickets(id,account_id,password_version,version,purpose,session_hash,expires_at) SELECT ?,id,password_hash,mfa_version,?,?,? FROM accounts WHERE id=?",params![token_hash(&raw),purpose,session,now()+300,account])?;
    let (totp, keys, _) = factors(db, account)?;
    Ok(json!({"mfa_required":true,"ticket":raw,"totp":totp,"keys":keys>0,"recovery":true}))
}
fn factors(db: &Connection, account: &str) -> Result<(bool, i64, i64), ApiError> {
    Ok((
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM totp_factors WHERE account_id=?)",
            [account],
            |r| r.get(0),
        )?,
        db.query_row(
            "SELECT COUNT(*) FROM passkeys WHERE account_id=?",
            [account],
            |r| r.get(0),
        )?,
        db.query_row(
            "SELECT COUNT(*) FROM recovery_codes WHERE account_id=?",
            [account],
            |r| r.get(0),
        )?,
    ))
}
pub(crate) fn elevated(db: &Connection, session: &str) -> Result<String, ApiError> {
    db.query_row("SELECT s.account_id FROM sessions s JOIN accounts a ON a.id=s.account_id WHERE s.token_hash=? AND s.expires_at>? AND s.elevated_until>? AND s.mfa_version=a.mfa_version",params![session,now(),now()],|r|r.get(0)).optional()?.ok_or(ApiError(StatusCode::FORBIDDEN,"Confirm your password and second factor before changing security settings"))
}
pub(crate) fn finish(db: &Connection, ticket: Ticket) -> Result<Value, ApiError> {
    if db.execute("DELETE FROM mfa_tickets WHERE id=? AND expires_at>? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND password_hash=? AND mfa_version=?)",params![ticket.id,now(),ticket.account,ticket.password,ticket.version])?==0{return Err(ApiError::unauthorized())}
    if ticket.purpose == "manage" {
        if db.execute("UPDATE sessions SET elevated_until=? WHERE token_hash=? AND account_id=? AND expires_at>? AND mfa_version=?",params![now()+300,ticket.session,ticket.account,now(),ticket.version])?==0{return Err(ApiError::unauthorized())}
        Ok(json!({"success":true}))
    } else {
        let email: String = db.query_row(
            "SELECT email FROM accounts WHERE id=?",
            [&ticket.account],
            |r| r.get(0),
        )?;
        issue_session(db, &ticket.account, &email)
    }
}
pub(crate) fn change(
    db: &Connection,
    session: &str,
    enabled: bool,
    codes: bool,
) -> Result<Value, ApiError> {
    let account = elevated(db, session)?;
    let version = i64::from_be_bytes({
        let mut v = [0u8; 8];
        OsRng.fill_bytes(&mut v[2..]);
        v
    });
    db.execute(
        "UPDATE accounts SET mfa_enabled=?,mfa_version=? WHERE id=?",
        params![enabled, version, account],
    )?;
    db.execute(
        "DELETE FROM sessions WHERE account_id=? AND token_hash<>?",
        params![account, session],
    )?;
    db.execute(
        "UPDATE sessions SET mfa_version=?,elevated_until=0 WHERE token_hash=?",
        params![version, session],
    )?;
    db.execute("DELETE FROM mfa_tickets WHERE account_id=?", [&account])?;
    if codes || !enabled {
        db.execute("DELETE FROM recovery_codes WHERE account_id=?", [&account])?;
    }
    let mut recovery = Vec::new();
    if enabled && codes {
        for _ in 0..10 {
            let mut bytes = [0u8; 16];
            OsRng.fill_bytes(&mut bytes);
            let raw = hex::encode(bytes);
            db.execute(
                "INSERT INTO recovery_codes VALUES (?,?)",
                params![account, token_hash(&raw)],
            )?;
            recovery.push(
                raw.as_bytes()
                    .chunks(8)
                    .map(|b| std::str::from_utf8(b).unwrap())
                    .collect::<Vec<_>>()
                    .join("-"),
            );
        }
    }
    Ok(json!({"success":true,"recovery_codes":recovery}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    pub ticket: String,
    pub code: String,
}
pub async fn verify(
    State(server): State<Arc<Server>>,
    Json(input): Json<Code>,
) -> Result<Json<Value>, ApiError> {
    let key = server.config.mfa_encryption_key.clone();
    server
        .work(move |db| {
            let t = ticket(db, &input.ticket)?;
            attempt(db, &t)?;
            let tx = db.transaction()?;
            if input.code.len() == 6 && input.code.bytes().all(|c| c.is_ascii_digit()) {
                let (encrypted, last): (String, i64) = tx
                    .query_row(
                        "SELECT secret,last_step FROM totp_factors WHERE account_id=?",
                        [&t.account],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?
                    .ok_or_else(ApiError::unauthorized)?;
                let secret = open(
                    &encrypted,
                    key.as_deref().ok_or_else(ApiError::unauthorized)?,
                    &t.account,
                )?;
                let step = matching(&secret, &input.code, last)?;
                tx.execute(
                    "UPDATE totp_factors SET last_step=? WHERE account_id=?",
                    params![step, t.account],
                )?;
            } else {
                let raw = input.code.replace(['-', ' '], "").to_lowercase();
                if raw.len() != 32 || !raw.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return Err(ApiError::unauthorized());
                }
                if tx.execute(
                    "DELETE FROM recovery_codes WHERE account_id=? AND code_hash=?",
                    params![t.account, token_hash(&raw)],
                )? == 0
                {
                    return Err(ApiError::unauthorized());
                }
            }
            let response = finish(&tx, t)?;
            tx.commit()?;
            Ok(Json(response))
        })
        .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Password {
    password: String,
}
pub async fn reauth(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<Password>,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    let lookup = session.clone();
    let (account, hash, version) = server
        .clone()
        .work(move |db| {
            let account = owner(db, &lookup)?;
            let (hash, version): (String, i64) = db.query_row(
                "SELECT password_hash,mfa_version FROM accounts WHERE id=?",
                [&account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok((account, hash, version))
        })
        .await?;
    let check = hash.clone();
    let permit = server
        .password_workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::limited())?;
    let valid = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        PasswordHash::new(&check).is_ok_and(|h| {
            Argon2::default()
                .verify_password(input.password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .map_err(|_| ApiError::internal())?;
    if !valid {
        return Err(ApiError::unauthorized());
    }
    server
        .work(move |db| {
            if owner(db, &session)? != account {
                return Err(ApiError::unauthorized());
            }
            let (current, current_version, enabled): (String, i64, bool) = db.query_row(
                "SELECT password_hash,mfa_version,mfa_enabled FROM accounts WHERE id=?",
                [&account],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if current != hash || current_version != version {
                return Err(ApiError::unauthorized());
            }
            if enabled {
                pending(db, &account, "manage", Some(&session))
            } else {
                db.execute(
                    "UPDATE sessions SET elevated_until=? WHERE token_hash=?",
                    params![now() + 300, session],
                )?;
                Ok(json!({"success":true}))
            }
        })
        .await
        .map(Json)
}
pub async fn info(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    let available = server.config.mfa_encryption_key.is_some();
    server.work(move|db|{let account=owner(db,&session)?;let enabled:bool=db.query_row("SELECT mfa_enabled FROM accounts WHERE id=?",[&account],|r|r.get(0))?;let(totp,keys,codes)=factors(db,&account)?;Ok(json!({"enabled":enabled,"totp":totp,"keys":keys,"recovery_codes":codes,"totp_available":available}))}).await.map(Json)
}
pub async fn setup(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    let key = server.config.mfa_encryption_key.clone().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Set EXTEND_MFA_ENCRYPTION_KEY to enable authenticator setup",
    ))?;
    server.work(move|db|{let account=elevated(db,&session)?;let(email,version):(String,i64)=db.query_row("SELECT email,mfa_version FROM accounts WHERE id=?",[&account],|r|Ok((r.get(0)?,r.get(1)?)))?;let secret=secret();db.execute("INSERT OR REPLACE INTO totp_setup VALUES (?,?,?,?,?)",params![session,account,seal(&secret,&key,&account)?,version,now()+300])?;let label=format!("extend.computer:{email}").bytes().map(|b|format!("%{b:02X}")).collect::<String>();Ok(json!({"secret":secret,"uri":format!("otpauth://totp/{label}?secret={secret}&issuer=extend.computer&algorithm=SHA1&digits=6&period=30")}))}).await.map(Json)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    code: String,
}
pub async fn confirm(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<Confirm>,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    let key = server
        .config
        .mfa_encryption_key
        .clone()
        .ok_or_else(ApiError::internal)?;
    server.work(move|db|{let account=elevated(db,&session)?;let encrypted:String=db.query_row("SELECT t.secret FROM totp_setup t JOIN accounts a ON a.id=t.account_id WHERE t.session_hash=? AND t.version=a.mfa_version AND t.expires_at>?",params![session,now()],|r|r.get(0)).optional()?.ok_or(ApiError::bad("Setup expired"))?;let step=matching(&open(&encrypted,&key,&account)?,&input.code,-1)?;let tx=db.transaction()?;tx.execute("INSERT OR REPLACE INTO totp_factors VALUES (?,?,?)",params![account,encrypted,step])?;tx.execute("DELETE FROM totp_setup WHERE session_hash=?",[&session])?;let response=change(&tx,&session,true,true)?;tx.commit()?;Ok(response)}).await.map(Json)
}
pub async fn enable(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    settings(server, headers, "enable").await
}
pub async fn disable(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    settings(server, headers, "disable").await
}
pub async fn recovery(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    settings(server, headers, "recovery").await
}
pub async fn remove_totp(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    settings(server, headers, "remove").await
}
async fn settings(
    server: Arc<Server>,
    headers: HeaderMap,
    kind: &'static str,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    server.work(move|db|{let account=elevated(db,&session)?;let(enabled,keys):(bool,i64)=db.query_row("SELECT mfa_enabled,(SELECT COUNT(*) FROM passkeys WHERE account_id=accounts.id) FROM accounts WHERE id=?",[&account],|r|Ok((r.get(0)?,r.get(1)?)))?;let(totp,_,_)=factors(db,&account)?;if kind=="enable"&&!totp&&keys==0{return Err(ApiError::bad("Add a factor first"))}
if kind=="recovery"&&!enabled{return Err(ApiError::bad("Enable two-factor authentication first"))}
if kind=="remove"&&enabled&&keys==0{return Err(ApiError::bad("Add another factor or turn off two-factor authentication first"))}let tx=db.transaction()?;if kind=="disable"||kind=="remove"{tx.execute("DELETE FROM totp_factors WHERE account_id=?",[&account])?;tx.execute("DELETE FROM totp_setup WHERE account_id=?",[&account])?;}let response=change(&tx,&session,kind!="disable"&&(enabled||kind=="enable"),kind!="remove")?;tx.commit()?;Ok(response)}).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_vectors_and_encryption() {
        let secret = base32(b"12345678901234567890");
        for (time, expected) in [
            (59, "94287082"),
            (1111111109, "07081804"),
            (20000000000, "65353130"),
        ] {
            assert_eq!(otp(&secret, time / 30, 8).unwrap(), expected)
        }
        let key = "12".repeat(32);
        let encrypted = seal(&secret, &key, "a").unwrap();
        assert_eq!(open(&encrypted, &key, "a").unwrap(), secret);
        assert!(open(&encrypted, &key, "b").is_err());
        assert!(open(&encrypted, &"13".repeat(32), "a").is_err());
    }
}
