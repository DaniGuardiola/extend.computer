fn main() {
    if std::env::var_os("CARGO_FEATURE_DEV_IDENTITY").is_some() {
        assert_eq!(
            std::env::var("PROFILE").as_deref(),
            Ok("debug"),
            "dev-identity is forbidden outside debug builds"
        );
    }
}
