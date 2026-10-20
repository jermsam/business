use super::*;
#[test]
fn rejects_insecure_startup() {
    let secret = "test-only-secret-with-more-than-32-bytes";
    assert!(validate_security(secret, "true", "false", "business_dev_app").is_ok());
    assert!(validate_security("dev-secret", "true", "false", "business_dev_app").is_err());
    assert!(validate_security(secret, "false", "false", "business_dev_app").is_err());
    assert!(validate_security(secret, "true", "true", "business_dev_app").is_err());
    assert!(validate_security(secret, "true", "false", "admin").is_err());
}
