use super::*;
use serde_json::json;
#[test]
fn accepts_complete_queries_not_authentication_claims_or_operation_ids() {
    assert!(validate_payload(Some(
        &json!({"query":"match $x isa resource; fetch {'id':$x.resource_id};"})
    ))
    .is_ok());
    for body in [
        json!({"operation":"resources"}),
        json!({"query":"", "actor":"alice"}),
        json!({"query":12}),
        json!({"query":"   "}),
    ] {
        assert!(validate_payload(Some(&body)).is_err());
    }
}
