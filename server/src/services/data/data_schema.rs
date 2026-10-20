//! Transport validation only; never a TypeQL registry, matcher or rewriter.
use anyhow::Result;
use dog_core::DogError;
use serde::Deserialize;
use serde_json::Value;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    query: String,
}
pub fn payload(data: Option<&Value>) -> Result<()> {
    let request: QueryRequest = serde_json::from_value(data.cloned().unwrap_or(Value::Null))
        .map_err(|_| {
            DogError::bad_request("Send only a query string in the request body").into_anyhow()
        })?;
    if request.query.trim().is_empty() {
        return Err(DogError::bad_request("Query must not be empty").into_anyhow());
    }
    Ok(())
}
