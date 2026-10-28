use super::*;
#[test]
fn target_and_owner_validation_reject_unsafe_inputs() {
    for name in [
        "",
        "postgres",
        "business_prod/other",
        "business_backend_test_not-a-uuid",
    ] {
        assert!(target(name).is_err());
    }
    assert!(target("business_dev").is_ok());
    let mut owner = Owner {
        email: "owner@example.invalid".into(),
        name: "Owner".into(),
        tenant_name: "Test".into(),
        password: "a-long-test-password".into(),
    };
    assert!(owner.validate().is_ok());
    owner.email = "bad@@example.com".into();
    assert!(owner.validate().is_err());
    owner.email = "owner@example.invalid".into();
    owner.password = "short".into();
    assert!(owner.validate().is_err());
}
