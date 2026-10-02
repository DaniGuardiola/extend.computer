//! WebAuthn state never leaves the server. In-flight challenges expire on restart.
use super::*;
use webauthn_rs::prelude::*;

pub struct Passkeys {
    webauthn: Webauthn,
    pub origin: String,
    rp_id: String,
    challenges: Mutex<HashMap<String, (Instant, Ceremony)>>,
}

enum Ceremony {
    Register {
        account: String,
        session: String,
        state: PasskeyRegistration,
        version: i64,
    },
    Login(DiscoverableAuthentication),
    RegisterFactor {
        account: String,
        session: String,
        version: i64,
        state: SecurityKeyRegistration,
    },
    Mfa {
        ticket: String,
        state: SecurityKeyAuthentication,
    },
}

impl Passkeys {
    pub fn new(origin: &str) -> anyhow::Result<Self> {
        let url = Url::parse(origin)?;
        let rp_id = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("origin needs a hostname"))?
            .to_owned();
        anyhow::ensure!(
            url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none(),
            "EXTEND_ORIGIN must be an origin, without a path or credentials"
        );
        anyhow::ensure!(
            url.scheme() == "https" || (url.scheme() == "http" && rp_id == "localhost"),
            "passkeys require HTTPS or http://localhost"
        );
        let webauthn = WebauthnBuilder::new(&rp_id, &url)?
            .rp_name("extend.computer")
            // Only second-factor SecurityKey ceremonies allow touch-only keys.
            // Passwordless Passkey ceremonies still require user verification.
            .danger_set_user_presence_only_security_keys(true)
            .build()?;
        Ok(Self {
            webauthn,
            origin: url.origin().ascii_serialization(),
            rp_id,
            challenges: Mutex::new(HashMap::new()),
        })
    }

    fn check_origin(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) != Some(self.origin.as_str()) {
            return Err(ApiError(StatusCode::FORBIDDEN, "Invalid origin"));
        }
        Ok(())
    }

    fn start(&self, state: Ceremony, mut options: Value) -> Result<Response, ApiError> {
        let mut challenges = self.challenges.lock().map_err(|_| ApiError::internal())?;
        challenges.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(300));
        if challenges.len() >= 1024 {
            return Err(ApiError::limited());
        }
        let id = random_token();
        challenges.insert(token_hash(&id), (Instant::now(), state));
        // Match SimpleWebAuthn's flat options shape. The modal browser flow chooses mediation.
        options = options["publicKey"].take();
        let mut response = Json(options).into_response();
        response
            .headers_mut()
            .insert("x-extend-challenge", id.parse().unwrap());
        response.headers_mut().insert(
            header::SET_COOKIE,
            format!(
                "extend_challenge={id}; Path=/v1; HttpOnly; SameSite=Strict; Max-Age=300{}",
                if self.origin.starts_with("https:") {
                    "; Secure"
                } else {
                    ""
                }
            )
            .parse()
            .unwrap(),
        );
        Ok(response)
    }

    fn consume(&self, headers: &HeaderMap) -> Result<Ceremony, ApiError> {
        self.check_origin(headers)?;
        let id = headers
            .get("x-extend-challenge")
            .and_then(|h| h.to_str().ok())
            .or_else(|| {
                headers
                    .get(header::COOKIE)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|cookies| {
                        cookies
                            .split(';')
                            .find_map(|c| c.trim().strip_prefix("extend_challenge="))
                    })
            })
            .ok_or_else(ApiError::unauthorized)?;
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::unauthorized());
        }
        let (at, state) = self
            .challenges
            .lock()
            .map_err(|_| ApiError::internal())?
            .remove(&token_hash(id))
            .ok_or_else(ApiError::unauthorized)?;
        if at.elapsed() >= Duration::from_secs(300) {
            return Err(ApiError::unauthorized());
        }
        Ok(state)
    }
}

fn service(server: &Server) -> Result<&Passkeys, ApiError> {
    server.passkeys.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Set EXTEND_ORIGIN to enable passkeys",
    ))
}

fn user_id(account: &str) -> Result<Uuid, ApiError> {
    let bytes = hex::decode(account).map_err(|_| ApiError::internal())?;
    Uuid::from_slice(bytes.get(..16).ok_or_else(ApiError::internal)?)
        .map_err(|_| ApiError::internal())
}

pub async fn register_options(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Response, ApiError> {
    if input.get("factor").and_then(Value::as_bool) == Some(true) {
        return factor_options(server, headers).await;
    }
    let service = service(&server)?;
    service.check_origin(&headers)?;
    let session = bearer(&headers)?;
    let lookup = session.clone();
    let rp = service.rp_id.clone();
    let (account, email, keys, version) = server
        .clone()
        .work(move |db| {
            let account = owner(db, &lookup)?;
            let (enabled, version): (bool, i64) = db.query_row(
                "SELECT mfa_enabled,mfa_version FROM accounts WHERE id=?",
                [&account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if enabled {
                crate::mfa::elevated(db, &lookup)?;
            }
            let email: String =
                db.query_row("SELECT email FROM accounts WHERE id = ?", [&account], |r| {
                    r.get(0)
                })?;
            let total: i64 = db.query_row(
                "SELECT COUNT(*) FROM passkeys WHERE account_id = ?",
                [&account],
                |r| r.get(0),
            )?;
            if total >= 10 {
                return Err(ApiError::bad("Passkey limit reached"));
            }
            let mut query =
                db.prepare("SELECT credential FROM passkeys WHERE account_id = ? AND rp_id = ?")?;
            let stored = query
                .query_map(params![account, rp], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let keys = stored
                .into_iter()
                .map(|s| {
                    serde_json::from_str::<Passkey>(&s)
                        .map(|k| k.cred_id().clone())
                        .map_err(|_| ApiError::internal())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((account, email, keys, version))
        })
        .await?;
    let (options, state) = service
        .webauthn
        .start_passkey_registration(user_id(&account)?, &email, &email, Some(keys))
        .map_err(|_| ApiError::bad("Could not start passkey registration"))?;
    let mut options = serde_json::to_value(options).map_err(|_| ApiError::internal())?;
    options["publicKey"]["authenticatorSelection"]["residentKey"] = json!("required");
    options["publicKey"]["authenticatorSelection"]["requireResidentKey"] = json!(true);
    service.start(
        Ceremony::Register {
            account,
            session,
            state,
            version,
        },
        options,
    )
}

pub async fn register_verify(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<RegisterPublicKeyCredential>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let service = service(&server)?;
    let session = bearer(&headers)?;
    let ceremony = service.consume(&headers)?;
    if let Ceremony::RegisterFactor {
        account,
        session: original,
        version,
        state,
    } = ceremony
    {
        return finish_factor(server, account, original, version, state, session, input).await;
    }
    let Ceremony::Register {
        account,
        session: original,
        state,
        version,
    } = ceremony
    else {
        return Err(ApiError::unauthorized());
    };
    if session != original {
        return Err(ApiError::unauthorized());
    }
    let key = service
        .webauthn
        .finish_passkey_registration(&input, &state)
        .map_err(|_| ApiError::bad("Could not verify passkey"))?;
    let id = input.id;
    let serialized = serde_json::to_string(&key).map_err(|_| ApiError::internal())?;
    let counter = serde_json::to_value(&key).map_err(|_| ApiError::internal())?["cred"]["counter"]
        .as_u64()
        .ok_or_else(ApiError::internal)?;
    let rp = service.rp_id.clone();
    server
        .work(move |db| {
            if owner(db, &session)? != account {
                return Err(ApiError::unauthorized());
            }
            let(enabled,current):(bool,i64)=db.query_row("SELECT mfa_enabled,mfa_version FROM accounts WHERE id=?",[&account],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if current!=version{return Err(ApiError::unauthorized())}
            if enabled {crate::mfa::elevated(db,&session)?;}
            let count: i64 = db.query_row(
                "SELECT COUNT(*) FROM passkeys WHERE account_id = ?",
                [&account],
                |r| r.get(0),
            )?;
            if count >= 10 {
                return Err(ApiError::bad("Passkey limit reached"));
            }
            let tx=db.transaction()?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO passkeys(id,account_id,credential,counter,name,rp_id,created_at,purpose) VALUES (?, ?, ?, ?, ?, ?, ?, 'login')",
                params![
                    id,
                    account,
                    serialized,
                    counter as i64,
                    "Passkey",
                    rp,
                    now()
                ],
            )?;
            if inserted != 1 {
                return Err(ApiError::bad("Passkey already registered"));
            }
            if enabled {crate::mfa::change(&tx,&session,true,false)?;}
            tx.commit()?;
            Ok((StatusCode::CREATED, Json(json!({"success":true}))))
        })
        .await
}

pub async fn login_options(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let service = service(&server)?;
    service.check_origin(&headers)?;
    let (options, state) = service
        .webauthn
        .start_discoverable_authentication()
        .map_err(|_| ApiError::internal())?;
    service.start(
        Ceremony::Login(state),
        serde_json::to_value(options).map_err(|_| ApiError::internal())?,
    )
}

pub async fn login_verify(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<PublicKeyCredential>,
) -> Result<Json<Value>, ApiError> {
    let service = service(&server)?;
    let Ceremony::Login(state) = service.consume(&headers)? else {
        return Err(ApiError::unauthorized());
    };
    let (user, _) = service
        .webauthn
        .identify_discoverable_authentication(&input)
        .map_err(|_| ApiError::unauthorized())?;
    let id = input.id.clone();
    let rp = service.rp_id.clone();
    let webauthn = service.webauthn.clone();
    server.work(move |db| {
        // Verification and counter update share this database lock, preventing concurrent replay.
        let (account, stored, counter): (String, String, u32) = db.query_row("SELECT account_id, credential, counter FROM passkeys WHERE id = ? AND rp_id = ? AND purpose='login'", params![id, rp], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?.ok_or_else(ApiError::unauthorized)?;
        if user_id(&account)? != user { return Err(ApiError::unauthorized()); }
        let mut key: Passkey = serde_json::from_str(&stored).map_err(|_| ApiError::internal())?;
        let result = webauthn.finish_discoverable_authentication(&input, state, &[DiscoverableKey::from(&key)]).map_err(|_| ApiError::unauthorized())?;
        if (counter > 0 || result.counter() > 0) && result.counter() <= counter { return Err(ApiError::unauthorized()); }
        key.update_credential(&result);
        let tx = db.transaction()?;
        tx.execute("UPDATE passkeys SET credential = ?, counter = ? WHERE id = ?", params![serde_json::to_string(&key).map_err(|_| ApiError::internal())?, result.counter(), id])?;
        let email: String = tx.query_row("SELECT email FROM accounts WHERE id = ?", [&account], |r| r.get(0))?;
        let response = new_session(&tx, &account, &email)?;
        tx.commit()?;
        Ok(Json(response))
    }).await
}

pub async fn list(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let session = bearer(&headers)?;
    server.work(move |db| {
        let account = owner(db, &session)?;
        let mut query = db.prepare("SELECT id, name, rp_id, created_at,purpose FROM passkeys WHERE account_id = ? ORDER BY created_at")?;
        let keys = query.query_map([account], |r| Ok(json!({"id":r.get::<_,String>(0)?, "name":r.get::<_,String>(1)?, "rp_id":r.get::<_,String>(2)?, "created_at":r.get::<_,i64>(3)?,"purpose":r.get::<_,String>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
        Ok(Json(json!({"passkeys":keys})))
    }).await
}

pub async fn remove(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session = bearer(&headers)?;
    server
        .work(move |db| {
            let account = owner(db, &session)?;
            let enabled:bool=db.query_row("SELECT mfa_enabled FROM accounts WHERE id=?",[&account],|r|r.get(0))?;
            if enabled {
                crate::mfa::elevated(db,&session)?;
                let(keys,totp):(i64,bool)=db.query_row("SELECT (SELECT COUNT(*) FROM passkeys WHERE account_id=?),EXISTS(SELECT 1 FROM totp_factors WHERE account_id=?)",params![account,account],|r|Ok((r.get(0)?,r.get(1)?)))?;
                if keys<=1&&!totp{return Err(ApiError::bad("Add another factor or disable two-factor authentication first"))}
                let tx=db.transaction()?;
                if tx.execute("DELETE FROM passkeys WHERE id=? AND account_id=?",params![id,account])?==0{return Err(ApiError(StatusCode::NOT_FOUND,"Passkey not found"))}
                crate::mfa::change(&tx,&session,true,false)?;tx.commit()?;
                return Ok(StatusCode::NO_CONTENT)
            }
            if db.execute(
                "DELETE FROM passkeys WHERE id = ? AND account_id = ?",
                params![id, account],
            )? == 0
            {
                return Err(ApiError(StatusCode::NOT_FOUND, "Passkey not found"));
            }
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

async fn factor_options(server: Arc<Server>, headers: HeaderMap) -> Result<Response, ApiError> {
    let service = service(&server)?;
    service.check_origin(&headers)?;
    let session = bearer(&headers)?;
    let lookup = session.clone();
    let rp = service.rp_id.clone();
    let (account, email, version, exclude) = server
        .clone()
        .work(move |db| {
            let account = crate::mfa::elevated(db, &lookup)?;
            let (email, version): (String, i64) = db.query_row(
                "SELECT email,mfa_version FROM accounts WHERE id=?",
                [&account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let count: i64 = db.query_row(
                "SELECT COUNT(*) FROM passkeys WHERE account_id=?",
                [&account],
                |r| r.get(0),
            )?;
            if count >= 10 {
                return Err(ApiError::bad("Passkey limit reached"));
            }
            let mut query =
                db.prepare("SELECT credential FROM passkeys WHERE account_id=? AND rp_id=?")?;
            let values = query
                .query_map(params![account, rp], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let exclude = values
                .into_iter()
                .map(|s| {
                    serde_json::from_str::<SecurityKey>(&s)
                        .map(|k| k.cred_id().clone())
                        .map_err(|_| ApiError::internal())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((account, email, version, exclude))
        })
        .await?;
    let (options, state) = service
        .webauthn
        .start_securitykey_registration(
            user_id(&account)?,
            &email,
            &email,
            Some(exclude),
            None,
            None,
        )
        .map_err(|_| ApiError::bad("Could not start security key registration"))?;
    service.start(
        Ceremony::RegisterFactor {
            account,
            session,
            version,
            state,
        },
        serde_json::to_value(options).map_err(|_| ApiError::internal())?,
    )
}
async fn finish_factor(
    server: Arc<Server>,
    account: String,
    original: String,
    version: i64,
    state: SecurityKeyRegistration,
    session: String,
    input: RegisterPublicKeyCredential,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if original != session {
        return Err(ApiError::unauthorized());
    }
    let service = service(&server)?;
    let key = service
        .webauthn
        .finish_securitykey_registration(&input, &state)
        .map_err(|_| ApiError::bad("Could not verify security key"))?;
    let serialized = serde_json::to_string(&key).map_err(|_| ApiError::internal())?;
    let counter = serde_json::to_value(&key).map_err(|_| ApiError::internal())?["cred"]["counter"]
        .as_u64()
        .ok_or_else(ApiError::internal)?;
    let rp = service.rp_id.clone();
    server.work(move|db|{if crate::mfa::elevated(db,&session)?!=account{return Err(ApiError::unauthorized())}
 let(current,enabled):(i64,bool)=db.query_row("SELECT mfa_version,mfa_enabled FROM accounts WHERE id=?",[&account],|r|Ok((r.get(0)?,r.get(1)?)))?;if current!=version{return Err(ApiError::unauthorized())}
 let count:i64=db.query_row("SELECT COUNT(*) FROM passkeys WHERE account_id=?",[&account],|r|r.get(0))?;if count>=10{return Err(ApiError::bad("Passkey limit reached"))}
 let tx=db.transaction()?;tx.execute("INSERT INTO passkeys(id,account_id,credential,counter,name,rp_id,created_at,purpose) VALUES (?,?,?,?,?,?,?,'factor')",params![input.id,account,serialized,counter as i64,"Security key",rp,now()])?;
 if enabled{crate::mfa::change(&tx,&session,true,false)?;}tx.commit()?;Ok((StatusCode::CREATED,Json(json!({"success":true}))))}).await
}

pub async fn mfa_options(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<MfaOptions>,
) -> Result<Response, ApiError> {
    let service = service(&server)?;
    service.check_origin(&headers)?;
    let raw = input.ticket.clone();
    let rp = service.rp_id.clone();
    let keys = server
        .clone()
        .work(move |db| {
            let t = crate::mfa::ticket(db, &raw)?;
            let mut query =
                db.prepare("SELECT credential FROM passkeys WHERE account_id=? AND rp_id=?")?;
            let stored = query
                .query_map(params![t.account, rp], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            stored
                .into_iter()
                .map(|s| serde_json::from_str::<SecurityKey>(&s).map_err(|_| ApiError::internal()))
                .collect::<Result<Vec<_>, _>>()
        })
        .await?;
    if keys.is_empty() {
        return Err(ApiError::bad("No passkey or security key registered"));
    }
    let (options, state) = service
        .webauthn
        .start_securitykey_authentication(&keys)
        .map_err(|_| ApiError::internal())?;
    service.start(
        Ceremony::Mfa {
            ticket: input.ticket,
            state,
        },
        serde_json::to_value(options).map_err(|_| ApiError::internal())?,
    )
}
#[derive(Deserialize)]
pub struct MfaOptions {
    ticket: String,
}
#[derive(Deserialize)]
pub struct MfaCredential {
    ticket: String,
    #[serde(flatten)]
    credential: PublicKeyCredential,
}
pub async fn mfa_verify(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(input): Json<MfaCredential>,
) -> Result<Json<Value>, ApiError> {
    let service = service(&server)?;
    let Ceremony::Mfa {
        ticket: original,
        state,
    } = service.consume(&headers)?
    else {
        return Err(ApiError::unauthorized());
    };
    if original != input.ticket {
        return Err(ApiError::unauthorized());
    }
    let webauthn = service.webauthn.clone();
    let rp = service.rp_id.clone();
    server.work(move|db|{let t=crate::mfa::ticket(db,&input.ticket)?;crate::mfa::attempt(db,&t)?;
 let (stored,counter):(String,u32)=db.query_row("SELECT credential,counter FROM passkeys WHERE id=? AND account_id=? AND rp_id=?",params![input.credential.id,t.account,rp],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or_else(ApiError::unauthorized)?;
 // Passkey and SecurityKey both serialize a Credential. The original credential's UV policy is retained.
 let mut key:SecurityKey=serde_json::from_str(&stored).map_err(|_|ApiError::internal())?;
 let result=webauthn.finish_securitykey_authentication(&input.credential,&state).map_err(|_|ApiError::unauthorized())?;
 if (counter>0||result.counter()>0)&&result.counter()<=counter{return Err(ApiError::unauthorized())}key.update_credential(&result);
 let tx=db.transaction()?;tx.execute("UPDATE passkeys SET credential=?,counter=? WHERE id=?",params![serde_json::to_string(&key).map_err(|_|ApiError::internal())?,result.counter(),input.credential.id])?;
 let response=crate::mfa::finish(&tx,t)?;tx.commit()?;Ok(Json(response))}).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_requires_a_secure_exact_origin() {
        assert!(Passkeys::new("http://example.com").is_err());
        assert!(Passkeys::new("https://example.com/path").is_err());
        assert!(Passkeys::new("https://user:secret@example.com").is_err());
        assert!(Passkeys::new("http://localhost:8080").is_ok());
        assert!(Passkeys::new("https://example.com").is_ok());
    }

    #[test]
    fn expired_challenges_cannot_be_replayed() {
        let service = Passkeys::new("http://localhost:8080").unwrap();
        let (_, state) = service
            .webauthn
            .start_discoverable_authentication()
            .unwrap();
        let token = random_token();
        service.challenges.lock().unwrap().insert(
            token_hash(&token),
            (
                Instant::now() - Duration::from_secs(301),
                Ceremony::Login(state),
            ),
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "http://localhost:8080".parse().unwrap());
        headers.insert("x-extend-challenge", token.parse().unwrap());
        assert!(service.consume(&headers).is_err());
        assert!(service.consume(&headers).is_err());
        assert!(service.challenges.lock().unwrap().is_empty());
    }
}
