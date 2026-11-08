//! Help for someone who hits a problem: a GitHub issue prefilled with only the
//! app version and OS, and a short diagnostic summary (`runtime::diagnostics`)
//! that they can read before sharing it. Nothing is sent automatically.
use url::Url;

const NEW_ISSUE: &str = "https://github.com/DaniGuardiola/extend.computer/issues/new";

/// What this copy of the app runs on.
pub struct System {
    pub version: &'static str,
    pub development: bool,
    pub os: String,
    pub arch: &'static str,
}

pub fn system() -> System {
    // The OS can't change under a running app, and Windows asks a process.
    static OS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    System {
        version: env!("CARGO_PKG_VERSION"),
        development: cfg!(feature = "dev-identity"),
        os: OS.get_or_init(crate::platform::os_version).clone(),
        arch: match std::env::consts::ARCH {
            "aarch64" => "arm64",
            arch => arch,
        },
    }
}

impl System {
    /// "0.3.1", or "0.3.1 (development)" for development builds.
    pub fn app_version(&self) -> String {
        if self.development {
            format!("{} (development)", self.version)
        } else {
            self.version.into()
        }
    }
}

/// A new GitHub issue whose body holds the app version and OS, nothing else.
pub fn issue_url(system: &System) -> String {
    let mut url = Url::parse(NEW_ISSUE).expect("valid issue URL");
    url.query_pairs_mut().append_pair(
        "body",
        &format!(
            "\n\nApp version: {}\nOS: {}\n",
            system.app_version(),
            system.os
        ),
    );
    url.into()
}

#[tauri::command]
pub async fn report_problem() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| open::that(issue_url(&system())))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| {
            String::from("Couldn’t open your browser. Report the problem at github.com/DaniGuardiola/extend.computer/issues.")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_carries_only_the_version_and_os() {
        let system = System {
            version: "0.4.0",
            development: false,
            os: "macOS 15.5".into(),
            arch: "arm64",
        };
        let url = Url::parse(&issue_url(&system)).unwrap();
        assert_eq!(
            url.as_str().split('?').next(),
            Some("https://github.com/DaniGuardiola/extend.computer/issues/new")
        );
        let pairs: Vec<_> = url.query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            [(
                "body".to_owned(),
                "\n\nApp version: 0.4.0\nOS: macOS 15.5\n".to_owned()
            )]
        );
        let development = System {
            development: true,
            ..system
        };
        assert!(issue_url(&development).contains("0.4.0+%28development%29"));
    }

    #[test]
    fn this_system_names_its_os_and_architecture() {
        let system = system();
        assert_eq!(system.version, env!("CARGO_PKG_VERSION"));
        #[cfg(target_os = "macos")]
        assert!(system.os.starts_with("macOS 1") || system.os.starts_with("macOS 2"));
        #[cfg(windows)]
        assert!(system.os.starts_with("Windows"));
        assert!(["arm64", "x86_64"].contains(&system.arch));
    }
}
