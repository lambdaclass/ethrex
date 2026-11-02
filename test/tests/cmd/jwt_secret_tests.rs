use ethrex::utils::read_jwtsecret_file;
use std::fs;

#[cfg(unix)]
#[test]
fn a_generated_jwt_secret_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jwt.hex");

    let secret = read_jwtsecret_file(path.to_str().unwrap());

    assert_eq!(secret.len(), 32);
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "generated secret file has mode {mode:o}");
}

#[test]
fn an_existing_jwt_secret_is_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jwt.hex");
    fs::write(&path, format!("0x{}\n", "ab".repeat(32))).unwrap();

    let secret = read_jwtsecret_file(path.to_str().unwrap());

    assert_eq!(secret.as_ref(), [0xab; 32]);
}

#[test]
#[should_panic(expected = "JWT secret must be 32 bytes")]
fn a_short_jwt_secret_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jwt.hex");
    fs::write(&path, "abcd").unwrap();

    read_jwtsecret_file(path.to_str().unwrap());
}

#[test]
#[should_panic(expected = "JWT secret must be 32 bytes")]
fn an_empty_jwt_secret_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jwt.hex");
    fs::write(&path, "").unwrap();

    read_jwtsecret_file(path.to_str().unwrap());
}
