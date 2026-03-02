use anyhow::Result;
use dog_core::DogApp;
use serde_json::Value;
use crate::services::BusinessParams;

pub fn channels(_app: &DogApp<Value, BusinessParams>) -> Result<()>{

    Ok(())
}