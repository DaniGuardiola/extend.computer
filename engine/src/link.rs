//! Best-effort physical link of a connection, for explaining routes to users.
//! It never affects trust or routing decisions.
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, TcpStream};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    Wifi,
    Ethernet,
    #[serde(other)]
    Other,
}

type Classifier = fn(IpAddr) -> Option<Link>;
static CLASSIFIER: Mutex<Option<Classifier>> = Mutex::new(None);

/// A platform adapter can classify local addresses through its own OS APIs.
pub fn set_classifier(classifier: Classifier) {
    *CLASSIFIER.lock().unwrap() = Some(classifier);
}

/// The link carrying this socket on the local side, when it can be told.
pub fn of(socket: &TcpStream) -> Option<Link> {
    let local = socket.local_addr().ok()?.ip();
    if local.is_loopback() {
        return None;
    }
    let classify = CLASSIFIER
        .lock()
        .unwrap()
        .unwrap_or(crate::platform::classify_link);
    classify(local)
}

#[cfg(test)]
mod tests {
    use super::Link;
    #[test]
    fn unknown_links_from_newer_peers_are_other() {
        assert_eq!(
            serde_json::from_str::<Link>(r#""cellular""#).unwrap(),
            Link::Other
        );
        assert_eq!(serde_json::to_string(&Link::Wifi).unwrap(), r#""wifi""#);
    }
}
