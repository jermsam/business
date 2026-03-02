use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::DogApp;
use serde_json::Value;

pub fn channels(_app: &DogApp<Value, BusinessParams>) -> Result<()> {
    Ok(())
}
