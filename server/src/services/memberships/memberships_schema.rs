use dog_core::DogError;
use serde::Deserialize;
use serde_json::Value;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub person_id: String,
    pub role: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub role: String,
    pub state: String,
}
pub fn create(v: Value) -> anyhow::Result<Input> {
    let mut i: Input = serde_json::from_value(v)
        .map_err(|_| DogError::bad_request("Supply person_id and role").into_anyhow())?;
    i.person_id = super::super::records::records_schema::id(&i.person_id)?;
    check(&i.role, "active")?;
    Ok(i)
}
pub fn change(v: Value) -> anyhow::Result<Change> {
    let i: Change = serde_json::from_value(v)
        .map_err(|_| DogError::bad_request("Supply role and state").into_anyhow())?;
    check(&i.role, &i.state)?;
    Ok(i)
}
fn check(role: &str, state: &str) -> anyhow::Result<()> {
    if !matches!(role, "owner" | "member") || !matches!(state, "active" | "suspended") {
        return Err(DogError::bad_request("Invalid membership role/state").into_anyhow());
    }
    Ok(())
}
