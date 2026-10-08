//! Stable identity migration and database-coordinated authorization writes.
use anyhow::{ensure, Result};
use futures::TryStreamExt;
use serde_json::Value;
use typedb_driver::Transaction;

pub async fn documents(tx: &Transaction, query: &str) -> Result<Vec<Value>> {
    let mut stream = tx.query(query).await?.into_documents();
    let mut docs = Vec::new();
    while let Some(doc) = stream.try_next().await? {
        ensure!(
            docs.len() < 100_000,
            "Migration/result limit exceeded; use a staged migration"
        );
        docs.push(serde_json::from_str(&doc.into_json().to_string())?);
    }
    Ok(docs)
}

/// Only server-owned templates call this. Preserve `$t` through every projection.
/// Changing the same ownership forces overlapping tenant writes to conflict.
pub fn guard_write(query: &str) -> Result<String> {
    let position = query
        .rfind("fetch ")
        .ok_or_else(|| anyhow::anyhow!("Guarded writes require a result projection"))?;
    Ok(format!(
        "{} update $t has biz_revision \"{}\"; {}",
        &query[..position],
        uuid::Uuid::new_v4(),
        &query[position..]
    ))
}

/// A protected mutation must produce exactly one result before it can commit.
/// No retries: a failed commit acknowledgement can have an uncertain outcome.
pub(crate) async fn write_one(
    state: &crate::typedb::TypeDBState,
    query: &str,
) -> Result<Vec<Value>> {
    let query = guard_write(query)?;
    write_atomic_one(state, &query).await
}

/// Server-owned identity/recovery templates which coordinate on a user instead of a tenant.
pub(crate) async fn write_atomic_one(
    state: &crate::typedb::TypeDBState,
    query: &str,
) -> Result<Vec<Value>> {
    use dog_core::DogError;
    use std::time::Duration;
    use tokio::time::timeout;
    use typedb_driver::{TransactionOptions, TransactionType};
    const DEADLINE: Duration = Duration::from_secs(30);
    #[cfg(test)]
    let stage = std::time::Instant::now();
    let tx = timeout(
        DEADLINE,
        state.driver.transaction_with_options(
            &state.database,
            TransactionType::Write,
            TransactionOptions::new().transaction_timeout(DEADLINE),
        ),
    )
    .await
    .map_err(|_| DogError::unavailable("Database transaction could not start").into_anyhow())?
    .map_err(|_| DogError::unavailable("Database transaction could not start").into_anyhow())?;
    #[cfg(test)]
    trace_stage("write-open", stage);
    #[cfg(test)]
    let stage = std::time::Instant::now();
    let result = timeout(DEADLINE, async {
        let mut stream = tx.query(query).await?.into_documents();
        let mut rows = Vec::new();
        while let Some(doc) = stream.try_next().await? {
            if !rows.is_empty() {
                return Err(
                    DogError::conflict("Ambiguous mutation; no changes committed").into_anyhow(),
                );
            }
            rows.push(serde_json::from_str(&doc.into_json().to_string())?);
        }
        if rows.is_empty() {
            return Err(
                DogError::not_found("Resource unavailable or operation not allowed").into_anyhow(),
            );
        }
        Ok::<_, anyhow::Error>(rows)
    })
    .await;
    #[cfg(test)]
    trace_stage("write-query", stage);
    let rows = match result {
        Ok(Ok(rows)) => rows,
        failure => {
            // Closing discards buffered changes. The server deadline also bounds cleanup
            // if the client is cancelled or disconnected before close completes.
            let _ = timeout(Duration::from_secs(2), tx.close()).await;
            return Err(match failure {
                Ok(Err(error)) if error.downcast_ref::<DogError>().is_some() => error,
                _ => DogError::unavailable("Mutation did not commit").into_anyhow(),
            });
        }
    };
    #[cfg(test)]
    let stage = std::time::Instant::now();
    let outcome = timeout(DEADLINE, tx.commit()).await;
    #[cfg(test)]
    trace_stage("write-commit", stage);
    match outcome {
        Ok(Ok(())) => Ok(rows),
        _ => Err(DogError::unavailable(
            "Commit was not confirmed; reconcile resource state before retrying",
        )
        .into_anyhow()),
    }
}

/// Run only in an explicit migration or synthetic-fixture setup transaction.
/// A schema transaction prevents concurrent provisioning during the real backfill.
pub async fn backfill(tx: &Transaction) -> Result<()> {
    for kind in ["user", "company"] {
        let rows = documents(
            tx,
            &format!(
                "match $x isa {kind}; not {{ $x has biz_id $id; }}; fetch {{ \"iid\": iid($x) }};"
            ),
        )
        .await?;
        for chunk in rows.chunks(100) {
            let mut matching = String::from("match ");
            let mut inserts = String::from("insert ");
            for (i, row) in chunk.iter().enumerate() {
                let iid = row["iid"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing IID"))?;
                ensure!(
                    iid.starts_with("0x") && iid[2..].bytes().all(|b| b.is_ascii_hexdigit()),
                    "Invalid IID"
                );
                matching += &format!("$x{i} isa {kind}, iid {iid}; ");
                inserts += &format!("$x{i} has biz_id \"{}\"; ", uuid::Uuid::new_v4());
            }
            tx.query(matching + &inserts).await?;
        }
    }
    for kind in ["user", "company"] {
        let rows = documents(
            tx,
            &format!("match $x isa {kind}, has biz_id $id; fetch {{ \"id\": $id }};"),
        )
        .await?;
        for row in rows {
            let id = field(&row, "id")?;
            ensure!(
                uuid::Uuid::parse_str(id)?.to_string() == id,
                "Identity IDs must be canonical UUIDs; resolve before migration"
            );
        }
    }
    tx.query("match $t isa company; not { $t has biz_revision $r; }; insert $t has biz_revision \"initial\";").await?;
    let rows = documents(tx, "match $m isa biz_membership, links (tenant: $t, person: $u); $t has biz_id $tid; $u has biz_id $uid; fetch { \"iid\": iid($m), \"tenant\": $tid, \"person\": $uid };").await?;
    let mut pairs = std::collections::HashSet::new();
    for row in &rows {
        let key = pair(field(row, "tenant")?, field(row, "person")?)?;
        ensure!(
            pairs.insert(key),
            "Duplicate membership pair: resolve explicitly before migration; no changes committed"
        );
    }
    for chunk in rows.chunks(100) {
        let mut query = String::from("match ");
        let mut update = String::from("update ");
        for (i, row) in chunk.iter().enumerate() {
            query += &format!("$m{i} isa biz_membership, iid {}; ", field(row, "iid")?);
            update += &format!(
                "$m{i} has biz_id \"{}\"; ",
                pair(field(row, "tenant")?, field(row, "person")?)?
            );
        }
        tx.query(query + &update).await?;
    }
    tx.query(r#"match
      $legacy isa tenant_membership, links (tenant: $t, member: $u); not { $legacy has biz_migrated true; }; $u isa user, has biz_id $uid; $t has biz_id $tid;
      let $key = $tid + ":" + $uid;
      not { $m isa biz_membership, has biz_id == $key; };
      select $t, $u, $key; distinct;
      insert (tenant: $t, person: $u) isa biz_membership, has biz_id == $key,
      has biz_state "active", has biz_role "member", has biz_start 1970-01-01T00:00:00;"#).await?;
    tx.query("match $legacy isa tenant_membership, links (member: $u); $u isa user; not { $legacy has biz_migrated true; }; insert $legacy has biz_migrated true;").await?;
    tx.query("match $m isa biz_membership; not { $m has biz_role $role; }; insert $m has biz_role \"member\";").await?;
    let orphan = documents(
        tx,
        r#"match $r isa business_record, has email $email, has company_domain $domain;
      not { $r has biz_person_id $existing; };
      not { $u isa user, has email $email; $t isa company, has company_domain $domain; };
      fetch { "id": $r.record_id };"#,
    )
    .await?;
    ensure!(
        orphan.is_empty(),
        "Unresolvable private ownership; no migration changes committed"
    );
    tx.query(r#"match $r isa business_record, has email $email, has company_domain $domain;
      not { $r has biz_person_id $existing; };
      $u isa user, has email $email, has biz_id $uid; $t isa company, has company_domain $domain, has biz_id $tid;
      insert $r has biz_person_id == $uid, has biz_tenant_id == $tid;"#).await?;
    let incomplete = documents(
        tx,
        r#"match $r isa business_record;
      { not { $r has biz_person_id $p; }; } or { not { $r has biz_tenant_id $t; }; };
      fetch { "id": $r.record_id };"#,
    )
    .await?;
    ensure!(
        incomplete.is_empty(),
        "Incomplete private ownership; no migration changes committed"
    );
    let dangling = documents(
        tx,
        r#"match
      $r isa business_record, has biz_person_id $pid, has biz_tenant_id $tid;
      { not { $u isa user, has biz_id == $pid; }; } or
      { not { $t isa company, has biz_id == $tid; }; };
      fetch { "id": $r.record_id };"#,
    )
    .await?;
    ensure!(
        dangling.is_empty(),
        "Dangling stable private ownership; no migration changes committed"
    );
    Ok(())
}
pub fn pair(tenant: &str, person: &str) -> Result<String> {
    Ok(format!(
        "{}:{}",
        uuid::Uuid::parse_str(tenant)?,
        uuid::Uuid::parse_str(person)?
    ))
}

fn field<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid migration result field: {key}"))
}

#[cfg(test)]
pub(crate) fn trace_stage(label: &str, started: std::time::Instant) {
    if std::env::var_os("BUSINESS_TRACE_STAGES").is_some() && started.elapsed().as_millis() >= 500 {
        eprintln!(
            "BUSINESS_STAGE {label} ms={}",
            started.elapsed().as_millis()
        );
    }
}

#[cfg(test)]
pub(crate) fn trace_enter(label: &str) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static EVENTS: AtomicUsize = AtomicUsize::new(0);
    if std::env::var_os("BUSINESS_TRACE_STAGES").is_some()
        && EVENTS.fetch_add(1, Ordering::Relaxed) < 80
    {
        eprintln!("BUSINESS_ENTER {label}");
    }
}
