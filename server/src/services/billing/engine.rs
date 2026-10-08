//! Durable scheduling and reconciliation. External requests never run inside a DB transaction.
use super::{
    billing_schema::{self as schema, Interval, Mode, Policy, Provider},
    providers::{self, InvoiceRequest, Providers},
};
use crate::typedb::TypeDBState;
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone)]
pub struct Engine {
    pub state: Arc<TypeDBState>,
    pub merchant: String,
    pub providers: Providers,
}
pub const CUSTOMER_DATA: &str = r#""customer_id":$c.bill_key,"name":$c.name,"email":$c.bill_email,"policy":$c.bill_policy,"choice":$c.bill_choice,"manual_provider":$c.bill_provider,"stripe_id":$c.bill_stripe_id,"mercury_id":$c.bill_mercury_id,"payment_method":$c.bill_payment_method,"consent":$c.bill_consent,"revision":$c.bill_revision"#;
impl Engine {
    pub fn configured(state: Arc<TypeDBState>) -> Result<Self> {
        let merchant = schema::id(&std::env::var("BILLING_MERCHANT_ID")?)?;
        Ok(Self {
            state,
            merchant,
            providers: Providers::new()?,
        })
    }
    pub fn prefix(&self) -> String {
        format!(
            r#"match $t isa company,has biz_id "{}",has biz_revision $revision;"#,
            self.merchant
        )
    }
    pub async fn read(&self, query: String) -> Result<Vec<Value>> {
        let response = dog_typedb::TypeDBAdapter::new(self.state.clone())
            .read(json!({"query":query}))
            .await?;
        Ok(response["ok"]["answers"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid database response"))?
            .iter()
            .map(|v| v["data"].clone())
            .collect())
    }
    pub async fn write(&self, query: String) -> Result<Value> {
        let mut rows = crate::access::write_one(&self.state, &query).await?;
        ensure!(rows.len() == 1, "Invalid billing write result");
        Ok(rows.remove(0))
    }
    pub fn customer_match(&self, id: &str) -> Result<String> {
        Ok(format!(
            r#"{} $c isa bill_customer,has bill_key "{}";(merchant:$t,customer:$c) isa bill_account;"#,
            self.prefix(),
            schema::id(id)?
        ))
    }
    pub async fn customer(&self, id: &str) -> Result<Value> {
        let rows = self
            .read(format!(
                "{} fetch {{{CUSTOMER_DATA}}};",
                self.customer_match(id)?
            ))
            .await?;
        ensure!(
            rows.len() == 1,
            "Customer unavailable for configured merchant"
        );
        Ok(rows[0].clone())
    }
    pub async fn ensure_customer(&self, customer: &Value, provider: Provider) -> Result<String> {
        let attribute = if provider == Provider::Stripe {
            "bill_stripe_id"
        } else {
            "bill_mercury_id"
        };
        let field = if provider == Provider::Stripe {
            "stripe_id"
        } else {
            "mercury_id"
        };
        let current = string(customer, field)?;
        if !current.is_empty() {
            providers::safe_id(current)?;
            return Ok(current.into());
        }
        let id = string(customer, "customer_id")?;
        let external = self
            .providers
            .customer(
                provider,
                id,
                string(customer, "name")?,
                string(customer, "email")?,
            )
            .await?;
        providers::safe_id(&external)?;
        self.write(format!(r#"{} $c has {attribute} "";select $t,$c;distinct;update $c has {attribute} {};fetch {{"id":$c.bill_key}};"#,self.customer_match(id)?,schema::quoted(&external))).await?;
        Ok(external)
    }
    /// At most ten due occurrences per invocation; each sequence advances atomically
    /// with its invoice. A conflict aborts before any provider request.
    pub async fn enqueue_due(&self, now: i64) -> Result<usize> {
        let rows=self.read(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;
   $p has bill_status "active",has bill_next $next;$next <= {now};
   select $c,$p,$next;distinct;sort $next;limit 10;
   fetch {{"plan_id":$p.bill_key,"anchor":$p.bill_anchor,"sequence":$p.bill_sequence,"interval":$p.bill_interval,"next":$next}};"#,self.prefix())).await?;
        let mut count = 0;
        for row in rows {
            let sequence = number(&row, "sequence")?;
            let interval: Interval = serde_json::from_value(row["interval"].clone())?;
            let next = schema::occurrence(
                number(&row, "anchor")?,
                interval,
                u32::try_from(sequence + 1)?,
            )?;
            let status = if next.is_some() { "active" } else { "finished" };
            let next = next.unwrap_or(0);
            let plan = schema::id(string(&row, "plan_id")?)?;
            let invoice = uuid::Uuid::new_v4();
            self.write(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;
    $p has bill_key "{plan}",has bill_status "active",has bill_sequence {sequence},has bill_next $issued,has bill_due_days $days,has bill_amount $amount,has name $name;
    $issued <= {now};let $due=$issued+($days*86400);
    select $t,$p,$name,$amount,$issued,$due;distinct;
    update $p has bill_sequence {},has bill_next {next},has bill_status "{status}";
    insert $i isa bill_invoice,has bill_key "{invoice}",has name == $name,has bill_amount == $amount,has bill_anchor == $issued,has bill_next == $due,has bill_sequence {sequence},has bill_choice "manual",has bill_provider "mercury",has bill_status "queued",has bill_external_id "",has bill_url "",has bill_updated {now},has bill_revision "initial";
    (plan:$p,invoice:$i) isa bill_invoice_owner;fetch {{"id":$i.bill_key}};"#,self.prefix(),sequence+1)).await?;
            count += 1;
        }
        Ok(count)
    }
    pub fn invoice_match(&self, id: &str) -> Result<String> {
        Ok(format!(
            r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;$i has bill_key "{}";"#,
            self.prefix(),
            schema::id(id)?
        ))
    }
    pub async fn issue(&self, id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        // Read customer settings at claim time. Revision validation below rejects a
        // preference or consent change racing the claim.
        let rows=self.read(format!(r#"{} fetch {{{CUSTOMER_DATA},"invoice_id":$i.bill_key,"description":$i.name,"amount":$i.bill_amount,"issued":$i.bill_anchor,"due":$i.bill_next}};"#,self.invoice_match(id)?)).await?;
        ensure!(rows.len() == 1, "Invoice missing");
        let row = &rows[0];
        let mode = schema::effective(
            serde_json::from_value::<Policy>(row["policy"].clone())?,
            serde_json::from_value::<Mode>(row["choice"].clone())?,
            !string(row, "consent")?.is_empty() && row["consent"] == row["revision"],
            !string(row, "payment_method")?.is_empty(),
        );
        let provider = if mode == Mode::Automatic {
            Provider::Stripe
        } else {
            serde_json::from_value::<Provider>(row["manual_provider"].clone())?
        };
        // A missing key must not consume a job; check before the durable claim.
        self.provider_ready(provider)?;
        self.write(format!(r#"{} $i has bill_status "queued";$c has bill_revision {};select $t,$i;distinct;update $i has bill_status "issuing",has bill_provider "{}",has bill_choice "{}",has bill_updated {now},has bill_revision {};fetch {{"id":$i.bill_key}};"#,self.invoice_match(id)?,schema::quoted(string(row,"revision")?),schema::word(provider),schema::word(mode),schema::quoted(string(row,"revision")?))).await?;
        // Once claimed, an ambiguous failure is attention-required, never a fresh charge retry.
        let result=async{
   let customer=self.ensure_customer(row,provider).await?;
   let request=InvoiceRequest{id:id.into(),customer,provider,mode,amount:number(row,"amount")?,description:string(row,"description")?.into(),issued:number(row,"issued")?,due:number(row,"due")?};
   let mut response=self.providers.invoice(&request).await?;
   let external=providers::field(&response,"id")?;providers::safe_id(external)?;
   // Save the provider ID before adding items/finalizing. A crash leaves a
   // recoverable draft, never a second invoice or an automatic draft charge.
   self.write(format!(r#"{} $i has bill_status "issuing",has bill_external_id "";select $t,$i;distinct;update $i has bill_external_id {};fetch {{"id":$i.bill_key}};"#,self.invoice_match(id)?,schema::quoted(external))).await?;
   if provider==Provider::Stripe {response=self.providers.finish_invoice(&request,external).await?;}

   self.apply_observation(id,provider,&response).await
  }.await;
        if result.is_err() {
            let _=self.write(format!(r#"{} $i has bill_status "issuing";select $t,$i;distinct;update $i has bill_status "attention";fetch {{"id":$i.bill_key}};"#,self.invoice_match(id)?)).await;
        }
        result
    }
    fn provider_ready(&self, provider: Provider) -> Result<()> {
        match provider {
            Provider::Stripe => {
                std::env::var("STRIPE_SECRET_KEY")?;
            }
            Provider::Mercury => {
                std::env::var(if self.providers.live {
                    "MERCURY_API_TOKEN"
                } else {
                    "MERCURY_SANDBOX_TOKEN"
                })?;
                std::env::var(if self.providers.live {
                    "MERCURY_ACCOUNT_ID"
                } else {
                    "MERCURY_SANDBOX_ACCOUNT_ID"
                })?;
            }
        }
        Ok(())
    }
    pub async fn apply_observation(
        &self,
        id: &str,
        provider: Provider,
        value: &Value,
    ) -> Result<()> {
        self.apply_observation_guarded(id, provider, value, "")
            .await
    }
    pub async fn apply_observation_guarded(
        &self,
        id: &str,
        provider: Provider,
        value: &Value,
        permission: &str,
    ) -> Result<()> {
        let external = providers::field(value, "id")?;
        providers::safe_id(external)?;
        let status = providers::status(provider, value)?;
        let url = providers::pay_url(provider, value, self.providers.live)?;
        let now = chrono::Utc::now().timestamp();
        let rows=self.read(format!(r#"{} fetch {{"provider":$i.bill_provider,"external":$i.bill_external_id,"amount":$i.bill_amount,"mercury_id":$c.bill_mercury_id,"stripe_id":$c.bill_stripe_id,"status":$i.bill_status}};"#,self.invoice_match(id)?)).await?;
        ensure!(rows.len() == 1, "Invoice missing");
        let row = &rows[0];
        ensure!(
            row["provider"] == schema::word(provider),
            "Provider mismatch"
        );
        let old = string(row, "external")?;
        ensure!(old.is_empty() || old == external, "Invoice ID mismatch");
        match provider {
            Provider::Stripe => {
                ensure!(
                    value["customer"] == row["stripe_id"]
                        && value["currency"] == "usd"
                        && value["total"].as_i64() == Some(number(row, "amount")?),
                    "Provider customer, currency or amount mismatch"
                );
                ensure!(
                    value["metadata"]["business_invoice"] == id,
                    "Invoice binding mismatch"
                );
            }
            Provider::Mercury => {
                ensure!(
                    value["customerId"] == row["mercury_id"]
                        && value["currencyCode"] == "USD"
                        && value["invoiceNumber"] == format!("JIT-{id}"),
                    "Invoice binding mismatch"
                );
                ensure!(
                    providers::decimal_cents(&value["amount"])? == number(row, "amount")?,
                    "Provider amount mismatch"
                );
            }
        }
        // Older deliveries may never undo a paid/void observation. Refunds and disputes
        // are distinct events, not a transition back to unpaid.
        if matches!(string(row, "status")?, "paid" | "void") && row["status"] != status {
            return Ok(());
        }
        self.write(format!(r#"{permission} {} $i has bill_status {};select $t,$i;distinct;update $i has bill_status "{status}",has bill_external_id {},has bill_url {},has bill_updated {now};fetch {{"id":$i.bill_key}};"#,self.invoice_match(id)?,schema::quoted(string(row,"status")?),schema::quoted(external),schema::quoted(&url))).await?;
        Ok(())
    }
    pub async fn reconcile(&self, id: &str) -> Result<()> {
        let rows = self
            .read(format!(
                r#"{} fetch {{"provider":$i.bill_provider,"external":$i.bill_external_id}};"#,
                self.invoice_match(id)?
            ))
            .await?;
        ensure!(rows.len() == 1, "Invoice missing");
        let row = &rows[0];
        let provider: Provider = serde_json::from_value(row["provider"].clone())?;
        let external = string(row, "external")?;
        ensure!(!external.is_empty(),"Unknown provider outcome: operator must locate the existing invoice; never recreate it");
        let observation = self.providers.observe(provider, external).await?;
        self.apply_observation(id, provider, &observation).await
    }
    /// One off-session attempt per invoice. A failed or uncertain attempt is
    /// reconciled and the customer receives a payment link; it is never recharged.
    pub async fn collect_due(&self, id: &str, now: i64) -> Result<()> {
        let rows=self.read(format!(r#"{} $i has bill_status "open",has bill_choice "automatic",has bill_provider "stripe",has bill_next $due;$due <= {now};
   $i has bill_revision $consent;$c has bill_revision $consent,has bill_consent == $consent;
   $consent!=""; $p has bill_status $plan_status;$plan_status!="paused";
   fetch {{{CUSTOMER_DATA},"external":$i.bill_external_id}};"#,self.invoice_match(id)?)).await?;
        if rows.is_empty() {
            return Ok(());
        }
        ensure!(rows.len() == 1, "Ambiguous invoice");
        let row = &rows[0];
        let consent = string(row, "revision")?;
        let attempted = format!("attempted:{consent}");
        self.write(format!(r#"{} $i has bill_status "open",has bill_revision {}; $c has bill_revision {},has bill_consent {};select $t,$i;distinct;update $i has bill_revision {},has bill_updated {now};fetch {{"id":$i.bill_key}};"#,self.invoice_match(id)?,schema::quoted(consent),schema::quoted(consent),schema::quoted(consent),schema::quoted(&attempted))).await?;
        let result = self
            .providers
            .charge(string(row, "external")?, id, string(row, "payment_method")?)
            .await;
        // Even HTTP 402 can contain an already-created PaymentIntent: fresh GET is
        // authoritative and only the first caller can obtain the claim above.
        match result {
            Ok(value) => self.apply_observation(id, Provider::Stripe, &value).await,
            Err(_) => self.reconcile(id).await,
        }
    }
    // Rotate attempted work even when a provider errors or the tick is cancelled.
    // Otherwise a permanently failing first batch can starve every later invoice.
    pub async fn mark_attempt(&self, id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.write(format!(r#"{} select $t,$i;distinct;update $i has bill_updated {now};fetch {{"id":$i.bill_key}};"#,self.invoice_match(id)?)).await?;
        Ok(())
    }
    pub async fn tick(&self) -> Result<Value> {
        let scheduled = self.enqueue_due(chrono::Utc::now().timestamp()).await?;
        let rows=self.read(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;$i has bill_status "queued",has bill_updated $updated,has bill_key $id;select $i,$updated,$id;distinct;sort $updated,$id;limit 10;fetch {{"id":$i.bill_key}};"#,self.prefix())).await?;
        let mut issued = 0;
        let mut errors = 0;
        for row in rows {
            let id = string(&row, "id")?;
            self.mark_attempt(id).await?;
            if self.issue(id).await.is_ok() {
                issued += 1
            } else {
                errors += 1
            }
        }
        let rows=self.read(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;$i has bill_external_id $external;$external!="";$i has bill_status $status;$status!="paid";$status!="void";$i has bill_updated $updated;select $i,$updated;distinct;sort $updated;limit 20;fetch {{"id":$i.bill_key}};"#,self.prefix())).await?;
        let mut reconciled = 0;
        for row in rows {
            let id = string(&row, "id")?;
            self.mark_attempt(id).await?;
            if self.reconcile(id).await.is_ok() {
                reconciled += 1;
                if self
                    .collect_due(id, chrono::Utc::now().timestamp())
                    .await
                    .is_err()
                {
                    errors += 1
                }
            } else {
                errors += 1
            }
        }
        let (notices, notification_errors) = self.send_notices().await?;
        errors += notification_errors;
        Ok(
            json!({"scheduled":scheduled,"issued":issued,"reconciled":reconciled,"notices":notices,"errors":errors}),
        )
    }
}
pub fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    providers::field(value, key)
}
pub fn number(value: &Value, key: &str) -> Result<i64> {
    let number = &value[key];
    if let Some(integer) = number.as_i64() {
        return Ok(integer);
    }
    // TypeDB's JSON transport emits integral values as JSON doubles. Accept only
    // finite, exactly representable integers; never silently round money or dates.
    let number = number
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("Missing integer {key}"))?;
    ensure!(
        number.is_finite() && number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0,
        "Inexact integer {key}"
    );
    Ok(number as i64)
}
