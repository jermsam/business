//! Durable notification claims. An uncertain send requires reconciliation, not blind resend.
use super::{
    billing_schema::quoted,
    engine::{number, string, Engine},
};
use anyhow::{ensure, Result};
impl Engine {
    pub async fn notify_invoice(&self, id: &str) -> Result<usize> {
        if std::env::var("BILLING_MAIL_ENABLED").as_deref() != Ok("true") {
            return Ok(0);
        }
        // Fresh provider state avoids reminders based only on an old local observation.
        self.reconcile(id).await?;
        let rows=self.read(format!(r#"{} fetch {{"email":$c.bill_email,"description":$i.name,"amount":$i.bill_amount,"due":$i.bill_next,"status":$i.bill_status,"provider":$i.bill_provider,"url":$i.bill_url}};"#,self.invoice_match(id)?)).await?;
        ensure!(rows.len() == 1, "Invoice missing");
        let row = &rows[0];
        let now = chrono::Utc::now().timestamp();
        let state = string(row, "status")?;
        let due = number(row, "due")?;
        let interval = std::env::var("BILLING_REMINDER_DAYS")
            .unwrap_or("7".into())
            .parse::<i64>()?;
        ensure!(
            (1..=30).contains(&interval),
            "Reminder interval must be 1..30 days"
        );
        let Some(kind) = provider_notice_kind(string(row, "provider")?, state, due, now, interval)
        else {
            return Ok(0);
        };
        let key = format!("{id}-{kind}");
        let exists=self.read(format!(r#"{} (invoice:$i,notice:$n) isa bill_notice_owner;$n has bill_key {};fetch {{"status":$n.bill_status}};"#,self.invoice_match(id)?,quoted(&key))).await?;
        if !exists.is_empty() {
            return Ok(0);
        }
        let recipient = string(row, "email")?;
        if !self.providers.live {
            ensure!(
                std::env::var("BILLING_TEST_EMAIL").as_deref() == Ok(recipient),
                "Sandbox emails require an explicitly allowed test recipient"
            );
        }
        let from = std::env::var("BILLING_EMAIL_FROM")?;
        let token = std::env::var("RESEND_API_KEY")?;
        ensure!(
            !token.is_empty() && !from.is_empty(),
            "Mail is not configured"
        );
        let amount = number(row, "amount")?;
        let description = string(row, "description")?;
        let url = string(row, "url")?;
        let (subject, body) = if kind == "receipt" {
            ("Payment received",format!("Thank you. Your invoice {id} for {description} is marked paid.\nAmount: USD {}.{:02}\nView your invoice: {url}\n",amount/100,amount%100))
        } else {
            ("Your invoice",format!("Your invoice {id} for {description} is {}.\nAmount: USD {}.{:02}\nDue: {} UTC\nView and pay securely: {url}\nIf you need more time, please contact us. If payment is already processing, no further action is needed.\n",if now>=due{"due"}else{"ready"},amount/100,amount%100,chrono::DateTime::from_timestamp(due,0).ok_or_else(||anyhow::anyhow!("Invalid date"))?.format("%Y-%m-%d")))
        };
        let subject = if self.providers.live {
            subject.to_owned()
        } else {
            format!("[TEST — no real payment] {subject}")
        };
        self.write(format!(r#"{} $i has bill_status {};not {{$n isa bill_notice,has bill_key {};}};select $t,$i;distinct;
   insert $n isa bill_notice,has bill_key {},has bill_status "sending",has bill_updated {now};(invoice:$i,notice:$n) isa bill_notice_owner;fetch {{"id":$n.bill_key}};"#,self.invoice_match(id)?,quoted(state),quoted(&key),quoted(&key))).await?;
        let result = self
            .providers
            .email(&token, &from, recipient, &subject, &body, &key)
            .await;
        let status = match &result {
            Ok(id) => format!("accepted:{id}"),
            Err(_) => "attention".into(),
        };
        self.write(format!(r#"{} (invoice:$i,notice:$n) isa bill_notice_owner;$n has bill_key {},has bill_status "sending";select $t,$n;distinct;update $n has bill_status {};fetch {{"id":$n.bill_key}};"#,self.invoice_match(id)?,quoted(&key),quoted(&status))).await?;
        result?;
        Ok(1)
    }
    pub async fn send_notices(&self) -> Result<usize> {
        if std::env::var("BILLING_MAIL_ENABLED").as_deref() != Ok("true") {
            return Ok(0);
        }
        let rows = self.notification_candidates().await?;
        let mut sent = 0;
        for row in rows {
            sent += self.notify_invoice(string(&row, "id")?).await?;
        }
        Ok(sent)
    }
    pub async fn notification_candidates(&self) -> Result<Vec<serde_json::Value>> {
        let rows=self.read(format!(r#"{} (merchant:$t,customer:$c) isa bill_account;(customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;
  {{$i has bill_status "open";}} or {{$i has bill_status "paid";not {{(invoice:$i,notice:$n) isa bill_notice_owner;$n has bill_key $key;$i has bill_key $id;let $receipt=$id+"-receipt";$key==$receipt;}};}};
  $i has bill_updated $updated;select $i,$updated;distinct;sort $updated;limit 20;fetch {{"id":$i.bill_key}};"#,self.prefix())).await?;
        Ok(rows)
    }
}
fn provider_notice_kind(
    provider: &str,
    state: &str,
    due: i64,
    now: i64,
    days: i64,
) -> Option<String> {
    let kind = notice_kind(state, due, now, days)?;
    // Mercury's SendNow creation already sends the initial invoice.
    if provider == "mercury" && kind == "invoice" {
        return None;
    }
    Some(kind)
}
pub fn notice_kind(state: &str, due: i64, now: i64, days: i64) -> Option<String> {
    if state == "paid" {
        return Some("receipt".into());
    }
    if state != "open" {
        return None;
    }
    if now < due {
        return Some("invoice".into());
    }
    let bucket = (now - due) / (days * 86400);
    if bucket > 3 {
        return None;
    }
    Some(format!("reminder-{bucket}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mercury_initial_invoice_is_not_sent_twice() {
        assert!(provider_notice_kind("mercury", "open", 100, 99, 7).is_none());
        assert_eq!(
            provider_notice_kind("mercury", "open", 100, 100, 7),
            Some("reminder-0".into())
        );
        assert_eq!(
            provider_notice_kind("mercury", "paid", 100, 101, 7),
            Some("receipt".into())
        );
        assert_eq!(
            provider_notice_kind("stripe", "open", 100, 99, 7),
            Some("invoice".into())
        );
    }
    #[test]
    fn reminder_and_receipt_rules() {
        assert_eq!(notice_kind("paid", 0, 100, 7), Some("receipt".into()));
        assert!(notice_kind("processing", 0, 100, 7).is_none());
        assert_eq!(notice_kind("open", 100, 99, 7), Some("invoice".into()));
        assert_eq!(notice_kind("open", 100, 100, 7), Some("reminder-0".into()));
        assert!(notice_kind("open", 0, 86400 * 29, 7).is_none());
    }
}
