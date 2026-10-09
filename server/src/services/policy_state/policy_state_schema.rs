use dog_core::DogError;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub state: String,
}
pub fn parse(v: serde_json::Value) -> anyhow::Result<Change> {
    let c: Change = serde_json::from_value(v)
        .map_err(|_| DogError::bad_request("Supply state only").into_anyhow())?;
    if !matches!(c.state.as_str(), "active" | "suspended") {
        return Err(DogError::bad_request("Invalid state").into_anyhow());
    }
    Ok(c)
}
pub fn id(s: &str) -> anyhow::Result<&str> {
    if s.is_empty()
        || s.len() > 256
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_:".contains(&c))
    {
        return Err(DogError::bad_request("Invalid policy ID").into_anyhow());
    }
    Ok(s)
}
