use super::*;
#[test]
fn raw_body_timestamp_and_rotating_signatures() {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
    mac.update(b"1000.{\"id\":1}");
    let sig = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let header = format!("t=1000,v1={},v1={sig}", "0".repeat(64));
    assert!(verify_signature("secret", &header, b"{\"id\":1}", 1001).is_ok());
    assert!(verify_signature("secret", &header, b"{ \"id\":1}", 1001).is_err());
    assert!(verify_signature("secret", &header, b"{\"id\":1}", 1301).is_err());
    assert!(verify_signature("secret", &(header + ",t=1000"), b"{\"id\":1}", 1001).is_err());
}
