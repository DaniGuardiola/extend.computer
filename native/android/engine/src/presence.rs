//! Read-only desktop-compatible presence. Discovery never authenticates a route.
use anyhow::Result;
use extend_computer_agent::{
    identity::Identity,
    session::{self, Presence},
    trust::TrustStore,
};
use std::{
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
struct State {
    running: bool,
    socket: Option<TcpStream>,
}
pub struct Server {
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
    port: u16,
}
impl Server {
    pub fn start(
        identity: Arc<Identity>,
        trust: TrustStore,
        status: impl Fn() -> Presence + Send + 'static,
    ) -> Result<Self> {
        let listener =
            TcpListener::bind("0.0.0.0:48178").or_else(|_| TcpListener::bind("0.0.0.0:0"))?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(Mutex::new(State {
            running: true,
            socket: None,
        }));
        let active = state.clone();
        let worker = thread::Builder::new()
            .name("extend-presence".into())
            .spawn(move || {
                loop {
                    if !active.lock().unwrap().running {
                        break;
                    }
                    let socket = match listener.accept() {
                        Ok((socket, _)) => socket,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    };
                    if socket.set_nonblocking(false).is_err() {
                        continue;
                    }
                    {
                        let mut state = active.lock().unwrap();
                        if !state.running {
                            break;
                        }
                        let Ok(clone) = socket.try_clone() else {
                            continue;
                        };
                        state.socket = Some(clone);
                    }
                    let _ = session::serve_presence(socket, &identity, &trust, &status);
                    active.lock().unwrap().socket = None;
                }
                active.lock().unwrap().running = false;
            })?;
        Ok(Self {
            state,
            worker: Some(worker),
            port,
        })
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn stop(&mut self) {
        let wake = {
            let mut state = self.state.lock().unwrap();
            let wake = state.running && state.socket.is_none();
            state.running = false;
            if let Some(socket) = &state.socket {
                let _ = socket.shutdown(Shutdown::Both);
            }
            wake
        };
        if wake {
            let _ = TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], self.port)),
                Duration::from_millis(100),
            );
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_presence_repairs_port_unknown_and_wrong_pin_fail() {
        let files = tempfile::tempdir().unwrap();
        let identity = Arc::new(Identity::generate());
        let remote = Identity::generate();
        let trust = TrustStore::open(files.path()).unwrap();
        trust.remember(&remote.fingerprint()).unwrap();
        let mut server = Server::start(identity.clone(), trust, || Presence {
            receiving: true,
            port: 52341,
            name: "Test Tablet".into(),
        })
        .unwrap();
        let address = ("127.0.0.1", server.port());
        let status = session::query_presence(
            TcpStream::connect(address).unwrap(),
            &remote,
            &identity.fingerprint(),
        )
        .unwrap()
        .unwrap();
        assert!(status.receiving);
        assert_eq!(status.port, 52341);
        assert_eq!(status.name, "Test Tablet");
        assert!(session::query_presence(
            TcpStream::connect(address).unwrap(),
            &Identity::generate(),
            &identity.fingerprint()
        )
        .unwrap()
        .is_none());
        assert!(session::query_presence(
            TcpStream::connect(address).unwrap(),
            &remote,
            &Identity::generate().fingerprint()
        )
        .is_err());
        server.stop();
        assert!(TcpStream::connect(address).is_err());
    }
}
