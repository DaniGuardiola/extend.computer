//! Opt-in, bounded timing-only trace. No coordinates, keys, identities, or contents.
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
struct Inner {
    start: Instant,
    path: PathBuf,
    rows: Mutex<Vec<(&'static str, u64, u128, u128)>>,
}
#[derive(Clone, Default)]
pub struct Trace(Option<Arc<Inner>>);
impl Trace {
    pub fn from_env() -> Self {
        Self(std::env::var_os("EXTEND_COMPUTER_TIMING_PATH").map(|path| {
            Arc::new(Inner {
                start: Instant::now(),
                path: path.into(),
                rows: Mutex::new(Vec::new()),
            })
        }))
    }
    pub fn record(&self, event: &'static str, id: u64, value: u128) {
        if let Some(inner) = &self.0 {
            let mut rows = inner.rows.lock().unwrap();
            if rows.len() < 50_000 {
                rows.push((event, id, inner.start.elapsed().as_micros(), value));
            }
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let result = (|| -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = std::io::BufWriter::new(options.open(&self.path)?);
            writeln!(file, "event,id,elapsed_us,value")?;
            for (event, id, elapsed, value) in self.rows.get_mut().unwrap() {
                writeln!(file, "{event},{id},{elapsed},{value}")?;
            }
            file.flush()
        })();
        if let Err(error) = result {
            eprintln!("timing trace could not be saved: {error}");
        }
    }
}
