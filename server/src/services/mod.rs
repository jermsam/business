pub mod types;
use crate::typedb::TypeDBState;
use anyhow::Result;
use dog_core::DogApp;
use serde_json::Value;
use std::sync::Arc;
pub use types::*;



pub struct BusinessService {

}

pub async fn configure(app: &DogApp<Value, BusinessParams>) -> Result<BusinessService> {
     let _state: Arc<TypeDBState> = app
        .get("typedb")
        .expect("TypeDBState should be initialised before configure() is called");

    Ok(BusinessService {
       
    })
}