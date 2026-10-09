//! Customer-only hosted card setup. Redirects are never evidence of consent/payment.
use super::{
    billing_schema as schema,
    billing_service::CUSTOMER,
    engine::{string, Engine, CUSTOMER_DATA},
    providers::{self},
};
use crate::{
    services::{authentication::AuthService, workspace::WorkspaceService, BusinessParams},
    typedb::TypeDBState,
};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use dog_core::{
    tenant::TenantContext, DogError, DogService, ServiceCapabilities, ServiceMethodKind,
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
pub struct Actions {
    access: WorkspaceService,
    state: Arc<TypeDBState>,
}
impl Actions {
    pub fn new(state: Arc<TypeDBState>, auth: Arc<AuthService>) -> Self {
        Self {
            access: WorkspaceService::new(state.clone(), auth),
            state,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    History {
        invoice_id: String,
    },
    ReconcileInvoice {
        invoice_id: String,
        provider_invoice_id: String,
    },
    SetupCard {
        customer_id: String,
        expected_revision: String,
    },
    ConfirmCard {
        customer_id: String,
        session_id: String,
    },
}
#[async_trait]
impl DogService<Value, BusinessParams> for Actions {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities::from_methods(vec![ServiceMethodKind::Create])
    }
    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        let scope = self.access.scope(ctx, params).await?;
        let action: Action = schema::parse(data)?;
        if let Action::History { invoice_id } = &action {
            let invoice_id = schema::id(invoice_id)?;
            let engine = Engine::configured(self.state.clone())?;
            let permitted = format!(
                r#"{scope} $i isa bill_invoice,has bill_key "{invoice_id}";(invoice:$i,plan:$p) isa bill_invoice_owner;(plan:$p,customer:$c) isa bill_plan_owner;let $allowed=bill_reader($u,$t,$c,$now);$allowed==true;"#
            );
            let rows=self.access.query(format!(r#"{permitted} fetch {{"provider":$i.bill_provider,"external":$i.bill_external_id}};"#),false).await?;
            if rows.len() != 1 {
                return Err(DogError::not_found("Invoice unavailable").into_anyhow());
            }
            let provider: schema::Provider = serde_json::from_value(rows[0]["provider"].clone())?;
            let external = providers::field(&rows[0], "external")?;
            ensure!(!external.is_empty(), "Invoice is not yet issued");
            let observation = engine.providers.observe(provider, external).await?;
            engine
                .validate_observation(&invoice_id, provider, &observation)
                .await?;
            let history = engine
                .providers
                .payment_history(provider, &observation)
                .await?;
            self.access.query(format!(r#"{permitted} select $t,$i;distinct;update $i has bill_history {};fetch {{"id":$i.bill_key}};"#,schema::quoted(&history.to_string())),true).await?;
            return Ok(history);
        }
        if let Action::ReconcileInvoice {
            invoice_id,
            provider_invoice_id,
        } = &action
        {
            let invoice_id = schema::id(invoice_id)?;
            providers::safe_id(provider_invoice_id)?;
            let engine = Engine::configured(self.state.clone())?;
            let permitted = format!(
                r#"{scope} $t has biz_id "{}";
                $i isa bill_invoice,has bill_key "{invoice_id}";
                (invoice:$i,plan:$p) isa bill_invoice_owner;(plan:$p,customer:$c) isa bill_plan_owner;
                let $allowed=bill_seller($u,$t,$c,$now);$allowed==true;
                fetch {{"provider":$i.bill_provider}};"#,
                engine.merchant
            );
            let rows = self.access.query(permitted, false).await?;
            if rows.len() != 1 {
                return Err(DogError::not_found("Invoice unavailable").into_anyhow());
            }
            let provider: schema::Provider = serde_json::from_value(rows[0]["provider"].clone())?;
            let observation = engine
                .providers
                .observe(provider, provider_invoice_id)
                .await?;
            // Binds customer, provider, exact amount and local invoice metadata before writing.
            // This action never creates an invoice or attempts a payment.
            let permission = format!(
                r#"{scope} $i isa bill_invoice,has bill_key "{invoice_id}";(invoice:$i,plan:$p) isa bill_invoice_owner;(plan:$p,customer:$c) isa bill_plan_owner;let $allowed=bill_seller($u,$t,$c,$now);$allowed==true;"#
            );
            engine
                .apply_observation_guarded(&invoice_id, provider, &observation, &permission)
                .await?;
            return Ok(json!({"reconciled":true}));
        }
        let customer_id = match &action {
            Action::SetupCard { customer_id, .. } | Action::ConfirmCard { customer_id, .. } => {
                schema::id(customer_id)?
            }
            Action::ReconcileInvoice { .. } | Action::History { .. } => unreachable!(),
        };
        let permitted = format!(
            r#"{scope} $c isa bill_customer,has bill_key "{customer_id}";let $allowed=bill_buyer($u,$t,$c,$now);$allowed==true;"#
        );
        let rows = self
            .access
            .query(
                format!(r#"{permitted} fetch {{{CUSTOMER_DATA},"actor":$u.biz_id}};"#),
                false,
            )
            .await?;
        if rows.len() != 1 {
            return Err(DogError::not_found("Billing customer unavailable").into_anyhow());
        }
        let row = &rows[0];
        let engine = Engine::configured(self.state.clone())?;
        // Check the configured merchant before passing this customer's data to Stripe.
        engine.customer(&customer_id).await?;
        let actor = string(row, "actor")?;
        let revision = string(row, "revision")?;
        match action {
            Action::ReconcileInvoice { .. } | Action::History { .. } => unreachable!(),
            Action::SetupCard {
                expected_revision, ..
            } => {
                ensure!(
                    expected_revision == revision,
                    "Billing changed; reload and review the schedules before authorizing"
                );
                ensure!(
                    row["policy"] != "manual",
                    "Seller has disabled automatic payments"
                );
                let customer = engine
                    .ensure_customer(row, schema::Provider::Stripe)
                    .await?;
                let result = engine
                    .providers
                    .setup(&customer, &customer_id, revision, actor)
                    .await?;
                let url = providers::field(&result, "url")?;
                providers::trusted_url(url, "checkout.stripe.com")?;
                Ok(json!({"url":url,"status":"awaiting_customer"}))
            }
            Action::ConfirmCard { session_id, .. } => {
                providers::safe_id(&session_id)?;
                let session = engine
                    .providers
                    .stripe(
                        Method::GET,
                        &format!("/checkout/sessions/{session_id}"),
                        "",
                        &[],
                    )
                    .await?;
                ensure!(
                    session["mode"] == "setup"
                        && session["status"] == "complete"
                        && session["customer"] == row["stripe_id"]
                        && session["client_reference_id"] == customer_id,
                    "Setup does not belong to this customer"
                );
                let intent_id = providers::field(&session, "setup_intent")?;
                providers::safe_id(intent_id)?;
                let intent = engine
                    .providers
                    .stripe(Method::GET, &format!("/setup_intents/{intent_id}"), "", &[])
                    .await?;
                ensure!(
                    intent["status"] == "succeeded"
                        && intent["usage"] == "off_session"
                        && intent["customer"] == row["stripe_id"]
                        && intent["metadata"]["business_customer"] == customer_id
                        && intent["metadata"]["business_revision"] == revision
                        && intent["metadata"]["business_actor"] == actor,
                    "Setup is unverified or billing terms changed; review and save again"
                );
                let card = providers::field(&intent, "payment_method")?;
                providers::safe_id(card)?;
                let method = engine
                    .providers
                    .stripe(Method::GET, &format!("/payment_methods/{card}"), "", &[])
                    .await?;
                ensure!(
                    method["type"] == "card" && method["customer"] == row["stripe_id"],
                    "Card does not belong to customer"
                );
                // Same customer revision participates in all preference/schedule writes.
                // The SetupIntent is retained in evidence for auditing, never PAN or CVC.
                let now = chrono::Utc::now().timestamp();
                let query = format!(
                    r#"{permitted} $c has bill_revision {},has bill_policy $policy;$policy!="manual";
     not {{$e isa bill_consent_evidence,has bill_key {};}};
     (merchant:$merchant,customer:$c) isa bill_account;select $t,$merchant,$u,$c;distinct;update $merchant has biz_revision "{intent_id}";update $c has bill_choice "automatic",has bill_consent {},has bill_payment_method {};
     insert $e isa bill_consent_evidence,has bill_key {},has bill_revision {},has bill_updated {now};(customer:$c,actor:$u,evidence:$e) isa bill_consent_owner;{CUSTOMER}"#,
                    schema::quoted(revision),
                    schema::quoted(intent_id),
                    schema::quoted(revision),
                    schema::quoted(card),
                    schema::quoted(intent_id),
                    schema::quoted(revision)
                );
                let mut rows = self.access.query(query, true).await?;
                ensure!(
                    rows.len() == 1,
                    "Billing changed or setup was already confirmed"
                );
                Ok(rows.remove(0))
            }
        }
    }
}
