//! Visual authentication of a completed Noise channel. Both fresh nonces are
//! committed before either is revealed, preventing adaptive SAS grinding.
use crate::{error::EngineError, wire::Channel};
use anyhow::{ensure, Result};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Message {
    Commit { value: [u8; 32] },
    Reveal { nonce: [u8; 32] },
    Confirm { matches: bool },
}
fn commitment(transcript: &[u8], initiator: bool, nonce: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"extend.computer/visual-pair/v1/commit");
    hash.update(transcript);
    hash.update([u8::from(initiator)]);
    hash.update(nonce);
    hash.finalize().into()
}
fn symbols(transcript: &[u8], initiator: &[u8; 32], responder: &[u8; 32]) -> [u8; 8] {
    let mut hash = Sha256::new();
    hash.update(b"extend.computer/visual-pair/v1/sas");
    hash.update(transcript);
    hash.update(initiator);
    hash.update(responder);
    let bytes = hash.finalize();
    // Eight independent six-bit indices: 48 bits, shown in a fixed order.
    std::array::from_fn(|i| bytes[i] & 63)
}
pub(crate) fn verify(
    channel: &mut Channel,
    transcript: &[u8],
    initiator: bool,
    mut approve: impl FnMut(&[u8; 8]) -> bool,
) -> Result<()> {
    let mut nonce = [0; 32];
    OsRng.fill_bytes(&mut nonce);
    channel.send(&Message::Commit {
        value: commitment(transcript, initiator, &nonce),
    })?;
    let Message::Commit { value } = channel.receive()? else {
        anyhow::bail!(EngineError::RequestRejected);
    };
    channel.send(&Message::Reveal { nonce })?;
    let Message::Reveal { nonce: remote } = channel.receive()? else {
        anyhow::bail!(EngineError::RequestRejected);
    };
    ensure!(
        value == commitment(transcript, !initiator, &remote),
        EngineError::AuthenticationFailed
    );
    let sas = if initiator {
        symbols(transcript, &nonce, &remote)
    } else {
        symbols(transcript, &remote, &nonce)
    };
    channel.consent_timeout()?;
    let accepted = approve(&sas);
    // Declining closes the socket without queuing a frame. A peer waiting on
    // its local UI can observe EOF immediately without consuming protocol data.
    ensure!(accepted, EngineError::LocalConsentDenied);
    channel.send(&Message::Confirm { matches: true })?;
    ensure!(
        matches!(channel.receive()?, Message::Confirm { matches: true }),
        EngineError::RemoteConsentDenied
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_binds_transcript_roles_and_both_nonces() {
        let a = [1; 32];
        let b = [2; 32];
        assert_ne!(commitment(b"one", true, &a), commitment(b"two", true, &a));
        assert_ne!(commitment(b"one", true, &a), commitment(b"one", false, &a));
        assert_ne!(symbols(b"one", &a, &b), symbols(b"two", &a, &b));
        assert_ne!(symbols(b"one", &a, &b), symbols(b"one", &b, &a));
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    #[test]
    fn altered_commitment_rejected_before_user_can_approve() {
        use std::net::{TcpListener, TcpStream};
        let mut a = snow::Builder::new("Noise_XX_25519_ChaChaPoly_SHA256".parse().unwrap())
            .local_private_key(&[3; 32])
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut b = snow::Builder::new("Noise_XX_25519_ChaChaPoly_SHA256".parse().unwrap())
            .local_private_key(&[4; 32])
            .unwrap()
            .build_responder()
            .unwrap();
        let mut packet = [0; 4096];
        let mut plain = [0; 4096];
        let n = a.write_message(&[], &mut packet).unwrap();
        b.read_message(&packet[..n], &mut plain).unwrap();
        let n = b.write_message(&[], &mut packet).unwrap();
        a.read_message(&packet[..n], &mut plain).unwrap();
        let n = a.write_message(&[], &mut packet).unwrap();
        b.read_message(&packet[..n], &mut plain).unwrap();
        let transcript = a.get_handshake_hash().to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut local = Channel::new(socket, a.into_transport_mode().unwrap());
        let mut remote = Channel::new(
            listener.accept().unwrap().0,
            b.into_transport_mode().unwrap(),
        );
        let t = std::thread::spawn(move || {
            let _: Message = remote.receive().unwrap();
            remote.send(&Message::Commit { value: [0; 32] }).unwrap();
            let _: Message = remote.receive().unwrap();
            remote.send(&Message::Reveal { nonce: [1; 32] }).unwrap();
        });
        let error = verify(&mut local, &transcript, true, |_| {
            panic!("tampered SAS reached approval")
        })
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<EngineError>(),
            Some(EngineError::AuthenticationFailed)
        ));
        t.join().unwrap();
    }
}
