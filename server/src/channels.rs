use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::DogAppBuilder;
use serde_json::Value;

pub fn channels(_app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
    Ok(())
}
