use super::{
    onboarding_schema::{self, Input},
    onboarding_shared::digest,
};
use crate::services::billing::{
    billing_schema::{parse, quoted, text},
    engine::Engine,
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
use serde_json::{json, Value};
use std::sync::Arc;
pub struct OnboardingService {
    access: WorkspaceService,
    state: Arc<TypeDBState>,
}
impl OnboardingService {
    pub fn new(state: Arc<TypeDBState>, auth: Arc<AuthService>) -> Self {
        Self {
            access: WorkspaceService::new(state.clone(), auth),
            state,
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for OnboardingService {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities::from_methods(vec![ServiceMethodKind::Create, ServiceMethodKind::Find])
    }
    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        let scope = self.access.scope(ctx, params).await?;
        let merchant =
            crate::services::billing::billing_schema::id(&std::env::var("BILLING_MERCHANT_ID")?)?;
        let live = std::env::var("BILLING_LIVE").as_deref() == Ok("true");
        self.access.query(format!(r#"{scope} let $owner=biz_owner($u,$t,$now);fetch {{"email":$u.email,"tenant_id":$t.biz_id,"owner":$owner,"merchant_id":"{merchant}","live":{live}}};"#),false).await
    }
    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        match parse::<Input>(data)? {
            Input::Invite { name, email } => {
                text(&name, 120)?;
                let email = onboarding_schema::email(&email)?;
                let scope = self.access.scope(ctx, params).await?;
                let engine = Engine::configured(self.state.clone())?;
                let merchant = &engine.merchant;
                let customer = uuid::Uuid::new_v4().to_string();
                let buyer = uuid::Uuid::new_v4().to_string();
                let invitation = uuid::Uuid::new_v4().to_string();
                let token = format!(
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                );
                let hash = digest(&token);
                let expires = chrono::Utc::now().timestamp() + 48 * 3600;
                let q = format!(
                    r#"{scope} $t has biz_id "{merchant}";let $owner=biz_owner($u,$t,$now);$owner==true;select $t;distinct;
    insert $buyer isa company,has biz_id "{buyer}",has company_domain "{buyer}.customer.jitpomi.com",has biz_revision "{invitation}";
    $c isa bill_customer,has bill_key "{customer}",has name {},has bill_email {},has bill_policy "customer",has bill_choice "manual",has bill_provider "stripe",has bill_stripe_id "",has bill_mercury_id "",has bill_payment_method "",has bill_consent "",has bill_revision "{invitation}";
    (merchant:$t,buyer:$buyer,customer:$c) isa bill_account;
    $invite isa portal_invitation,has biz_id "{invitation}",has portal_token_hash "{hash}",has portal_expires {expires},has portal_used false,has email {};
    (invitation:$invite,tenant:$buyer) isa portal_invitation_owner;fetch {{"id":$c.bill_key}};"#,
                    quoted(&name),
                    quoted(&email),
                    quoted(&email)
                );
                self.access.query(q, true).await?;
                Ok(
                    json!({"customer_id":customer,"tenant_id":buyer,"token":token,"expires_at":expires}),
                )
            }
            action @ (Input::RevokeInvite { .. } | Input::RenewInvite { .. }) => {
                let (customer_id, renew) = match action {
                    Input::RevokeInvite { customer_id } => (customer_id, false),
                    Input::RenewInvite { customer_id } => (customer_id, true),
                    _ => unreachable!(),
                };
                let customer = crate::services::billing::billing_schema::id(&customer_id)?;
                let scope = self.access.scope(ctx, params).await?;
                let merchant = Engine::configured(self.state.clone())?.merchant;
                let token = format!(
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                );
                let hash = digest(&token);
                let expires = if renew {
                    chrono::Utc::now().timestamp() + 48 * 3600
                } else {
                    0
                };
                let query = format!(
                    r#"{scope} $t has biz_id "{merchant}";
                    $c isa bill_customer,has bill_key "{customer}";
                    let $allowed=bill_seller($u,$t,$c,$now);$allowed==true;
                    (merchant:$t,buyer:$buyer,customer:$c) isa bill_account;
                    (tenant:$buyer,invitation:$invite) isa portal_invitation_owner;
                    $invite has portal_used false;
                    select $t,$invite,$buyer;distinct;
                    update $invite has portal_token_hash "{hash}",has portal_expires {expires};
                    fetch {{"tenant_id":$buyer.biz_id}};"#
                );
                let rows = self.access.query(query, true).await?;
                ensure!(
                    rows.len() == 1,
                    "Invitation unavailable or already accepted"
                );
                if renew {
                    Ok(json!({"token":token,"expires_at":expires,"tenant_id":rows[0]["tenant_id"]}))
                } else {
                    Ok(json!({"revoked":true}))
                }
            }
            Input::Accept { token, password } => {
                ensure!(
                    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Invalid invitation"
                );
                ensure!(
                    (12..=72).contains(&password.len()),
                    "Use a password of 12–72 bytes"
                );
                let hash = digest(&token);
                let now = chrono::Utc::now().timestamp();
                let selection = format!(
                    r#"match $invite isa portal_invitation,has portal_token_hash "{hash}",has portal_used false,has portal_expires $expires,has email $email;$expires>{now};(invitation:$invite,tenant:$t) isa portal_invitation_owner;$t has biz_id $tenant;not {{$t has biz_state "suspended";}};"#
                );
                let rows = self
                    .access
                    .query(
                        format!(r#"{selection} fetch {{"email":$email,"tenant":$tenant}};"#),
                        false,
                    )
                    .await?;
                if rows.len() != 1 {
                    return Err(
                        DogError::not_authenticated("Invitation expired or already used")
                            .into_anyhow(),
                    );
                }
                let email = rows[0]["email"].as_str().unwrap();
                let existing=self.access.query(format!(r#"match $u isa user,has email {};fetch {{"id":$u.biz_id,"password":$u.password}};"#,quoted(email)),false).await?;
                ensure!(existing.len() <= 1, "Ambiguous identity");
                let user_id = if existing.is_empty() {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    existing[0]["id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Account unavailable"))?
                        .to_owned()
                };
                let user_id = crate::services::billing::billing_schema::id(&user_id)?;
                let user_clause = if let Some(user) = existing.first() {
                    let old = user["password"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Account unavailable"))?
                        .to_owned();
                    let old_check = old.clone();
                    ensure!(
                        tokio::task::spawn_blocking(move || bcrypt::verify(password, &old_check))
                            .await??,
                        "For an existing account, use its current password"
                    );
                    format!(
                        r#"match $u isa user,has email {},has biz_id "{user_id}",has password {};not {{$u has biz_state "suspended";}};"#,
                        quoted(email),
                        quoted(&old)
                    )
                } else {
                    let password_hash =
                        tokio::task::spawn_blocking(move || bcrypt::hash(password, 12)).await??;
                    format!(
                        r#"match not {{$existing isa user,has email {};}};insert $u isa user,has email {},has biz_id "{user_id}",has password {};"#,
                        quoted(email),
                        quoted(email),
                        quoted(&password_hash)
                    )
                };
                let tenant = rows[0]["tenant"].as_str().unwrap();
                let membership = crate::access::pair(tenant, &user_id)?;
                let q = format!(
                    r#"{selection} {user_clause}
    update $invite has portal_used true;
    insert $m isa biz_membership,has biz_id "{membership}",has biz_state "active",has biz_role "owner",has biz_start {},links (tenant:$t,person:$u);
    fetch {{"tenant_id":$t.biz_id,"email":$u.email}};"#,
                    crate::auth::local::now()
                );
                let mut result = self.access.query(q, true).await?;
                ensure!(result.len() == 1, "Invitation unavailable");
                Ok(result.remove(0))
            }
        }
    }
}
