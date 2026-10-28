//! Bounded privileged transactions, available only to server-owned billing templates.
use crate::{database::documents, typedb::TypeDBState};
use anyhow::{ensure, Result};
use serde_json::Value;
use std::time::Duration;
use typedb_driver::{TransactionOptions, TransactionType};
const DEADLINE: Duration = Duration::from_secs(30);
pub async fn read(state: &TypeDBState, query: &str) -> Result<Vec<Value>> {
    tokio::time::timeout(DEADLINE, async {
        let tx = state
            .driver
            .transaction_with_options(
                &state.database,
                TransactionType::Read,
                TransactionOptions::new().transaction_timeout(DEADLINE),
            )
            .await?;
        let result = documents(&tx, query).await;
        tx.close().await?;
        result
    })
    .await?
}
pub async fn write(state: &TypeDBState, query: &str) -> Result<Value> {
    let at = query
        .rfind("fetch ")
        .ok_or_else(|| anyhow::anyhow!("Missing write projection"))?;
    // Every template retains $t through select. Updating its revision makes concurrent
    // claims on the same merchant conflict, including phantom/absence checks.
    let query = format!(
        "{} update $t has revision \"{}\"; {}",
        &query[..at],
        uuid::Uuid::new_v4(),
        &query[at..]
    );
    tokio::time::timeout(DEADLINE, async {
        let tx = state
            .driver
            .transaction_with_options(
                &state.database,
                TransactionType::Write,
                TransactionOptions::new().transaction_timeout(DEADLINE),
            )
            .await?;
        let result = documents(&tx, &query).await;
        let rows = match result {
            Ok(rows) if rows.len() == 1 => rows,
            result => {
                tx.close().await?;
                let rows = result?;
                ensure!(
                    rows.len() == 1,
                    "Billing claim unavailable or ambiguous; nothing committed"
                );
                unreachable!()
            }
        };
        // Never retry commit: a lost acknowledgement can conceal a successful commit.
        tx.commit().await?;
        Ok(rows.into_iter().next().unwrap())
    })
    .await?
}
