//! Named IOKit power assertions. Each shows in `pmset -g assertions` under the
//! app's process until it is dropped; macOS also ends it with the process.
use objc2::rc::Retained;
use objc2_foundation::NSString;
use std::ffi::c_void;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        kind: *const c_void,
        level: u32,
        name: *const c_void,
        id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
}
const LEVEL_ON: u32 = 255;
/// `kIOPMAssertionTypePreventUserIdleSystemSleep`: the Mac stays awake while
/// idle; the display may still sleep.
pub const PREVENT_IDLE_SYSTEM_SLEEP: &str = "PreventUserIdleSystemSleep";

pub struct Assertion(u32);
impl Assertion {
    /// `None` when macOS refuses it; a session then runs without it.
    pub fn new(kind: &str, name: &str) -> Option<Self> {
        // NSString is toll-free bridged with CFStringRef.
        let kind = NSString::from_str(kind);
        let name = NSString::from_str(name);
        let mut id = 0;
        let status = unsafe {
            IOPMAssertionCreateWithName(
                Retained::as_ptr(&kind).cast(),
                LEVEL_ON,
                Retained::as_ptr(&name).cast(),
                &mut id,
            )
        };
        if status != 0 {
            eprintln!("Could not keep this Mac awake: IOKit error {status:#x}");
            return None;
        }
        Some(Self(id))
    }
}
impl Drop for Assertion {
    fn drop(&mut self) {
        unsafe { IOPMAssertionRelease(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn listed(name: &str) -> bool {
        let output = std::process::Command::new("/usr/bin/pmset")
            .args(["-g", "assertions"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.contains(&format!("pid {}(", std::process::id()))
                && line.contains(&format!("{PREVENT_IDLE_SYSTEM_SLEEP} named: \"{name}\""))
        })
    }
    #[test]
    fn named_assertion_is_listed_until_dropped() {
        let name = format!("extend.computer test assertion {}", std::process::id());
        let assertion = Assertion::new(PREVENT_IDLE_SYSTEM_SLEEP, &name).unwrap();
        assert!(listed(&name));
        drop(assertion);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while listed(&name) {
            assert!(
                std::time::Instant::now() < deadline,
                "{name} was not released"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
