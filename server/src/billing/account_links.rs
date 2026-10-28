//! One-use invitations and recovery on canonical identities. Tokens are hashed at rest.
use super::{billing_schema::quoted as q, engine::Engine};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
fn token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn datetime() -> String {
    chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}
fn field<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or_else(|| anyhow::anyhow!("Missing {k}"))
}
fn email(v: &Value) -> Result<String> {
    let s = field(v, "email")?.trim().to_ascii_lowercase();
    ensure!(
        s.len() <= 254
            && s.split_once('@')
                .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.contains('@'))
            && !s.chars().any(|c| c.is_control() || c.is_whitespace()),
        "Enter a valid email address"
    );
    Ok(s)
}
async fn hash(password: String) -> Result<String> {
    ensure!(
        (12..=72).contains(&password.len()) && !password.trim().is_empty(),
        "Use 12–72 characters for your password"
    );
    Ok(tokio::task::spawn_blocking(move || bcrypt::hash(password, bcrypt::DEFAULT_COST)).await??)
}
impl Engine {
    async fn send_link(&self, email: &str, token: &str, kind: &str) -> Result<()> {
        ensure!(!self.providers.live, "Sandbox only");
        let allowed = std::env::var("INVITATION_TEST_EMAILS")
            .or_else(|_| std::env::var("BILLING_TEST_EMAIL"))
            .unwrap_or_default();
        ensure!(
            allowed
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case(email)),
            "Sandbox recipient is not approved"
        );
        ensure!(
            std::env::var("BILLING_MAIL_ENABLED").as_deref() == Ok("true"),
            "Email disabled"
        );
        let (subject, body) = if kind == "invite" {
            ("[TEST] Your JITPOMI account is ready",format!("You’re invited to JITPOMI. Set your password to see your company’s invoices and payments.\n\nhttps://jitpomi.com/account/#invite={token}\n\nThis private link works once and expires in 48 hours. Already have an account? Use your existing password.\n\nThis is a sandbox test. No real payments. Need help? Email dev@jitpomi.com."))
        } else {
            ("[TEST] Reset your JITPOMI password",format!("Choose a new password here:\n\nhttps://jitpomi.com/account/#reset={token}\n\nThis private link works once and expires in 30 minutes. If you didn’t ask for this, ignore this email. Your password has not changed.\n\nThis is a sandbox test. Need help? Email dev@jitpomi.com."))
        };
        self.providers
            .email(
                &std::env::var("RESEND_API_KEY")?,
                &std::env::var("BILLING_EMAIL_FROM")?,
                email,
                subject,
                &body,
                &format!("account-{kind}-{}", digest(token)),
            )
            .await?;
        Ok(())
    }
    pub async fn invitation(&self, scope: &str, data: &Value) -> Result<Value> {
        let action = field(data, "action")?;
        let secret = token();
        let key = digest(&secret);
        let expires = now() + 172800;
        let (customer, buyer, address) = if action == "invite" {
            let name = field(data, "name")?.trim();
            ensure!(
                !name.is_empty() && name.len() <= 120 && !name.chars().any(char::is_control),
                "Enter a company name"
            );
            let address = email(data)?;
            let customer = uuid::Uuid::new_v4().to_string();
            let buyer = uuid::Uuid::new_v4().to_string();
            self.write(format!(r#"{} {scope} not {{(merchant:$t,customer:$old) isa bill_account;$old has bill_email {};}};select $t;distinct;insert $buyer isa tenant,has tenant_id "{buyer}",has name {},has status "active",has revision "{key}";$c isa bill_customer,has bill_key "{customer}",has name {},has bill_email {},has bill_policy "customer",has bill_choice "manual",has bill_provider "stripe",has bill_stripe_id "",has bill_mercury_id "",has bill_payment_method "",has bill_consent "",has bill_revision "{key}";(merchant:$t,buyer:$buyer,customer:$c) isa bill_account;$l isa account_link,has token_key "{key}",has email {},has link_kind "invite",has link_target "{buyer}",has link_version "{key}",has link_expires {expires},has link_used false;fetch {{"id":$c.bill_key}};"#,self.prefix(),q(&address),q(name),q(name),q(&address),q(&address))).await?;
            (customer, buyer, address)
        } else {
            ensure!(
                matches!(action, "renew_invite" | "revoke_invite"),
                "Action unavailable"
            );
            let customer = super::billing_schema::id(field(data, "customer_id")?)?;
            let found=self.read(format!(r#"{} {scope} (merchant:$t,buyer:$b,customer:$c) isa bill_account;$c has bill_key "{customer}";fetch {{"buyer":$b.tenant_id,"email":$c.bill_email}};"#,self.prefix())).await?;
            ensure!(found.len() == 1, "Customer unavailable");
            let buyer = field(&found[0], "buyer")?.to_owned();
            let address = field(&found[0], "email")?.to_owned();
            if action == "revoke_invite" {
                self.write(format!(r#"{} {scope} $b isa tenant,has tenant_id "{buyer}";select $t,$b;distinct;update $b has revision "{key}";fetch {{"revoked":true}};"#,self.prefix())).await?;
                return Ok(json!({"revoked":true}));
            }
            self.write(format!(r#"{} {scope} $b isa tenant,has tenant_id "{buyer}",has status "active";not {{(tenant:$b,member:$u) isa membership;$u isa person,has email {};}};select $t,$b;distinct;update $b has revision "{key}";insert $l isa account_link,has token_key "{key}",has email {},has link_kind "invite",has link_target "{buyer}",has link_version "{key}",has link_expires {expires},has link_used false;fetch {{"done":true}};"#,self.prefix(),q(&address),q(&address))).await?;
            (customer, buyer, address)
        };
        let delivery = if self.send_link(&address, &secret, "invite").await.is_ok() {
            json!({"status":"accepted"})
        } else {
            json!({"status":"unconfirmed","message":"The invitation was created, but we couldn’t confirm its email. Check the inbox before resending."})
        };
        Ok(
            json!({"customer_id":customer,"tenant_id":buyer,"token":secret,"expires_at":expires,"delivery":delivery}),
        )
    }
    pub async fn public_account_action(&self, kind: &str, data: &Value) -> Result<Value> {
        ensure!(!self.providers.live, "Sandbox only");
        if kind == "reset" && data["action"] == "request" {
            let address = email(data)?;
            let answer = json!({"requested":true,"message":"If you have an account, we’ll email you a link to reset your password. Check your inbox and spam folder."});
            let at = datetime();
            let people=self.read(format!("match $u isa person,has email {},has status 'active';$b isa tenant;let $ok=active_member($u,$b,{at});$ok==true;select $u;distinct;fetch {{'id':$u.identity_id,'version':$u.credential_version}};",q(&address))).await?;
            if people.len() != 1 {
                return Ok(answer);
            }
            let cutoff = now() + 1500;
            let recent=self.read(format!("match $l isa account_link,has email {},has link_kind 'reset',has link_expires $expires;$expires>{cutoff};fetch {{'id':$l.token_key}};",q(&address))).await?;
            if !recent.is_empty() {
                return Ok(answer);
            }
            let secret = token();
            let key = digest(&secret);
            let expires = now() + 1800;
            let result=self.write(format!(r#"{} $u isa person,has identity_id {},has credential_version {};not {{$recent isa account_link,has email {},has link_kind "reset",has link_expires $expiry;$expiry>{cutoff};}};select $t,$u;distinct;insert $l isa account_link,has token_key "{key}",has email {},has link_kind "reset",has link_target {},has link_version {},has link_expires {expires},has link_used false;fetch {{"done":true}};"#,self.prefix(),q(field(&people[0],"id")?),q(field(&people[0],"version")?),q(&address),q(&address),q(field(&people[0],"id")?),q(field(&people[0],"version")?))).await;
            if result.is_ok() {
                let _ = self.send_link(&address, &secret, "reset").await;
            }
            return Ok(answer);
        }
        let secret = field(data, "token")?;
        ensure!(
            secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()),
            "This link is invalid or expired"
        );
        let key = digest(secret);
        let timestamp = now();
        let link_scope = format!(
            r#"$l isa account_link,has token_key "{key}",has link_kind "{kind}",has link_used false,has link_expires $expiry;$expiry>{timestamp};"#
        );
        let links=self.read(format!("{} {link_scope} fetch {{'email':$l.email,'target':$l.link_target,'version':$l.link_version}};",self.prefix())).await?;
        ensure!(links.len() == 1, "This link is invalid or expired");
        let l = &links[0];
        let address = field(l, "email")?;
        let target = field(l, "target")?;
        let password = field(data, "password")?.to_owned();
        if kind == "reset" {
            ensure!(data["confirm_password"] == password, "Passwords must match");
            let hashed = hash(password).await?;
            let version = uuid::Uuid::new_v4().to_string();
            self.write(format!(r#"{} {link_scope} $u isa person,has identity_id {},has email {},has status "active",has credential_version {};select $t,$l,$u;distinct;update $l has link_used true;$u has password_hash {},has credential_version "{version}";fetch {{"reset":true}};"#,self.prefix(),q(target),q(address),q(field(l,"version")?),q(&hashed))).await?;
            return Ok(json!({"reset":true}));
        }
        ensure!(kind == "invite", "Action unavailable");
        let at = datetime();
        let existing=self.read(format!("match $u isa person,has email {};fetch {{'id':$u.identity_id,'hash':$u.password_hash,'version':$u.credential_version,'status':$u.status}};",q(address))).await?;
        let person = if let Some(u) = existing.first() {
            ensure!(u["status"] == "active", "Account unavailable");
            let hashed = field(u, "hash")?.to_owned();
            ensure!(
                tokio::task::spawn_blocking(move || bcrypt::verify(password, &hashed)).await??,
                "Use your existing account password"
            );
            format!(
                "$u isa person,has identity_id {},has credential_version {},has status 'active';",
                q(field(u, "id")?),
                q(field(u, "version")?)
            )
        } else {
            String::new()
        };
        let create = if existing.is_empty() {
            let hashed = hash(field(data, "password")?.to_owned()).await?;
            format!("insert $u isa person,has identity_id {},has email {},has name {},has password_hash {},has credential_version {},has status 'active';",q(&uuid::Uuid::new_v4().to_string()),q(address),q(address),q(&hashed),q(&uuid::Uuid::new_v4().to_string()))
        } else {
            String::new()
        };
        self.write(format!(r#"{} {link_scope} $b isa tenant,has tenant_id {},has status "active",has revision {};{person} not {{(tenant:$b,member:$member) isa membership;$member isa person,has email {};}};{create} update $l has link_used true;insert (tenant:$b,member:$u) isa membership,has membership_role "owner",has start_at {at};fetch {{"email":$u.email}};"#,self.prefix(),q(target),q(field(l,"version")?),q(address))).await
    }
}
