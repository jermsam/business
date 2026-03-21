use crate::services::types::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::hooks::{DogAfterHook, DogBeforeHook, HookContext};
use serde_json::Value;

pub struct BeforeRead;

#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for BeforeRead {
    async fn run(&self, _ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        // Validate subjects query parameters
        //         if let Some(data) = &_ctx.data {
        //             if let Some(query_match) = data.get("match") {
        //                 if let Some(match_str) = query_match.as_str() {
        //                     let targets_subject = match_str.contains("isa subject")
        //                         || match_str.contains("isa user")
        //                         || match_str.contains("isa group")
        //                         || match_str.contains("isa company");
        //
        //                     if !targets_subject {
        //                         return Err(anyhow::anyhow!("Query must target subject entities"));
        //                     }
        //                 }
        //             }
        //         }
        Ok(())
    }
}

pub struct AfterRead;

#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterRead {
    async fn run(&self, _ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        // Query completed
        Ok(())
    }
}

pub struct BeforeWrite;

#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for BeforeWrite {
    async fn run(&self, _ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        // Validate subjects data before writing
        if let Some(_data) = &_ctx.data {}
        Ok(())
    }
}

pub struct AfterWrite;

#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterWrite {
    async fn run(&self, _ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        // Write operation completed
        Ok(())
    }
}
