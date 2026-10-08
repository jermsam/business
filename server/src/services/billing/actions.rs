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
    SetupCard {
        customer_id: String,
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
        let customer_id = match &action {
            Action::SetupCard { customer_id } | Action::ConfirmCard { customer_id, .. } => {
                schema::id(customer_id)?
            }
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
            Action::SetupCard { .. } => {
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
