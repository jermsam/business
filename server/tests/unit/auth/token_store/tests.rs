use super::*;
#[test]
fn issuer_and_token_are_unambiguous() {
    assert_ne!(
        TypeDbTokenStore::key("ab", "c"),
        TypeDbTokenStore::key("a", "bc")
    );
}
