//! Remember explicit receiving choices separately from the running listener.
use super::*;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;

#[derive(Default, Deserialize, Serialize)]
struct Preferences {
    #[serde(default)]
    receiving: bool,
}
pub(super) fn load(root: &Path) -> Result<bool> {
    match std::fs::read(root.join("preferences.json")) {
        Ok(bytes) => {
            ensure!(
                bytes.len() < 64 * 1024,
                "extend.computer preferences are too large."
            );
            Ok(serde_json::from_slice::<Preferences>(&bytes)?.receiving)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}
pub(super) fn save(root: &Path, receiving: bool) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(&serde_json::to_vec(&Preferences { receiving })?)?;
    file.as_file().sync_all()?;
    file.persist(root.join("preferences.json"))?;
    Ok(())
}
impl Desktop {
    pub fn restore_receiving(self: &Arc<Self>) {
        let app = self.clone();
        std::thread::spawn(move || {
            app.restore_receiving_at(48177);
        });
    }
    pub(super) fn restore_receiving_at(self: &Arc<Self>, port: u16) {
        // Serialize with explicit toggles so a late startup cannot undo Turn off.
        let mut preference = self.receiving_preference.lock().unwrap();
        if !*preference || self.closing.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = self.receive_with_preference(false, port, &mut preference) {
            // Missing permission is handled when the user explicitly enables receiving.
            if error.is::<incoming::ReceivingPermissionRequired>() {
                return;
            }
            self.notify(format!(
                "Incoming connections couldn’t start. {}",
                friendly_error(&error)
            ));
        }
    }
}
