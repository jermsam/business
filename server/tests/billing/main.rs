//! Explicitly gated real sandbox workflow. No configured database is changed.
use anyhow::{ensure, Context, Result};
use serde_json::json;
use server::{
    billing::{billing_schema::Provider, engine::Engine, providers::pairs},
    database,
    typedb::TypeDBState,
};
use std::{collections::HashMap, sync::Arc};
use typedb_driver::TransactionType;

#[tokio::test]
#[ignore = "Requires approved development TypeDB, Stripe TEST key and approved Resend test recipient; creates only a disposable database and synthetic provider records"]
async fn scheduled_payment_receipt_and_decline_reminder() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == "business_dev",
        "Development credentials required"
    );
    let secrets: HashMap<String, String> =
        dotenvy::from_path_iter(std::env::var("BILLING_TEST_ENV")?)?
            .collect::<std::result::Result<_, _>>()?;
    let token = secrets
        .get("STRIPE_SECRET_KEY")
        .context("Missing test key")?;
    ensure!(token.starts_with("sk_test_"), "Sandbox only");
    let merchant = uuid::Uuid::new_v4().to_string();
    std::env::set_var("BILLING_MERCHANT_ID", &merchant);
    std::env::set_var("BILLING_LIVE", "false");
    std::env::set_var("MERCURY_CONNECTIONS_JSON", json!([{"tenant_id":merchant,"identity_ids":["sandbox-fixture"],"environment":"sandbox","token":"readonly-fixture-never-sent","write":false}]).to_string());
    std::env::set_var("STRIPE_CONNECTIONS_JSON", json!([{"tenant_id":merchant,"identity_ids":["sandbox-fixture"],"environment":"test","token":token,"write":true}]).to_string());
    for key in ["RESEND_API_KEY", "BILLING_EMAIL_FROM", "BILLING_TEST_EMAIL"] {
        std::env::set_var(
            key,
            secrets
                .get(key)
                .context("Missing test mail configuration")?,
        );
    }
    std::env::set_var("BILLING_MAIL_ENABLED", "true");
    let recipient = std::env::var("BILLING_TEST_EMAIL")?;
    ensure!(
        recipient == "dev@jitpomi.com",
        "Only approved validation mailbox"
    );
    let address = std::env::var("TYPEDB_ADDR")?;
    let tls = std::env::var("TYPEDB_TLS").as_deref() != Ok("false");
    ensure!(
        tls || address.starts_with("127.0.0.1:"),
        "Plaintext is restricted to disposable loopback testing"
    );
    let driver = Arc::new(
        dog_typedb::TypeDBDriverFactory::connect(
            &address,
            &std::env::var("TYPEDB_USERNAME")?,
            &std::env::var("TYPEDB_PASSWORD")?,
            tls,
        )
        .await?,
    );
    let name = format!("business_backend_test_{}", uuid::Uuid::new_v4().simple());
    database::install(&driver, &name).await?;
    let state = Arc::new(TypeDBState {
        driver: driver.clone(),
        database: name.clone(),
    });
    let engine = Engine::configured(state)?;
    ensure!(
        engine.providers.ready(Provider::Mercury).is_err(),
        "Read-only Mercury connection became writable"
    );
    let outcome=async {
        let now=chrono::Utc::now().timestamp();
        let tx=driver.transaction(&name,TransactionType::Write).await?;
        tx.query(format!(r#"insert $t isa tenant,has tenant_id "{merchant}",has name "TEST billing merchant",has status "active",has revision "initial";"#)).await?;
        tx.commit().await?;
        for (label,method) in [("success","pm_card_visa"),("decline","pm_card_chargeCustomerFail")] {
            let cid=uuid::Uuid::new_v4().to_string();
            let customer=engine.providers.customer(Provider::Stripe,&cid,&format!("TEST ONLY {label}"),&recipient).await?;
            let saved=engine.providers.stripe(reqwest::Method::POST,&format!("/payment_methods/{method}/attach"),&format!("fixture-{cid}"),&pairs(&[("customer",&customer)])).await?;
            let pm=saved["id"].as_str().context("Missing payment method")?;
            let plan=uuid::Uuid::new_v4().to_string();
            engine.write(format!(r#"{} insert
                $buyer isa tenant,has tenant_id "{}",has name "TEST buyer {label}",has status "active",has revision "initial";
                $c isa bill_customer,has bill_key "{cid}",has name "TEST {label}",has bill_email "{recipient}",has bill_policy "customer",has bill_choice "automatic",has bill_provider "stripe",has bill_stripe_id "{customer}",has bill_mercury_id "",has bill_payment_method "{pm}",has bill_consent "fixture-consent",has bill_revision "fixture-consent";
                (merchant:$t,buyer:$buyer,customer:$c) isa bill_account;
                $p isa bill_plan,has bill_key "{plan}",has name "TEST ONLY {label} scheduled invoice",has bill_amount 100,has bill_due_days 0,has bill_anchor {now},has bill_next {now},has bill_sequence 0,has bill_interval "once",has bill_status "active";
                (customer:$c,plan:$p) isa bill_plan_owner;fetch {{"id":$p.bill_key}};"#,engine.prefix(),uuid::Uuid::new_v4())).await?;
        }
        // Not due yet; no provider effects. Overlapping due claims cannot duplicate occurrences.
        ensure!(engine.enqueue_due(now-1).await?==0,"Early invoice");
        let (a,b)=tokio::join!(engine.enqueue_due(now),engine.enqueue_due(now));
        ensure!(a.is_ok() || b.is_ok(),"Both scheduling claims failed: {a:?} {b:?}");
        ensure!(engine.enqueue_due(now).await?==0,"Schedule repeated");
        let rows=engine.read(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;fetch {{"id":$i.bill_key,"name":$i.name,"customer":$c.bill_key,"plan":$p.bill_key}};"#,engine.prefix())).await?;
        ensure!(rows.len()==2,"Duplicate or missing invoice: {}",rows.len());
        for row in &rows { engine.issue(row["id"].as_str().unwrap()).await?; }
        // Revocation and paused schedules must stop collection even without a changed revision.
        let sample=&rows[0]; let invoice=sample["id"].as_str().unwrap();
        engine.write(format!(r#"{} select $t,$c;distinct;update $c has bill_choice "manual";fetch {{"id":$c.bill_key}};"#,engine.invoice_match(invoice)?)).await?;
        engine.collect_due(invoice,now).await?;
        engine.write(format!(r#"{} select $t,$c,$p;distinct;update $c has bill_choice "automatic";$p has bill_status "paused";fetch {{"id":$c.bill_key}};"#,engine.invoice_match(invoice)?)).await?;
        engine.collect_due(invoice,now).await?;
        let binding=engine.read(format!(r#"{} fetch {{"external":$i.bill_external_id}};"#,engine.invoice_match(invoice)?)).await?;
        let observed=engine.providers.observe(Provider::Stripe,binding[0]["external"].as_str().unwrap()).await?;
        ensure!(observed["attempt_count"]==0,"Revoked consent or paused schedule was charged");
        engine.write(format!(r#"{} select $t,$p;distinct;update $p has bill_status "finished";fetch {{"id":$p.bill_key}};"#,engine.invoice_match(invoice)?)).await?;
        let mut other=engine.clone();other.merchant=uuid::Uuid::new_v4().to_string();
        ensure!(other.reconcile(invoice).await.is_err(),"Cross-tenant invoice access");
        let mut wrong=observed.clone();wrong["total"]=json!(101);
        ensure!(engine.apply_observation(invoice,Provider::Stripe,&wrong).await.is_err(),"Amount mismatch accepted");
        let other_invoice=rows[1]["id"].as_str().unwrap();
        ensure!(engine.apply_observation(other_invoice,Provider::Stripe,&observed).await.is_err(),"Invoice substitution accepted");
        // Actual production tick handles both due collection and receipt/reminder delivery.
        let tick=engine.tick().await?;
        ensure!(tick["errors"]==0,"Tick failed: {tick}");
        let again=engine.tick().await?;
        ensure!(again["errors"]==0 && again["notices"]==0 && again["issued"]==0,"Retry was not idempotent: {again}");
        let evidence=engine.read(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;(invoice:$i,notice:$n) isa bill_notice_owner;fetch {{"id":$i.bill_key,"name":$i.name,"status":$i.bill_status,"external":$i.bill_external_id,"notice":$n.bill_key,"delivery":$n.bill_status}};"#,engine.prefix())).await?;
        ensure!(evidence.len()==2,"Expected exactly receipt and reminder");
        for row in &evidence {
            let success=row["name"].as_str().unwrap().contains("success");
            ensure!(row["status"]==if success{"paid"}else{"open"},"Unexpected invoice outcome");
            ensure!(row["notice"].as_str().unwrap().ends_with(if success{"-receipt"}else{"-reminder-0"}),"Wrong notice");
            ensure!(row["delivery"].as_str().unwrap().starts_with("accepted:"),"Email not accepted");
            let observation=engine.providers.observe(Provider::Stripe,row["external"].as_str().unwrap()).await?;
            ensure!(observation["auto_advance"]==false,"Hidden retries enabled");
            if !success { ensure!(observation["attempt_count"].as_u64()==Some(1),"Duplicate payment attempt"); }
        }
        // Exercise the actual HTTP ingress using signed raw provider events. Payment
        // state is fetched from Stripe, never taken from the supplied event payload.
        use tower::ServiceExt;
        use hmac::{Hmac,Mac};
        use sha2::Sha256;
        use axum::{body::Body,http::Request};
        std::env::set_var("BILLING_TICK_SECRET","sandbox-tick-secret-at-least-32-characters");
        std::env::set_var("STRIPE_WEBHOOK_SECRET","whsec_synthetic_signature_test");
        let router=server::billing::http::router(engine.clone())?;
        let denied=router.clone().oneshot(Request::post("/internal/billing/tick").body(Body::empty())?).await?;
        ensure!(denied.status()==401,"Unauthenticated scheduler accepted");
        let paid=evidence.iter().find(|r|r["status"]=="paid").unwrap();
        for (signed,live,account,expected) in [(false,false,None,401),(true,true,None,400),(true,false,Some("acct_foreign"),400),(true,false,None,200),(true,false,None,200)] {
            let body=json!({"id":"evt_fixture","type":"invoice.payment_succeeded","livemode":live,"account":account,"data":{"object":{"id":paid["external"],"status":"open","metadata":{"business_invoice":paid["id"]}}}}).to_string();
            let ts=chrono::Utc::now().timestamp();
            let mut mac=Hmac::<Sha256>::new_from_slice(b"whsec_synthetic_signature_test")?;
            mac.update(format!("{ts}.{body}").as_bytes());
            let digest=mac.finalize().into_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>();
            let header=if signed{format!("t={ts},v1={digest}")}else{"invalid".into()};
            let response=router.clone().oneshot(Request::post("/webhooks/stripe").header("stripe-signature",header).body(Body::from(body))?).await?;
            ensure!(response.status().as_u16()==expected,"Unexpected webhook response {}",response.status());
        }
        ensure!(engine.notify_invoice(paid["id"].as_str().unwrap()).await?==0,"Webhook replay duplicated receipt");
        let report=json!({"test":"current-backend-full-sandbox-billing","tick":tick,"repeat_tick":again,"invoices":evidence,"security_checks":["tenant isolation","provider binding","consent revocation","paused schedule","unsigned webhook denied","wrong environment denied","wrong Connect account denied","signed webhook replay"]});
        if let Ok(path)=std::env::var("BILLING_TEST_REPORT") { std::fs::write(path,serde_json::to_string_pretty(&report)?)?; }
        println!("Verified: two scheduled invoices, successful payment + receipt, declined payment + reminder, repeated tick with no duplicate effects.");
        Ok::<_,anyhow::Error>(())
    }.await;
    // Delete only the exact UUID database created above; provider test objects remain auditable.
    driver.databases().get(&name).await?.delete().await?;
    outcome
}
