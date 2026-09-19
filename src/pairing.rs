use anyhow::{ensure, Result};
use rand::{rngs::OsRng, RngCore};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// One claim per pairing window, including failed attempts. Reopening requires local action.
pub struct PairingWindow {
    code: Option<Zeroizing<String>>,
    expires: Instant,
}

impl PairingWindow {
    pub fn new(ttl: Duration) -> Self {
        let mut random = [0u8; 8];
        OsRng.fill_bytes(&mut random);
        let encoded = hex::encode(random);
        let code = encoded
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("-");
        Self {
            code: Some(Zeroizing::new(code)),
            expires: Instant::now() + ttl,
        }
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref().map(String::as_str)
    }

    pub fn claim(&mut self) -> Result<(Zeroizing<String>, Instant)> {
        let code = self
            .code
            .take()
            .ok_or_else(|| anyhow::anyhow!("pairing window already consumed"))?;
        ensure!(Instant::now() < self.expires, "pairing window expired");
        Ok((code, self.expires))
    }
}
