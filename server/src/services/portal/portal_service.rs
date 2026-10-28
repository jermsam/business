//! Authenticated account discovery and provider-owned payment operations.
//! Ordinary billing CRUD remains in the schema-policy data service.
use super::{portal_schema as schema, portal_shared};
use crate::{
    billing::{
        billing_schema::{quoted, Provider},
        engine::Engine,
        providers::pairs,
    },
    services::BusinessParams,
    typedb::TypeDBState,
};
use anyhow::{ensure, Result};
use dog_core::{DogError, DogService, ServiceCapabilities, TenantContext};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::Arc;
pub struct PortalService {
    state: Arc<TypeDBState>,
}
impl PortalService {
    pub fn new(state: Arc<TypeDBState>) -> Self {
        Self { state }
    }
    fn engine(&self) -> Result<Engine> {
        Engine::configured(self.state.clone())
    }
    fn permission(ctx: &TenantContext, actor: &str, customer: &str, kind: &str) -> Result<String> {
        ensure!(
            matches!(kind, "buyer" | "reader" | "seller"),
            "Invalid permission"
        );
        let customer = crate::billing::billing_schema::id(customer)?;
        Ok(format!(
            r#"$viewer isa person,has identity_id {};$scope isa tenant,has tenant_id {}; $c has bill_key "{customer}";let $allowed=bill_{kind}($viewer,$scope,$c,{});$allowed==true;"#,
            quoted(actor),
            quoted(&ctx.tenant_id.0),
            now()
        ))
    }
    async fn allowed(
        &self,
        e: &Engine,
        ctx: &TenantContext,
        actor: &str,
        customer: &str,
        kind: &str,
    ) -> Result<String> {
        let permission = Self::permission(ctx, actor, customer, kind)?;
        let rows = e
            .read(format!(
                "{} {permission} fetch {{'id':$c.bill_key}};",
                e.customer_match(customer)?
            ))
            .await?;
        if rows.len() != 1 {
            return Err(
                DogError::forbidden("You do not have access to this billing account").into_anyhow(),
            );
        }
        Ok(permission)
    }
    async fn payment(&self, e: &Engine, customer: &Value, method: &str) -> Result<Value> {
        crate::billing::providers::safe_id(method)?;
        let p = e
            .providers
            .stripe(Method::GET, &format!("/payment_methods/{method}"), "", &[])
            .await?;
        ensure!(
            p["customer"] == customer["stripe_id"]
                && matches!(p["type"].as_str(), Some("card" | "us_bank_account")),
            "Payment method does not belong to this account"
        );
        Ok(p)
    }
}
fn now() -> String {
    chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}
#[async_trait::async_trait]
impl DogService<Value, BusinessParams> for PortalService {
    fn capabilities(&self) -> ServiceCapabilities {
        portal_shared::capabilities()
    }
    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        let actor = schema::actor(ctx, &params)?;
        let e = self.engine()?;
        let query = format!(
            r#"match $u isa person,has identity_id {}; $t isa tenant,has tenant_id {};let $active=active_member($u,$t,{});$active==true;let $owner=role_at_least($u,$t,"owner",{});fetch {{"email":$u.email,"tenant_id":$t.tenant_id,"owner":$owner,"merchant_id":{},"live":{},"companies":[match $company isa tenant;let $member=active_member($u,$company,{});$member==true;fetch {{"id":$company.tenant_id,"domain":$company.name}};]}};"#,
            quoted(&actor),
            quoted(&ctx.tenant_id.0),
            now(),
            now(),
            quoted(&e.merchant),
            e.providers.live,
            now()
        );
        e.read(query).await
    }
    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        let actor = schema::actor(ctx, &params)?;
        let e = self.engine()?;
        let action = schema::field(&data, "action")?;
        if matches!(action, "invite" | "renew_invite" | "revoke_invite") {
            ensure!(ctx.tenant_id.0 == e.merchant, "Merchant account required");
            let scope=format!("$viewer isa person,has identity_id {};let $owner=role_at_least($viewer,$t,\"owner\",{});$owner==true;",quoted(&actor),now());
            return e.invitation(&scope, &data).await;
        }
        if action == "history" || action == "reconcile_invoice" {
            let id = crate::billing::billing_schema::id(schema::field(&data, "invoice_id")?)?;
            let rows=e.read(format!("{} fetch {{'customer':$c.bill_key,'status':$i.bill_status,'amount':$i.bill_amount,'provider':$i.bill_provider,'updated':$i.bill_updated}};",e.invoice_match(&id)?)).await?;
            let row = rows
                .first()
                .ok_or_else(|| DogError::not_found("Invoice not found").into_anyhow())?;
            self.allowed(
                &e,
                ctx,
                &actor,
                schema::field(row, "customer")?,
                if action == "history" {
                    "reader"
                } else {
                    "seller"
                },
            )
            .await?;
            if action == "reconcile_invoice" {
                e.reconcile(&id).await?;
                return Ok(json!({"verified":true}));
            }
            let binding = e
                .read(format!(
                    "{} fetch {{'external':$i.bill_external_id}};",
                    e.invoice_match(&id)?
                ))
                .await?;
            let external = schema::field(&binding[0], "external")?;
            let provider = if row["provider"] == "stripe" {
                Provider::Stripe
            } else {
                Provider::Mercury
            };
            if provider == Provider::Mercury {
                return e.providers.payment_history(provider, &json!({})).await;
            }
            let invoice = e.providers.observe(provider, external).await?;
            e.validate_observation(&id, provider, &invoice).await?;
            return e.providers.payment_history(provider, &invoice).await;
        }
        let id = crate::billing::billing_schema::id(schema::field(&data, "customer_id")?)?;
        let permission = self.allowed(&e, ctx, &actor, &id, "buyer").await?;
        let customer = e.customer(&id).await?;
        let stripe = schema::field(&customer, "stripe_id").unwrap_or("");
        match action {
            "payment_methods" => {
                if stripe.is_empty() {
                    return Ok(json!({"methods":[]}));
                }
                let mut methods = Vec::new();
                for kind in ["card", "us_bank_account"] {
                    let list = e
                        .providers
                        .stripe(
                            Method::GET,
                            &format!("/customers/{stripe}/payment_methods"),
                            "",
                            &pairs(&[("type", kind), ("limit", "100")]),
                        )
                        .await?;
                    for p in list["data"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("Invalid payment methods"))?
                    {
                        ensure!(p["customer"] == stripe, "Wrong customer binding");
                        let part = &p[kind];
                        let pending = e
                            .read(format!(
                                "{} $c has bill_removed_method {};fetch {{'id':$c.bill_key}};",
                                e.customer_match(&id)?,
                                quoted(schema::field(p, "id")?)
                            ))
                            .await?;
                        methods.push(json!({"id":p["id"],"type":kind,"brand":part["brand"],"bank_name":part["bank_name"],"last4":part["last4"],"exp_month":part["exp_month"],"exp_year":part["exp_year"],"is_default":p["id"]==customer["payment_method"],"removal_pending":!pending.is_empty(),"automatic_enabled":pending.is_empty() && p["id"]==customer["payment_method"] && customer["choice"]=="automatic" && customer["consent"]==customer["revision"] && customer["policy"]!="manual"}));
                    }
                }
                return Ok(json!({"methods":methods}));
            }
            "setup_card" => {
                ensure!(data["save_only"] == true, "Save a payment method first");
                let external = e.ensure_customer(&customer, Provider::Stripe).await?;
                let base = std::env::var("BILLING_PUBLIC_URL")?;
                let base = reqwest::Url::parse(&base)?;
                ensure!(
                    base.scheme() == "https" && base.host_str() == Some("jitpomi.com"),
                    "Invalid payment return destination"
                );
                let success = format!(
                    "https://jitpomi.com/account/?setup={{CHECKOUT_SESSION_ID}}&customer={id}"
                );
                let key = format!("save-method-{}", uuid::Uuid::new_v4());
                let method = if data["bank_account"] == true {
                    "us_bank_account"
                } else {
                    "card"
                };
                let mut form = pairs(&[
                    ("mode", "setup"),
                    ("customer", &external),
                    ("currency", "usd"),
                    ("payment_method_types[0]", method),
                    ("success_url", &success),
                    ("cancel_url", "https://jitpomi.com/account/"),
                    ("metadata[business_customer]", &id),
                    ("metadata[purpose]", "save_only"),
                ]);
                if method == "us_bank_account" {
                    form.extend(pairs(&[("payment_method_options[us_bank_account][financial_connections][permissions][0]","payment_method")]));
                }
                let result = e
                    .providers
                    .stripe(Method::POST, "/checkout/sessions", &key, &form)
                    .await?;
                let url = schema::field(&result, "url")?;
                ensure!(
                    url.starts_with("https://checkout.stripe.com/"),
                    "Unexpected checkout URL"
                );
                return Ok(json!({"url":url}));
            }
            "confirm_card" => {
                let session = schema::field(&data, "session_id")?;
                crate::billing::providers::safe_id(session)?;
                let checkout = e
                    .providers
                    .stripe(
                        Method::GET,
                        &format!("/checkout/sessions/{session}"),
                        "",
                        &[],
                    )
                    .await?;
                ensure!(
                    checkout["mode"] == "setup"
                        && checkout["status"] == "complete"
                        && checkout["customer"] == stripe
                        && checkout["metadata"]["business_customer"] == id
                        && checkout["metadata"]["purpose"] == "save_only",
                    "Setup not complete for this account"
                );
                let intent = schema::field(&checkout, "setup_intent")?;
                crate::billing::providers::safe_id(intent)?;
                let setup = e
                    .providers
                    .stripe(Method::GET, &format!("/setup_intents/{intent}"), "", &[])
                    .await?;
                ensure!(
                    setup["status"] == "succeeded" && setup["customer"] == stripe,
                    "Payment method is not verified yet"
                );
                let method = schema::field(&setup, "payment_method")?;
                self.payment(&e, &customer, method).await?;
                if customer["payment_method"] == "" {
                    e.write(format!(r#"{} {permission} $c has bill_payment_method "",has bill_revision {};select $t,$c;distinct;update $c has bill_payment_method {},has bill_revision {},has bill_consent "",has bill_choice "manual";fetch {{"id":$c.bill_key}};"#,e.customer_match(&id)?,quoted(schema::field(&customer,"revision")?),quoted(method),quoted(&uuid::Uuid::new_v4().to_string()))).await?;
                }
                return Ok(json!({"setup_purpose":"save_only"}));
            }
            "authorize_saved" | "set_default" | "remove_method" => {}
            _ => return Err(DogError::bad_request("Action unavailable").into_anyhow()),
        }
        let revision = schema::field(&data, "expected_revision")?;
        ensure!(
            customer["revision"] == revision,
            "Payment settings changed. Refresh and review them again."
        );
        let next = uuid::Uuid::new_v4().to_string();
        if action == "authorize_saved" {
            ensure!(
                customer["policy"] != "manual",
                "Automatic payments are not available for this account"
            );
            let method = schema::field(&customer, "payment_method")?;
            self.payment(&e, &customer, method).await?;
            let removed = e
                .read(format!(
                    "{} $c has bill_removed_method {};fetch {{'id':$c.bill_key}};",
                    e.customer_match(&id)?,
                    quoted(method)
                ))
                .await?;
            ensure!(removed.is_empty(), "Payment method is being removed");
            let timestamp = chrono::Utc::now().timestamp();
            e.write(format!(r#"{} {permission} $c has bill_revision {}; (customer:$c,plan:$p) isa bill_plan_owner;$p has bill_status "active";select $t,$c,$viewer;distinct;update $c has bill_choice "automatic",has bill_consent {},has bill_revision {};insert $e isa bill_consent_evidence,has bill_key {},has bill_revision {},has bill_updated {timestamp};(customer:$c,actor:$viewer,evidence:$e) isa bill_consent_owner;fetch {{"id":$c.bill_key}};"#,e.customer_match(&id)?,quoted(revision),quoted(&next),quoted(&next),quoted(&next),quoted(&next))).await?;
            return Ok(json!({"authorized":true}));
        }
        let method = schema::field(&data, "method_id")?;
        self.payment(&e, &customer, method).await?;
        if action == "set_default" {
            let removed = e
                .read(format!(
                    "{} $c has bill_removed_method {};fetch {{'id':$c.bill_key}};",
                    e.customer_match(&id)?,
                    quoted(method)
                ))
                .await?;
            ensure!(removed.is_empty(), "Payment method is being removed");
            e.write(format!(r#"{} {permission} $c has bill_revision {};select $t,$c;distinct;update $c has bill_payment_method {},has bill_choice "manual",has bill_consent "",has bill_revision {};fetch {{"id":$c.bill_key}};"#,e.customer_match(&id)?,quoted(revision),quoted(method),quoted(&next))).await?;
        } else {
            let default = customer["payment_method"] == method;
            let update = if default {
                format!(
                    r#"update $c has bill_payment_method "",has bill_choice "manual",has bill_consent "",has bill_revision {};"#,
                    quoted(&next)
                )
            } else {
                format!("update $c has bill_revision {};", quoted(&next))
            };
            e.write(format!(r#"{} {permission} $c has bill_revision {};select $t,$c;distinct;{update} put $c has bill_removed_method {};fetch {{"id":$c.bill_key}};"#,e.customer_match(&id)?,quoted(revision),quoted(method))).await?;
            e.providers
                .stripe(
                    Method::POST,
                    &format!("/payment_methods/{method}/detach"),
                    &format!("detach-{id}-{method}"),
                    &[],
                )
                .await?;
        }
        Ok(json!({"updated":true}))
    }
}
