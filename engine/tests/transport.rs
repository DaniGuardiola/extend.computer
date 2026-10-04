use extend_computer_agent::wire::{self, Channel};
use snow::{Builder, TransportState};
use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

fn transports() -> (TransportState, TransportState) {
    let params = "Noise_NN_25519_ChaChaPoly_SHA256";
    let mut a = Builder::new(params.parse().unwrap())
        .build_initiator()
        .unwrap();
    let mut b = Builder::new(params.parse().unwrap())
        .build_responder()
        .unwrap();
    let mut out = [0; 256];
    let mut decoded = [0; 256];
    let n = a.write_message(&[], &mut out).unwrap();
    b.read_message(&out[..n], &mut decoded).unwrap();
    let n = b.write_message(&[], &mut out).unwrap();
    a.read_message(&out[..n], &mut decoded).unwrap();
    (
        a.into_transport_mode().unwrap(),
        b.into_transport_mode().unwrap(),
    )
}

fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let b = listener.accept().unwrap().0;
    (a, b)
}

#[test]
fn encrypted_frame_replay_is_rejected() {
    let (mut sender, receiver) = transports();
    let (mut a, b) = sockets();
    let mut ciphertext = [0; 256];
    let n = sender.write_message(b"7", &mut ciphertext).unwrap();
    wire::write_frame(&mut a, &ciphertext[..n]).unwrap();
    wire::write_frame(&mut a, &ciphertext[..n]).unwrap();
    let mut channel = Channel::new(b, receiver);
    assert_eq!(channel.receive::<u32>().unwrap(), 7);
    assert!(channel.receive::<u32>().is_err());
}

#[test]
fn encrypted_frame_tampering_is_rejected() {
    let (mut sender, receiver) = transports();
    let (mut a, b) = sockets();
    let mut ciphertext = [0; 256];
    let n = sender.write_message(b"7", &mut ciphertext).unwrap();
    ciphertext[0] ^= 1;
    wire::write_frame(&mut a, &ciphertext[..n]).unwrap();
    let error = Channel::new(b, receiver).receive::<u32>().unwrap_err();
    assert_eq!(
        error.downcast_ref::<extend_computer_agent::error::EngineError>(),
        Some(&extend_computer_agent::error::EngineError::AuthenticationFailed)
    );
}

#[test]
fn slow_drip_does_not_extend_absolute_read_deadline() {
    let (mut a, mut b) = sockets();
    let task = thread::spawn(move || {
        for byte in [0, 5, 1, 2, 3, 4, 5] {
            if a.write_all(&[byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    });
    let start = Instant::now();
    assert!(wire::read_frame_until(&mut b, start + Duration::from_millis(100)).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(b);
    task.join().unwrap();
}
