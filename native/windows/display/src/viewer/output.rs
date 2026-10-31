//! The viewer's stdout. A stopped parent can backpressure the pipe, so the
//! window never writes itself: it queues control messages and input intent
//! here, and one thread writes them.
use super::input::Outbox;
use extend_computer_agent::helper::{
    display::{HelperMessage, NativeEvent},
    packet::{write_packet, Kind},
};
use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Queue {
    control: VecDeque<(Kind, Vec<u8>)>,
    input: Outbox,
    writing: bool,
    closed: bool,
}

pub struct Writer {
    queue: Mutex<Queue>,
    changed: Condvar,
}
impl Writer {
    pub fn spawn(mut sink: impl Write + Send + 'static) -> Arc<Self> {
        let writer = Arc::new(Self {
            queue: Mutex::default(),
            changed: Condvar::new(),
        });
        let worker = writer.clone();
        std::thread::spawn(move || loop {
            let (kind, payload) = {
                let mut queue = worker.queue.lock().unwrap();
                let next = loop {
                    if let Some(next) = queue.control.pop_front() {
                        break next;
                    }
                    if let Some(bytes) = queue.input.pop() {
                        break (Kind::ViewerInput, bytes);
                    }
                    if queue.closed {
                        return;
                    }
                    queue = worker.changed.wait(queue).unwrap();
                };
                queue.writing = true;
                next
            };
            let written = write_packet(&mut sink, kind, &payload).and_then(|()| sink.flush());
            let mut queue = worker.queue.lock().unwrap();
            queue.writing = false;
            // The app stopped reading; nothing more can reach it.
            if written.is_err() {
                queue.closed = true;
                queue.control.clear();
                while queue.input.pop().is_some() {}
            }
            worker.changed.notify_all();
        });
        writer
    }

    pub fn packet(&self, kind: Kind, payload: Vec<u8>) {
        let mut queue = self.queue.lock().unwrap();
        if !queue.closed {
            queue.control.push_back((kind, payload));
            self.changed.notify_all();
        }
    }

    pub fn control(&self, message: &HelperMessage) {
        if let Ok(bytes) = message.to_bytes() {
            self.packet(Kind::Control, bytes);
        }
    }

    /// Queues input intent. Returns whether the queue overflowed, after which
    /// the caller drops its own held state as the focus loss tells the app.
    pub fn input(&self, events: Vec<NativeEvent>) -> bool {
        if events.is_empty() {
            return false;
        }
        let mut queue = self.queue.lock().unwrap();
        if queue.closed {
            return false;
        }
        let overflowed = queue.input.enqueue(events);
        self.changed.notify_all();
        overflowed
    }

    /// Waits until everything queued is written, up to `timeout`.
    pub fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut queue = self.queue.lock().unwrap();
        while !queue.closed
            && (queue.writing || !queue.control.is_empty() || !queue.input.is_empty())
        {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            queue = self.changed.wait_timeout(queue, left).unwrap().0;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extend_computer_agent::{
        display::input::{Command, Point},
        helper::packet::read_packet,
    };
    use std::sync::mpsc;

    /// A sink whose first write blocks until released.
    struct Gate {
        started: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        bytes: Arc<Mutex<Vec<u8>>>,
    }
    impl Write for Gate {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Some(started) = self.started.take() {
                started.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_blocked_pipe_never_blocks_the_window_and_overflow_ends_in_focus_loss() {
        let (started, wait_started) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = Writer::spawn(Gate {
            started: Some(started),
            release: released,
            bytes: bytes.clone(),
        });
        let focus = NativeEvent::Focus {
            active: true,
            revision: 1,
        };
        writer.input(vec![focus.clone()]);
        wait_started.recv_timeout(Duration::from_secs(2)).unwrap();
        let before = Instant::now();
        for _ in 0..100 {
            assert!(!writer.input(vec![NativeEvent::Input {
                command: Command::Move {
                    point: Point { x: 0.4, y: 0.5 },
                },
                revision: 1,
            }]));
        }
        let overflowed = (0..65).any(|_| writer.input(vec![focus.clone()]));
        assert!(overflowed && before.elapsed() < Duration::from_secs(1));
        writer.control(&HelperMessage::Closed);
        release.send(()).unwrap();
        assert!(writer.flush(Duration::from_secs(2)));
        let written = bytes.lock().unwrap().clone();
        let mut input = written.as_slice();
        let mut packets = Vec::new();
        while let Some(packet) = read_packet(&mut input).unwrap() {
            packets.push(packet);
        }
        assert!(packets.len() <= 4, "{packets:?}");
        assert_eq!(
            packets
                .iter()
                .filter(|(kind, _)| *kind == Kind::Control)
                .count(),
            1
        );
        assert_eq!(
            packets
                .iter()
                .rfind(|(kind, _)| *kind == Kind::ViewerInput)
                .unwrap()
                .1,
            br#"{"type":"focus","active":false,"revision":1}"#
        );
    }
}
