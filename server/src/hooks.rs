use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::DogAppBuilder;
use serde_json::Value;

pub fn global_hook(app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
    // No global policy hook: strict authentication and scope binding belong
    // to their resource services, so provider proxies remain independent.
    let _ = app;
    Ok(())
}
