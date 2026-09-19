#![cfg(all(feature = "dev-identity", debug_assertions, unix))]
use extend_computer_agent::identity::Identity;
use std::os::unix::fs::PermissionsExt;
#[test]
fn stable_private_key_without_keychain() {
    let root = tempfile::tempdir().unwrap();
    let a = Identity::load_development(root.path()).unwrap();
    let b = Identity::load_development(root.path()).unwrap();
    assert_eq!(a.fingerprint(), b.fingerprint());
    let key = root.path().join("development-identity.key");
    assert_eq!(
        std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Identity::load_development(root.path()).is_err());
}
#[test]
fn corrupt_and_symlink_keys_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("development-identity.key");
    std::fs::write(&key, b"invalid").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Identity::load_development(root.path()).is_err());
    let second = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(&key, second.path().join("development-identity.key")).unwrap();
    assert!(Identity::load_development(second.path()).is_err());
}

#[test]
fn empty_existing_identity_is_not_replaced() {
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("development-identity.key");
    std::fs::write(&key, []).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Identity::load_development(root.path()).is_err());
    assert_eq!(std::fs::metadata(key).unwrap().len(), 0);
}
