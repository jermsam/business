use super::*;
#[test]
fn admin_cannot_invent_cardholder_consent() {
    for p in [Policy::Customer, Policy::Automatic] {
        assert_eq!(effective(p, Mode::Automatic, false, true), Mode::Manual);
        assert_eq!(effective(p, Mode::Automatic, true, false), Mode::Manual);
    }
    assert_eq!(
        effective(Policy::Manual, Mode::Automatic, true, true),
        Mode::Manual
    );
    assert_eq!(
        effective(Policy::Customer, Mode::Automatic, true, true),
        Mode::Automatic
    );
}
#[test]
fn months_keep_anchor_and_once_ends() {
    let a = DateTime::parse_from_rfc3339("2027-01-31T15:00:00Z")
        .unwrap()
        .timestamp();
    assert_eq!(
        DateTime::from_timestamp(occurrence(a, Interval::Month, 1).unwrap().unwrap(), 0)
            .unwrap()
            .to_rfc3339(),
        "2027-02-28T15:00:00+00:00"
    );
    assert_eq!(
        DateTime::from_timestamp(occurrence(a, Interval::Month, 2).unwrap().unwrap(), 0)
            .unwrap()
            .to_rfc3339(),
        "2027-03-31T15:00:00+00:00"
    );
    assert_eq!(occurrence(a, Interval::Once, 1).unwrap(), None);
}
#[test]
fn customer_opt_out_wins_even_with_old_consent() {
    assert_eq!(
        effective(Policy::Automatic, Mode::Manual, true, true),
        Mode::Manual
    );
}
