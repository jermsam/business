//! Explicit, additive legacy sandbox migration. Source is read-only; passwords never leave memory.
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use server::database::documents;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use typedb_driver::TransactionType;
const SOURCE: &str = "business_dev";
const TARGET: &str = "business_native_dev";
const OLD: &str = "c76e9dc2-f6fc-465d-bef0-9f1bd6218a23";
const NEW: &str = "6917f7a0-b86a-4bb9-86d8-06c6628ed5bb";
const MARKER: &str = "legacy-sandbox-accounts-v1";
fn s<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid source field"))
}
fn q(v: &str) -> String {
    serde_json::to_string(v).unwrap()
}
fn attrs(v: &Value, fields: &[(&str, &str)]) -> Result<String> {
    fields
        .iter()
        .map(|(field, attribute)| {
            let value = &v[*field];
            ensure!(
                value.is_string() || value.is_i64(),
                "Invalid source attribute"
            );
            Ok(format!(",has {attribute} {value}"))
        })
        .collect()
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        let diagnostic = regex::Regex::new(r"\$2[aby]\$[0-9]{2}\$[./A-Za-z0-9]{53}")
            .unwrap()
            .replace_all(&error.to_string(), "[password hash redacted]")
            .into_owned();
        eprintln!("{}", diagnostic.chars().take(1800).collect::<String>());
        eprintln!("Migration stopped; no credentials or source queries are printed. Check the current phase and inspect the databases before retrying.");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == SOURCE,
        "Development source required"
    );
    let apply = std::env::args().nth(1).as_deref() == Some("--apply");
    let d = dog_typedb::TypeDBDriverFactory::connect(
        &std::env::var("TYPEDB_ADDR")?,
        &std::env::var("TYPEDB_USERNAME")?,
        &std::env::var("TYPEDB_PASSWORD")?,
        true,
    )
    .await?;
    server::database::verify(&d, TARGET).await?;
    let check = d.transaction(TARGET, TransactionType::Read).await?;
    if !documents(
        &check,
        &format!(
            "match $b isa bootstrap_record,has bootstrap_id {};fetch {{'done':true}};",
            q(MARKER)
        ),
    )
    .await?
    .is_empty()
    {
        let copied=documents(&check,"match $p isa person,has credential_version \"migration-v1\";fetch {'id':$p.identity_id,'hash':$p.password_hash};").await?;
        let source = d.transaction(SOURCE, TransactionType::Read).await?;
        for person in &copied {
            let original = documents(
                &source,
                &format!(
                    "match $u isa user,has biz_id {};fetch {{'hash':$u.password}};",
                    q(s(person, "id")?)
                ),
            )
            .await?;
            ensure!(
                original.len() == 1 && original[0]["hash"] == person["hash"],
                "Password preservation check failed"
            );
        }
        let accounts=documents(&check,r#"match $p isa person;{ $p has email "dev@jitpomi.com"; } or { $p has email "ssali@jitpomi.com"; }; (tenant:$t,member:$p) isa membership,has membership_role "owner";fetch {"email":$p.email,"tenant":$t.tenant_id};"#).await?;
        ensure!(
            accounts
                .iter()
                .any(|a| a["email"] == "dev@jitpomi.com" && a["tenant"] == NEW),
            "Admin membership missing"
        );
        ensure!(
            accounts.iter().any(|a| a["email"] == "ssali@jitpomi.com"
                && a["tenant"] == "14e34eaf-01d4-44af-a153-a19a4017870c"),
            "Customer membership missing"
        );
        println!("Verified {} copied password hashes against the unchanged source; admin and customer memberships present.",copied.len());
        println!("Migration already committed; no changes made.");
        return Ok(());
    }
    let existing_people = documents(
        &check,
        "match $p isa person;fetch {'id':$p.identity_id,'email':$p.email};",
    )
    .await?;
    let existing_members = documents(&check, "match (tenant:$t,member:$u) isa membership;fetch {'tenant':$t.tenant_id,'id':$u.identity_id};").await?;
    check.close().await?;
    println!("Phase: read source snapshot");
    let tx = d.transaction(SOURCE, TransactionType::Read).await?;
    let prefix = format!(
        r#"match $m isa company,has biz_id "{OLD}";(merchant:$m,buyer:$b,customer:$c) isa bill_account;"#
    );
    let customers=documents(&tx,&format!(r#"{prefix} fetch {{"id":$c.bill_key,"buyer":$b.biz_id,"name":$c.name,"email":$c.bill_email,"policy":$c.bill_policy,"provider":$c.bill_provider,"stripe":$c.bill_stripe_id,"mercury":$c.bill_mercury_id,"method":$c.bill_payment_method,"removed":[$c.bill_removed_method]}};"#)).await?;
    let now = chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string();
    let mut members=documents(&tx,&format!(r#"match $t isa company;
 $u isa user; $membership isa biz_membership,links(tenant:$t,person:$u),has biz_state "active";
 let $active=biz_account_active($u,$t);$active==true;let $member=biz_member($u,$t,{now});$member==true;
 fetch {{"id":$u.biz_id,"email":$u.email,"password":$u.password,"tenant":$t.biz_id,"role":$membership.biz_role,"start":$membership.biz_start,"ends":[$membership.biz_end]}};"#)).await?;
    let allowed: BTreeSet<_> = customers
        .iter()
        .filter_map(|c| c["buyer"].as_str())
        .chain(std::iter::once(OLD))
        .collect();
    members.retain(|u| u["tenant"].as_str().is_some_and(|t| allowed.contains(t)));
    let plans=documents(&tx,&format!(r#"{prefix}(customer:$c,plan:$p) isa bill_plan_owner;fetch {{"id":$p.bill_key,"customer":$c.bill_key,"name":$p.name,"amount":$p.bill_amount,"due_days":$p.bill_due_days,"anchor":$p.bill_anchor,"next":$p.bill_next,"sequence":$p.bill_sequence,"interval":$p.bill_interval,"status":$p.bill_status}};"#)).await?;
    let invoices=documents(&tx,&format!(r#"{prefix}(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;fetch {{"id":$i.bill_key,"plan":$p.bill_key,"name":$i.name,"amount":$i.bill_amount,"anchor":$i.bill_anchor,"next":$i.bill_next,"sequence":$i.bill_sequence,"choice":$i.bill_choice,"provider":$i.bill_provider,"status":$i.bill_status,"external":$i.bill_external_id,"url":$i.bill_url,"updated":$i.bill_updated,"revision":$i.bill_revision,"history":[$i.bill_history]}};"#)).await?;
    tx.close().await?;
    let mut users = BTreeMap::new();
    let mut tenants = BTreeMap::new();
    for c in &customers {
        tenants.insert(s(c, "buyer")?.to_owned(), s(c, "name")?.to_owned());
    }
    for u in &members {
        ensure!(
            matches!(s(u, "role")?, "owner" | "admin" | "member"),
            "Unsupported role"
        );
        bcrypt::HashParts::from_str(s(u, "password")?)?;
        users.insert(s(u, "id")?.to_owned(), u);
    }
    let emails: Vec<_> = users.values().map(|u| u["email"].clone()).collect();
    println!(
        "{}",
        json!({"apply":apply,"users":users.len(),"memberships":members.len(),"customers":customers.len(),"plans":plans.len(),"historical_invoices":invoices.len(),"emails":emails,"source_unchanged":true,"schedules_paused":true,"automatic_consent_reset":true})
    );

    println!("Phase: atomic target import");
    let out = d.transaction(TARGET, TransactionType::Write).await?;
    ensure!(
        documents(
            &out,
            &format!(r#"match $t isa tenant,has tenant_id "{NEW}";fetch {{"id":$t.tenant_id}};"#)
        )
        .await?
        .len()
            == 1,
        "Target merchant missing"
    );
    out.query(format!(
        r#"insert $marker isa bootstrap_record,has bootstrap_id "{MARKER}";"#
    ))
    .await?;
    for (id, name) in &tenants {
        ensure!(id != NEW && id != OLD, "Ambiguous buyer");
        out.query(format!(r#"insert $t isa tenant,has tenant_id {},has name {},has status "active",has revision "migration-v1";"#,q(id),q(name))).await?;
    }
    let mut aliases = BTreeMap::new();
    for (id, u) in &users {
        if let Some(existing) = existing_people.iter().find(|p| p["email"] == u["email"]) {
            aliases.insert(id.clone(), s(existing, "id")?.to_owned());
        }
    }
    for (id, u) in users {
        if aliases.contains_key(&id) {
            continue;
        }
        out.query(format!(r#"insert $u isa person,has identity_id {},has name {},has email {},has password_hash {},has credential_version "migration-v1",has status "active";"#,q(&id),q(s(u,"email")?),q(s(u,"email")?),q(s(u,"password")?))).await?;
    }
    let mut seen = BTreeSet::new();
    for u in &members {
        let tid = if s(u, "tenant")? == OLD {
            NEW
        } else {
            s(u, "tenant")?
        };
        let source_id = s(u, "id")?;
        let id = aliases
            .get(source_id)
            .map(String::as_str)
            .unwrap_or(source_id);
        if existing_members
            .iter()
            .any(|m| m["tenant"] == tid && m["id"] == id)
        {
            continue;
        }

        ensure!(seen.insert((tid, id)), "Duplicate membership");
        let start = s(u, "start")?;
        chrono::NaiveDateTime::parse_from_str(start, "%Y-%m-%dT%H:%M:%S%.f")?;
        let mut end = String::new();
        for v in u["ends"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid end"))?
        {
            let date = v.as_str().ok_or_else(|| anyhow::anyhow!("Invalid end"))?;
            chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S%.f")?;
            end.push_str(&format!(",has end_at {date}"));
        }
        out.query(format!(r#"match $t isa tenant,has tenant_id {};$u isa person,has identity_id {};insert (tenant:$t,member:$u) isa membership,has membership_role {},has start_at {start}{end};"#,q(tid),q(id),q(s(u,"role")?))).await?;
    }
    for c in &customers {
        out.query(format!(r#"match $m isa tenant,has tenant_id "{NEW}";$b isa tenant,has tenant_id {};insert $c isa bill_customer{},has bill_choice "manual",has bill_consent "",has bill_revision "migration-v1";(merchant:$m,buyer:$b,customer:$c) isa bill_account;"#,q(s(c,"buyer")?),attrs(c,&[("id","bill_key"),("name","name"),("email","bill_email"),("policy","bill_policy"),("provider","bill_provider"),("stripe","bill_stripe_id"),("mercury","bill_mercury_id"),("method","bill_payment_method")])?)).await?;
        for removed in c["removed"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid removed methods"))?
        {
            out.query(format!(
                "match $c isa bill_customer,has bill_key {};insert $c has bill_removed_method {};",
                q(s(c, "id")?),
                removed
            ))
            .await?;
        }
    }
    for p in &plans {
        let status = if s(p, "status")? == "finished" {
            "finished"
        } else {
            "paused"
        };
        out.query(format!(r#"match $c isa bill_customer,has bill_key {};insert $p isa bill_plan{},has bill_status "{status}";(customer:$c,plan:$p) isa bill_plan_owner;"#,q(s(p,"customer")?),attrs(p,&[("id","bill_key"),("name","name"),("amount","bill_amount"),("due_days","bill_due_days"),("anchor","bill_anchor"),("next","bill_next"),("sequence","bill_sequence"),("interval","bill_interval")])?)).await?;
    }
    for i in &invoices {
        out.query(format!(r#"match $p isa bill_plan,has bill_key {};insert $i isa bill_invoice{};(plan:$p,invoice:$i) isa bill_invoice_owner;$n isa bill_notice,has bill_key {},has bill_status "imported-history",has bill_updated {}; (invoice:$i,notice:$n) isa bill_notice_owner;"#,q(s(i,"plan")?),attrs(i,&[("id","bill_key"),("name","name"),("amount","bill_amount"),("anchor","bill_anchor"),("next","bill_next"),("sequence","bill_sequence"),("choice","bill_choice"),("provider","bill_provider"),("status","bill_status"),("external","bill_external_id"),("url","bill_url"),("updated","bill_updated"),("revision","bill_revision")])?,q(&format!("{}-migration-archive",s(i,"id")?)),chrono::Utc::now().timestamp())).await?;
        for h in i["history"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid history"))?
        {
            out.query(format!(
                "match $i isa bill_invoice,has bill_key {};insert $i has bill_history {};",
                q(s(i, "id")?),
                h
            ))
            .await?;
        }
    }
    if !apply {
        out.close().await?;
        println!("Dry run validated all writes, then rolled back; no data changed.");
        return Ok(());
    }
    println!("Phase: commit (never automatically retried)");
    out.commit().await?;
    println!("Migration committed. Old database unchanged; existing passwords retained; old invoices archived from automatic processing.");
    Ok(())
}
