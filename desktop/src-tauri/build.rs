fn main() {
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    if std::env::var("TAURI_CONFIG")
        .unwrap_or_default()
        .contains("computer.extend.desktop.development")
    {
        assert!(
            std::env::var("PROFILE").as_deref() == Ok("debug")
                && std::env::var_os("CARGO_FEATURE_DEV_IDENTITY").is_some(),
            "development bundle identity requires a debug dev-identity build"
        );
    }
    tauri_build::build()
}
