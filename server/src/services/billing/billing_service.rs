use super::{
    billing_schema as schema,
    billing_shared::{self, Kind},
};
use crate::{
    services::{workspace::WorkspaceService, BusinessParams},
    typedb::TypeDBState,
};
use async_trait::async_trait;
use dog_core::{tenant::TenantContext, DogError, DogService, ServiceCapabilities};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

pub struct BillingService {
    pub access: WorkspaceService,
    pub kind: Kind,
}
impl BillingService {
    pub fn new(
        state: Arc<TypeDBState>,
        auth: Arc<crate::services::authentication::AuthService>,
        kind: Kind,
    ) -> Self {
        Self {
            access: WorkspaceService::new(state, auth),
            kind,
        }
    }
    fn selection(&self, id: Option<&str>) -> anyhow::Result<String> {
        let target = match self.kind {
            Kind::Customers => "$c",
            Kind::Plans => "$p",
            Kind::Invoices => "$i",
        };
        let clause = if let Some(id) = id {
            format!(
                "{target} has bill_key {};",
                schema::quoted(&schema::id(id)?)
            )
        } else {
            String::new()
        };
        let relation=match self.kind{Kind::Customers=>"",Kind::Plans=>"$p isa bill_plan; (customer:$c,plan:$p) isa bill_plan_owner;",Kind::Invoices=>"$i isa bill_invoice; (plan:$p,invoice:$i) isa bill_invoice_owner; (customer:$c,plan:$p) isa bill_plan_owner;"};
        Ok(format!("$c isa bill_customer; {relation} {clause} let $allowed=bill_reader($u,$t,$c,$now); $allowed==true;"))
    }
    fn projection(&self) -> &'static str {
        match self.kind {
            Kind::Customers => CUSTOMER,
            Kind::Plans => PLAN,
            Kind::Invoices => INVOICE,
        }
    }
    async fn one(&self, query: String, write: bool) -> anyhow::Result<Value> {
        let mut rows = self.access.query(query, write).await?;
        if rows.len() != 1 {
            return Err(DogError::not_found("Billing resource unavailable").into_anyhow());
        }
        Ok(rows.remove(0))
    }
}
pub const CUSTOMER: &str = r#"fetch {"id":$c.bill_key,"name":$c.name,"email":$c.bill_email,"policy":$c.bill_policy,"choice":$c.bill_choice,"manual_provider":$c.bill_provider};"#;
pub const PLAN: &str = r#"fetch {"id":$p.bill_key,"customer_id":$c.bill_key,"description":$p.name,"amount_cents":$p.bill_amount,"currency":"USD","start_at":$p.bill_anchor,"next_at":$p.bill_next,"interval":$p.bill_interval,"due_days":$p.bill_due_days,"status":$p.bill_status};"#;
pub const INVOICE: &str = r#"fetch {"id":$i.bill_key,"customer_id":$c.bill_key,"description":$i.name,"amount_cents":$i.bill_amount,"currency":"USD","issued_at":$i.bill_anchor,"due_at":$i.bill_next,"status":$i.bill_status,"mode":$i.bill_choice,"provider":$i.bill_provider,"payment_url":$i.bill_url};"#;
#[async_trait]
impl DogService<Value, BusinessParams> for BillingService {
    fn capabilities(&self) -> ServiceCapabilities {
        billing_shared::capabilities(self.kind)
    }
    async fn find(
        &self,
        ctx: &TenantContext,
        params: BusinessParams,
    ) -> anyhow::Result<Vec<Value>> {
        let after = params
            .inner
            .query
            .get("after")
            .map(|s| schema::id(s))
            .transpose()?;
        let cursor = after
            .map(|id| format!("$key > {};", schema::quoted(&id)))
            .unwrap_or_default();
        let scope = self.access.scope(ctx, params).await?;
        let target = match self.kind {
            Kind::Customers => "$c",
            Kind::Plans => "$p",
            Kind::Invoices => "$i",
        };
        let selected = if matches!(self.kind, Kind::Customers) {
            "$c".to_string()
        } else {
            format!("$c,{target}")
        };
        self.access.query(format!("{scope} {} {target} has bill_key $key; {cursor} select {selected}, $key; distinct; sort $key; limit 100; {}",self.selection(None)?,self.projection()),false).await
    }
    async fn get(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let scope = self.access.scope(ctx, params).await?;
        self.one(
            format!(
                "{scope} {} {}",
                self.selection(Some(id))?,
                self.projection()
            ),
            false,
        )
        .await
    }
    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let scope = self.access.scope(ctx, params).await?;
        let key = uuid::Uuid::new_v4();
        let revision = uuid::Uuid::new_v4();
        let query = match self.kind {
            Kind::Customers => {
                let input: schema::CustomerInput = schema::parse(data)?;
                input
                    .validate()
                    .map_err(|_| DogError::bad_request("Invalid customer").into_anyhow())?;
                format!(
                    r#"{scope} let $owner=biz_owner($u,$t,$now); $owner==true;
     $buyer isa company,has biz_id "{}"; select $t,$buyer; distinct;
     insert $c isa bill_customer,has bill_key "{key}",has name {},has bill_email {},has bill_policy "{}",has bill_choice "manual",has bill_provider "{}",has bill_stripe_id "",has bill_mercury_id "",has bill_payment_method "",has bill_consent "",has bill_revision "{revision}";
     (merchant:$t,buyer:$buyer,customer:$c) isa bill_account; {CUSTOMER}"#,
                    schema::id(&input.buyer_id)?,
                    schema::quoted(&input.name),
                    schema::quoted(&input.email),
                    schema::word(input.policy),
                    schema::word(input.manual_provider)
                )
            }
            Kind::Plans => {
                let input: schema::PlanInput = schema::parse(data)?;
                let start = input.validate(chrono::Utc::now()).map_err(|_| {
                    DogError::bad_request("Invalid USD amount, date, interval or due days")
                        .into_anyhow()
                })?;
                format!(
                    r#"{scope} $c isa bill_customer,has bill_key "{}"; let $owner=bill_seller($u,$t,$c,$now); $owner==true; select $t,$c; distinct;
     update $c has bill_revision "{revision}",has bill_consent "";
     insert $p isa bill_plan,has bill_key "{key}",has name {},has bill_amount {},has bill_due_days {},has bill_anchor {start},has bill_next {start},has bill_sequence 0,has bill_interval "{}",has bill_status "active";
     (customer:$c,plan:$p) isa bill_plan_owner; {PLAN}"#,
                    schema::id(&input.customer_id)?,
                    schema::quoted(&input.description),
                    input.amount_cents,
                    input.due_days,
                    schema::word(input.interval)
                )
            }
            Kind::Invoices => {
                return Err(
                    DogError::forbidden("Only the scheduler can issue an invoice").into_anyhow(),
                )
            }
        };
        self.one(query, true).await
    }
    async fn patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let id = schema::id(id.ok_or_else(|| DogError::bad_request("ID required").into_anyhow())?)?;
        let scope = self.access.scope(ctx, params).await?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Change {
            policy: Option<schema::Policy>,
            choice: Option<schema::Mode>,
            status: Option<String>,
        }
        let change: Change = schema::parse(data)?;
        let rev = uuid::Uuid::new_v4();
        let query = match self.kind {
            Kind::Customers => {
                let (policy,changes)=match (change.policy,change.choice,change.status){
     (Some(p),None,None)=>("bill_seller",format!("has bill_policy \"{}\",has bill_revision \"{rev}\",has bill_consent \"\"",schema::word(p))),
     (None,Some(c),None)=>("bill_buyer",format!("has bill_choice \"{}\",has bill_revision \"{rev}\",has bill_consent \"\"",schema::word(c))),
     _=>return Err(DogError::bad_request("Set policy as seller or choice as buyer").into_anyhow())};
                format!(
                    r#"{scope} $c isa bill_customer,has bill_key "{id}"; let $allowed={policy}($u,$t,$c,$now);$allowed==true;(merchant:$merchant,customer:$c) isa bill_account;select $t,$merchant,$c;distinct;update $merchant has biz_revision "{rev}";update $c {changes};{CUSTOMER}"#
                )
            }
            Kind::Plans => {
                if change.policy.is_some() || change.choice.is_some() {
                    return Err(DogError::bad_request("Only status can change").into_anyhow());
                }
                let status = change.status.unwrap_or_default();
                if !matches!(status.as_str(), "active" | "paused") {
                    return Err(DogError::bad_request("Use active or paused").into_anyhow());
                }
                format!(
                    r#"{scope} $p isa bill_plan,has bill_key "{id}",has bill_status $previous;$previous!="finished"; (customer:$c,plan:$p) isa bill_plan_owner; let $allowed=bill_seller($u,$t,$c,$now);$allowed==true;select $t,$c,$p;distinct;update $p has bill_status "{status}";update $c has bill_revision "{rev}",has bill_consent ""; {PLAN}"#
                )
            }
            Kind::Invoices => {
                return Err(DogError::forbidden("Invoice state is provider-verified").into_anyhow())
            }
        };
        self.one(query, true).await
    }
}
