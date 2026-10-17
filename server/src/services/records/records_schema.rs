use dog_core::DogError;
use serde::Deserialize;
use serde_json::Value;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordInput {
    pub name: String,
}
pub fn input(value: Value) -> anyhow::Result<RecordInput> {
    let data: RecordInput = serde_json::from_value(value)
        .map_err(|_| DogError::bad_request("Only name may be supplied").into_anyhow())?;
    if data.name.trim().is_empty()
        || data.name.len() > 128
        || !data
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " .,_-()".contains(c))
    {
        return Err(DogError::bad_request(
            "Name must be 1-128 plain text characters (letters, numbers, spaces or .,_-())",
        )
        .into_anyhow());
    }
    Ok(data)
}
pub fn id(value: &str) -> anyhow::Result<String> {
    uuid::Uuid::parse_str(value)
        .map(|v| v.to_string())
        .map_err(|_| DogError::bad_request("Invalid record ID").into_anyhow())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_authority_fields_and_query_injection() {
        for field in ["tenant", "company_domain", "owner", "email", "id", "query"] {
            let mut v = serde_json::json!({"name":"Test"});
            v[field] = Value::String("forged".into());
            assert!(input(v).is_err());
        }
        assert!(input(serde_json::json!({"name":"x\"; delete $u;"})).is_err());
        assert!(id("x\"; delete $u;").is_err());
    }
}
