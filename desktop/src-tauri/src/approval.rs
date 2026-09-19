use serde::Serialize;
use std::{
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
pub struct Request {
    pub id: u64,
    pub kind: String,
    pub peer: String,
    pub symbols: Option<[u8; 8]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    Deny,
    Remember,
}
struct Pending {
    request: Request,
    reply: mpsc::Sender<Answer>,
}
#[derive(Default)]
pub struct Approvals {
    pending: Mutex<Option<Pending>>,
    sequence: Mutex<u64>,
}
impl Approvals {
    pub fn current(&self) -> Option<Request> {
        self.pending
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.request.clone())
    }
    pub fn answer(&self, id: u64, answer: Answer) -> anyhow::Result<()> {
        let mut pending = self.pending.lock().unwrap();
        anyhow::ensure!(
            pending.as_ref().is_some_and(|p| p.request.id == id),
            "This request has expired."
        );
        let p = pending.take().unwrap();
        let _ = p.reply.send(answer);
        Ok(())
    }
    pub fn cancel(&self) {
        if let Some(p) = self.pending.lock().unwrap().take() {
            let _ = p.reply.send(Answer::Deny);
        }
    }
    pub fn ask(&self, kind: &str, peer: &str, cancelled: impl Fn() -> bool) -> Answer {
        self.ask_verified(kind, peer, None, cancelled)
    }
    pub fn ask_verified(
        &self,
        kind: &str,
        peer: &str,
        symbols: Option<[u8; 8]>,
        cancelled: impl Fn() -> bool,
    ) -> Answer {
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut sequence = self.sequence.lock().unwrap();
            *sequence += 1;
            *sequence
        };
        if cancelled() {
            return Answer::Deny;
        }
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.is_some() {
                return Answer::Deny;
            }
            *pending = Some(Pending {
                request: Request {
                    id,
                    kind: kind.into(),
                    peer: peer.into(),
                    symbols,
                },
                reply: tx,
            });
        }
        let deadline = Instant::now() + Duration::from_secs(90);
        let answer = loop {
            if cancelled() || Instant::now() >= deadline {
                break Answer::Deny;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => break answer,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Answer::Deny,
                Err(_) => {}
            }
        };
        let mut pending = self.pending.lock().unwrap();
        if pending.as_ref().is_some_and(|p| p.request.id == id) {
            pending.take();
        }
        answer
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_answers_cannot_authorize_a_later_request() {
        let a = std::sync::Arc::new(Approvals::default());
        let b = a.clone();
        let waiter = std::thread::spawn(move || b.ask("pair", "peer", || false));
        while a.current().is_none() {
            std::thread::yield_now();
        }
        let id = a.current().unwrap().id;
        assert!(a.answer(id + 1, Answer::Remember).is_err());
        a.cancel();
        assert_eq!(waiter.join().unwrap(), Answer::Deny);
        assert!(a.answer(id, Answer::Remember).is_err());
    }
}
