//! Account-private relay for opaque encrypted engine bytes.
use super::*;
use axum::extract::{
    ws::{Message, WebSocket, WebSocketUpgrade},
    Query,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

#[derive(Clone)]
struct Auth {
    account: String,
    device: String,
    fingerprint: String,
    credential: String,
}
#[derive(Clone)]
struct Endpoint {
    id: String,
    auth: Auth,
    send: mpsc::Sender<Message>,
}
struct Tunnel {
    sender: Endpoint,
    receiver: Option<Endpoint>,
    target: String,
    created: Instant,
}
#[derive(Default)]
pub(super) struct Registry {
    controls: HashMap<(String, String), Endpoint>,
    tunnels: HashMap<String, Tunnel>,
    traffic: HashMap<String, (Instant, usize, usize)>,
}
#[derive(Deserialize)]
pub(super) struct Parameters {
    device: String,
    peer: Option<String>,
    kind: Option<String>,
    channel: Option<String>,
}
fn authenticate(db: &Connection, credential: &str, device: &str) -> Result<Auth, ApiError> {
    db.query_row("SELECT d.account_id,d.fingerprint FROM device_sessions ds JOIN devices d ON d.id=ds.device_id JOIN sessions s ON s.token_hash=ds.session_hash JOIN accounts a ON a.id=s.account_id WHERE ds.token_hash=? AND d.id=? AND ds.key_verified=1 AND s.expires_at>? AND s.mfa_version=a.mfa_version", params![credential,device,now()], |r| Ok(Auth { account:r.get(0)?,fingerprint:r.get(1)?,device:device.into(),credential:credential.into() })).optional()?.ok_or_else(ApiError::unauthorized)
}
async fn valid(server: &Arc<Server>, auth: &Auth) -> bool {
    let auth = auth.clone();
    server
        .clone()
        .work(move |db| authenticate(db, &auth.credential, &auth.device))
        .await
        .is_ok()
}
async fn upgrade(
    server: Arc<Server>,
    headers: HeaderMap,
    params: Parameters,
    ws: WebSocketUpgrade,
    role: &'static str,
) -> Result<Response, ApiError> {
    let credential = bearer(&headers)?;
    let device = params.device.clone();
    let auth = server
        .clone()
        .work(move |db| authenticate(db, &credential, &device))
        .await?;
    let endpoint_id = random_token();
    let (send, receive) = mpsc::channel(64);
    let endpoint = Endpoint {
        id: endpoint_id.clone(),
        auth: auth.clone(),
        send,
    };
    let channel;
    {
        let mut registry = server.relay.lock().map_err(|_| ApiError::internal())?;
        match role {
            "connect" => {
                channel = String::new();
                if registry.controls.len() >= 10000 {
                    return Err(ApiError::limited());
                }
                if let Some(old) = registry.controls.insert(
                    (auth.account.clone(), auth.device.clone()),
                    endpoint.clone(),
                ) {
                    let _ = old.send.try_send(Message::Close(None));
                }
                let _ = endpoint
                    .send
                    .try_send(Message::Text("{\"type\":\"ready\"}".into()));
            }
            "tunnel" => {
                let target = params
                    .peer
                    .ok_or_else(|| ApiError::bad("Missing target device"))?;
                let kind = params
                    .kind
                    .ok_or_else(|| ApiError::bad("Missing connection kind"))?;
                if target == auth.device || !matches!(kind.as_str(), "presence" | "control") {
                    return Err(ApiError::bad("Invalid relay target"));
                }
                let control = registry
                    .controls
                    .get(&(auth.account.clone(), target.clone()))
                    .cloned()
                    .ok_or_else(|| ApiError::bad("Device offline"))?;
                if registry
                    .tunnels
                    .values()
                    .filter(|t| t.sender.auth.account == auth.account)
                    .count()
                    >= 4
                {
                    return Err(ApiError::limited());
                }
                channel = random_token();
                control.send.try_send(Message::Text(json!({"type":"offer","channel":channel,"peer":auth.fingerprint,"kind":kind}).to_string().into())).map_err(|_|ApiError::limited())?;
                registry.tunnels.insert(
                    channel.clone(),
                    Tunnel {
                        sender: endpoint.clone(),
                        receiver: None,
                        target,
                        created: Instant::now(),
                    },
                );
            }
            "accept" => {
                channel = params
                    .channel
                    .ok_or_else(|| ApiError::bad("Missing channel"))?;
                let tunnel = registry
                    .tunnels
                    .get_mut(&channel)
                    .ok_or_else(|| ApiError::bad("Channel unavailable"))?;
                if tunnel.target != auth.device
                    || tunnel.sender.auth.account != auth.account
                    || tunnel.receiver.is_some()
                    || tunnel.created.elapsed() > Duration::from_secs(30)
                {
                    return Err(ApiError::unauthorized());
                }
                tunnel.receiver = Some(endpoint.clone());
                tunnel
                    .sender
                    .send
                    .try_send(Message::Text("{\"type\":\"ready\"}".into()))
                    .map_err(|_| ApiError::limited())?;
                let _ = endpoint
                    .send
                    .try_send(Message::Text("{\"type\":\"ready\"}".into()));
            }
            _ => return Err(ApiError::bad("Invalid relay action")),
        }
    }
    Ok(ws
        .max_message_size(65536)
        .max_frame_size(65536)
        .on_upgrade(move |socket| run(server, socket, endpoint, receive, role, channel)))
}
macro_rules! route {
    ($name:ident,$role:literal) => {
        pub(super) async fn $name(
            State(server): State<Arc<Server>>,
            headers: HeaderMap,
            Query(params): Query<Parameters>,
            ws: WebSocketUpgrade,
        ) -> Result<Response, ApiError> {
            upgrade(server, headers, params, ws, $role).await
        }
    };
}
route!(connect, "connect");
route!(tunnel, "tunnel");
route!(accept, "accept");

async fn run(
    server: Arc<Server>,
    socket: WebSocket,
    endpoint: Endpoint,
    mut receive: mpsc::Receiver<Message>,
    role: &str,
    channel: String,
) {
    let (mut sink, mut stream) = socket.split();
    let mut check = tokio::time::interval(Duration::from_secs(5));
    let mut idle = Instant::now();
    loop {
        tokio::select! {
            _=check.tick()=>{
                if !valid(&server,&endpoint.auth).await { break; }
                if role=="connect" && idle.elapsed()>Duration::from_secs(60) { break; }
                if role!="connect" {
                    let expired={let registry=server.relay.lock().unwrap(); registry.tunnels.get(&channel).is_none_or(|t| t.created.elapsed()>Duration::from_secs(3600) || (t.receiver.is_none() && t.created.elapsed()>Duration::from_secs(30)))};
                    if expired { break; }
                }
            }
            message=receive.recv()=>{
                let Some(message)=message else { break };
                let close=matches!(message,Message::Close(_));
                if sink.send(message).await.is_err() || close { break; }
            }
            message=stream.next()=>{
                let Some(Ok(message))=message else { break };
                idle=Instant::now();
                match message {
                    Message::Text(text) if role=="connect" && text=="ping"=>{ if sink.send(Message::Text("pong".into())).await.is_err() { break; } }
                    Message::Ping(data)=>{ if sink.send(Message::Pong(data)).await.is_err() { break; } }
                    Message::Pong(_)=>{},
                    Message::Binary(bytes) if role!="connect" && !bytes.is_empty()=>{
                        let destination={
                            let mut registry=server.relay.lock().unwrap();
                            let bucket=registry.traffic.entry(endpoint.auth.account.clone()).or_insert((Instant::now(),0,0));
                            if bucket.0.elapsed()>Duration::from_secs(1) { *bucket=(Instant::now(),0,0); }
                            bucket.1+=bytes.len();bucket.2+=1;
                            if bucket.1>2*1024*1024 || bucket.2>2000 { None } else {
                                registry.tunnels.get(&channel).and_then(|t| if role=="tunnel" { t.receiver.clone() } else { Some(t.sender.clone()) })
                            }
                        };
                        let Some(destination)=destination else { break; };
                        if destination.send.try_send(Message::Binary(bytes)).is_err() { break; }
                    }
                    _=>break,
                }
            }
        }
    }
    {
        let mut registry = server.relay.lock().unwrap();
        if role == "connect" {
            let key = (endpoint.auth.account.clone(), endpoint.auth.device.clone());
            if registry
                .controls
                .get(&key)
                .is_some_and(|e| e.id == endpoint.id)
            {
                registry.controls.remove(&key);
            }
        } else if let Some(tunnel) = registry.tunnels.remove(&channel) {
            let _ = tunnel.sender.send.try_send(Message::Close(None));
            if let Some(receiver) = tunnel.receiver {
                let _ = receiver.send.try_send(Message::Close(None));
            }
        }
        if !registry
            .controls
            .keys()
            .any(|(account, _)| *account == endpoint.auth.account)
            && !registry
                .tunnels
                .values()
                .any(|t| t.sender.auth.account == endpoint.auth.account)
        {
            registry.traffic.remove(&endpoint.auth.account);
        }
    }
    let _ = sink.close().await;
}
