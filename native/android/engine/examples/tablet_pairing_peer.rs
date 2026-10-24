//! Local interoperability fixture. Never uses a desktop user's real identity or trust.
use anyhow::Result;
use extend_computer_agent::{
    identity::Identity,
    pairing::PairingWindow,
    session::{self, Decision},
    trust::TrustStore,
};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{fs::OpenOptions, io::Write, net::TcpListener, time::Duration};
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("private fixture JSON path required");
    let advertise = std::env::args().nth(2).as_deref() == Some("--advertise");
    let scratch = tempfile::tempdir()?;
    let identity = Identity::generate();
    let trust = TrustStore::open(scratch.path())?;
    let listener = TcpListener::bind(if advertise {
        "0.0.0.0:0"
    } else {
        "127.0.0.1:0"
    })?;
    let _advertisement = if advertise {
        Some(
            extend_computer_agent::discovery::Advertisement::start_pairing(
                listener.local_addr()?.port(),
                "Discovery Test Mac",
                &identity.fingerprint(),
            )?,
        )
    } else {
        None
    };
    let mut window = Some(PairingWindow::new(Duration::from_secs(120)));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut output = options.open(path)?;
    write!(
        output,
        "{}",
        serde_json::json!({"address":listener.local_addr()?.to_string(),
        "code":window.as_ref().unwrap().code().unwrap(),"peer":identity.fingerprint()})
    )?;
    output.sync_all()?;
    println!("Ready: temporary authenticated peer, three test connections.");
    for _ in 0..3 {
        session::serve_connection(
            listener.accept()?.0,
            &identity,
            &trust,
            &mut window,
            |_, _| Decision::Remember,
        )?;
    }
    println!("PASS: Android paired and reconnected twice using shared engine.");
    Ok(())
}
