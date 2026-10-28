use super::*;
#[test]
fn money_is_exact_and_payment_links_are_trusted() {
    assert_eq!(decimal_cents(&json!("12.50")).unwrap(), 1250);
    for value in [
        json!("0.001"),
        json!("-1"),
        json!("1e3"),
        json!("9223372036854775807"),
    ] {
        assert!(decimal_cents(&value).is_err());
    }
    assert!(trusted_url(
        "https://invoice.stripe.com/i/test_fixture",
        "invoice.stripe.com"
    )
    .is_ok());
    for url in [
        "http://invoice.stripe.com/i/x",
        "https://invoice.stripe.com.evil.example/i/x",
        "https://user@invoice.stripe.com/i/x",
        "https://invoice.stripe.com:444/i/x",
    ] {
        assert!(trusted_url(url, "invoice.stripe.com").is_err());
    }
}
#[test]
fn pending_bank_payment_and_unknown_states_never_look_payable() {
    assert_eq!(
        status(
            Provider::Stripe,
            &json!({"status":"open","business_payment_processing":true})
        )
        .unwrap(),
        "processing"
    );
    assert!(status(Provider::Stripe, &json!({"status":"unexpected"})).is_err());
    assert!(safe_id("in_a/../in_b").is_err());
}
