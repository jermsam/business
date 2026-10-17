use super::{
    recovery_schema::{self, Input},
    recovery_shared::{digest, token},
};
use crate::{
    services::{
        billing::{
            billing_schema::{parse, quoted},
            providers::Providers,
        },
        BusinessParams,
    },
    typedb::TypeDBState,
};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use dog_core::{
    tenant::TenantContext, DogError, DogService, ServiceCapabilities, ServiceMethodKind,
};
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock};
static PASSWORD_WORK: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(2));
pub struct RecoveryService {
    state: Arc<TypeDBState>,
}
impl RecoveryService {
    pub fn new(state: Arc<TypeDBState>) -> Self {
        Self { state }
    }
    async fn read(&self, query: String) -> Result<Vec<Value>> {
        let result = dog_typedb::TypeDBAdapter::new(self.state.clone())
            .read(json!({"query":query}))
            .await?;
        Ok(result["ok"]["answers"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Recovery lookup unavailable"))?
            .iter()
            .map(|v| v["data"].clone())
            .collect())
    }
    async fn write(&self, query: String) -> Result<Vec<Value>> {
        crate::access::write_atomic_one(&self.state, &query).await
    }
    pub async fn request(&self, email: &str) -> Result<()> {
        let email = crate::services::onboarding::onboarding_schema::email(email)?;
        let now = chrono::Utc::now().timestamp();
        let key = digest(&format!("{email}:{}", now / 600));
        // Same durable operation for existing and nonexistent accounts. No account lookup
        // or email request is performed on this public request path.
        let query = format!(
            r#"match $r isa portal_recovery,has auth_request_key "{key}";fetch {{"id":$r.biz_id}};"#
        );
        if !self.read(query.clone()).await?.is_empty() {
            return Ok(());
        }
        let id = uuid::Uuid::new_v4();
        let expires = now + 3600;
        let result=self.write(format!(r#"insert $r isa portal_recovery,has biz_id "{id}",has auth_request_key "{key}",has email {},has portal_expires {expires},has portal_used false,has auth_status "queued";fetch {{"id":$r.biz_id}};"#,quoted(&email))).await;
        if result.is_err() && self.read(query).await?.is_empty() {
            result?;
        }
        Ok(())
    }
    async fn reset(&self, raw: &str, password: String, confirmation: &str) -> Result<()> {
        recovery_schema::password(&password, confirmation)?;
        ensure!(
            raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid recovery link"
        );
        let hash = digest(raw);
        let now = chrono::Utc::now().timestamp();
        let selection = format!(
            r#"match $r isa portal_recovery,has portal_token_hash "{hash}",has portal_used false,has portal_expires $expiry,has auth_version $version;$expiry>{now};(recovery:$r,person:$u) isa portal_recovery_owner;not {{$u has biz_state "suspended";}};not {{$u has auth_version $changed;$changed!=$version;}};"#
        );
        let rows = self
            .read(format!(r#"{selection} fetch {{"id":$u.biz_id}};"#))
            .await?;
        if rows.len() != 1 {
            return Err(
                DogError::not_authenticated("Recovery link expired or already used").into_anyhow(),
            );
        }
        let _slot = PASSWORD_WORK.acquire().await?;
        let password = tokio::task::spawn_blocking(move || bcrypt::hash(password, 12)).await??;
        let version = uuid::Uuid::new_v4();
        let revision = uuid::Uuid::new_v4();
        // Same transaction consumes the token, changes credentials, and conflicts with
        // protected writes in every existing membership. All older recovery links and
        // sessions carry the old version and cannot be reused after this commit.
        self.write(format!(r#"{selection} update $u has password {},has auth_version "{version}";$r has portal_used true,has auth_status "reset_pending";
   match (person:$u,tenant:$t) isa biz_membership;$t has biz_revision $previous;
   update $t has biz_revision "{revision}";select $r;distinct;fetch {{"id":$r.biz_id}};"#,quoted(&password))).await?;
        Ok(())
    }
    pub(crate) async fn claim_link(&self, id: &str, user: &str, version: &str) -> Result<()> {
        let id = uuid::Uuid::parse_str(id)?;
        let user = uuid::Uuid::parse_str(user)?;
        let hash = digest(&token(&id.to_string(), version)?);
        self.write(format!(r#"match $r isa portal_recovery,has biz_id "{id}",has auth_status "queued";$u isa user,has biz_id "{user}";not {{$u has auth_version $changed;$changed!={};}};update $r has auth_status "sending",has portal_token_hash "{hash}",has auth_version {};insert (recovery:$r,person:$u) isa portal_recovery_owner;fetch {{"id":$r.biz_id}};"#,quoted(version),quoted(version))).await?;
        Ok(())
    }
    pub async fn process_pending(&self) -> Result<(usize, usize)> {
        if std::env::var("BILLING_MAIL_ENABLED").as_deref() != Ok("true") {
            return Ok((0, 0));
        }
        let now = chrono::Utc::now().timestamp();
        let rows=self.read(format!(r#"match $r isa portal_recovery,has auth_status $status,has portal_expires $expires;{{$status=="queued";$expires>{now};}} or {{$status=="reset_pending";}};select $r,$expires;sort $expires;limit 20;fetch {{"id":$r.biz_id,"email":$r.email,"status":$r.auth_status}};"#)).await?;
        let mut sent = 0;
        let mut errors = 0;
        for row in rows {
            match self.send(&row).await {
                Ok(n) => sent += n,
                Err(_) => errors += 1,
            }
            if sent + errors > 0 {
                break;
            }
        }
        Ok((sent, errors))
    }
    async fn send(&self, row: &Value) -> Result<usize> {
        let id = row["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid recovery record"))?;
        let email = row["email"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid recovery recipient"))?;
        let confirm = row["status"] == "reset_pending";
        let users=self.read(format!(r#"match $u isa user,has email {};not {{$u has biz_state "suspended";}};fetch {{"id":$u.biz_id,"versions":[$u.auth_version]}};"#,quoted(email))).await?;
        let providers = Providers::new()?;
        let allowed = providers.live || std::env::var("BILLING_TEST_EMAIL").as_deref() == Ok(email);
        if users.len() != 1 || !allowed {
            self.write(format!(r#"match $r isa portal_recovery,has biz_id "{id}",has auth_status {};update $r has auth_status "ignored";fetch {{"id":$r.biz_id}};"#,quoted(row["status"].as_str().unwrap()))).await?;
            return Ok(0);
        }
        let user = users[0]["id"].as_str().unwrap();
        let version = users[0]
            .pointer("/versions/0")
            .and_then(Value::as_str)
            .unwrap_or("initial");
        let raw = token(id, version)?;
        let base = std::env::var("BILLING_PUBLIC_URL")?;
        let url = reqwest::Url::parse(&base)?;
        ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.query().is_none(),
            "Trusted recovery URL required"
        );
        let (subject, body, status) = if confirm {
            ("Your JITPOMI password was changed", "Your password was changed and your earlier sessions have been invalidated. If this was not you, contact dev@jitpomi.com immediately.".to_owned(),"confirming")
        } else {
            ("Reset your JITPOMI password",format!("Use this private, single-use link within one hour to choose a new password:\n{base}#reset={raw}\n\nIf you did not request this, ignore this email. Your password has not changed."),"sending")
        };
        let api_key = std::env::var("RESEND_API_KEY")?;
        let sender = std::env::var("BILLING_EMAIL_FROM")?;
        if confirm {
            self.write(format!(r#"match $r isa portal_recovery,has biz_id "{id}",has auth_status "reset_pending";update $r has auth_status "confirming";fetch {{"id":$r.biz_id}};"#)).await?;
        } else {
            self.claim_link(id, user, version).await?;
        }
        let result = providers
            .email(
                &api_key,
                &sender,
                email,
                &if providers.live {
                    subject.to_owned()
                } else {
                    format!("[TEST] {subject}")
                },
                &body,
                &format!("recovery-{id}-{status}"),
            )
            .await;
        let terminal = if result.is_ok() {
            if confirm {
                "confirmed"
            } else {
                "sent"
            }
        } else {
            "attention"
        };
        self.write(format!(r#"match $r isa portal_recovery,has biz_id "{id}",has auth_status "{status}";update $r has auth_status "{terminal}";fetch {{"id":$r.biz_id}};"#)).await?;
        result?;
        Ok(1)
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for RecoveryService {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities::from_methods(vec![ServiceMethodKind::Create])
    }
    async fn create(&self, _: &TenantContext, data: Value, _: BusinessParams) -> Result<Value> {
        match parse::<Input>(data)? {
            Input::Request { email } => {
                self.request(&email).await?;
                Ok(
                    json!({"requested":true,"message":"If this email has an account, a recovery link will arrive shortly. Check spam, then wait ten minutes before trying again."}),
                )
            }
            Input::Reset {
                token,
                password,
                confirm_password,
            } => {
                self.reset(&token, password, &confirm_password).await?;
                Ok(json!({"reset":true}))
            }
        }
    }
}
