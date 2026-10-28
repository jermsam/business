use super::*;
#[test]
fn mercury_uses_our_durable_email_delivery() {
    assert_eq!(
        provider_notice_kind("mercury", "open", 100, 99, 7),
        Some("invoice".into())
    );
    assert_eq!(
        provider_notice_kind("mercury", "open", 100, 100, 7),
        Some("reminder-0".into())
    );
    assert_eq!(
        provider_notice_kind("mercury", "paid", 100, 101, 7),
        Some("receipt".into())
    );
    assert_eq!(
        provider_notice_kind("stripe", "open", 100, 99, 7),
        Some("invoice".into())
    );
}
#[test]
fn reminder_and_receipt_rules() {
    assert_eq!(notice_kind("paid", 0, 100, 7), Some("receipt".into()));
    assert!(notice_kind("processing", 0, 100, 7).is_none());
    assert_eq!(notice_kind("open", 100, 99, 7), Some("invoice".into()));
    assert_eq!(notice_kind("open", 100, 100, 7), Some("reminder-0".into()));
    assert!(notice_kind("open", 0, 86400 * 29, 7).is_none());
}
