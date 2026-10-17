pub use crate::services::records::records_schema::{id, input};
use dog_core::DogError;
use serde::Deserialize;
use serde_json::Value;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    name: String,
    project_id: String,
}
pub fn create(value: Value) -> anyhow::Result<(String, String)> {
    let data: CreateInput = serde_json::from_value(value)
        .map_err(|_| DogError::bad_request("Supply name and project_id only").into_anyhow())?;
    let project = id(&data.project_id)?;
    let name = input(serde_json::json!({"name": data.name}))?.name;
    Ok((name, project))
}
