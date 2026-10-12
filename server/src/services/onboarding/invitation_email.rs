//! Email follows a committed invitation. Failure must not hide the created customer.
use crate::services::billing::providers::Providers;
use anyhow::{ensure, Result};
use serde_json::{json, Value};

fn invitation_body(base: &str, token: &str) -> Result<String> {
    let url = reqwest::Url::parse(base)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("jitpomi.com")
            && url.path() == "/account/"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port().is_none(),
        "Trusted invitation URL required"
    );
    ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid invitation token"
    );
    Ok(format!("You’re invited to your JITPOMI account.\n\nChoose your password to view your company’s invoices and payments:\n{base}#invite={token}\n\nThis private link works once and expires in 48 hours. If you already have a JITPOMI account, use your existing password.\n\nNeed a hand? Reply to dev@jitpomi.com. If you weren’t expecting this invitation, you can ignore it."))
}

pub(super) async fn deliver(email: &str, token: &str) -> Value {
    let send = async {
        ensure!(
            std::env::var("BILLING_MAIL_ENABLED").as_deref() == Ok("true"),
            "Email disabled"
        );
        let providers = Providers::new()?;
        if !providers.live {
            let allowed = std::env::var("INVITATION_TEST_EMAILS")
                .or_else(|_| std::env::var("BILLING_TEST_EMAIL"))
                .unwrap_or_default();
            ensure!(
                allowed
                    .split(',')
                    .any(|item| item.trim().eq_ignore_ascii_case(email)),
                "Test recipient not allowed"
            );
        }
        let body = invitation_body(&std::env::var("BILLING_PUBLIC_URL")?, token)?;
        let subject = if providers.live {
            "You’re invited to JITPOMI"
        } else {
            "[TEST] You’re invited to JITPOMI"
        };
        // The hash identifies this particular link without exposing its bearer secret.
        let key = format!("invitation-{}", super::onboarding_shared::digest(token));
        providers
            .email(
                &std::env::var("RESEND_API_KEY")?,
                &std::env::var("BILLING_EMAIL_FROM")?,
                email,
                subject,
                &body,
                &key,
            )
            .await
    }
    .await;
    match send {
        Ok(id) => json!({"status":"accepted","message_id":id}),
        // A timeout can occur after provider acceptance. Do not automatically send again.
        Err(_) => {
            json!({"status":"unconfirmed","message":"The invitation was created, but we couldn’t confirm the email was sent. Check the inbox before resending. You can also copy the invitation link."})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitation_links_are_fixed_to_our_account_page() {
        let token = "a".repeat(64);
        assert!(invitation_body("https://jitpomi.com/account/", &token)
            .unwrap()
            .contains(&format!("#invite={token}")));
        for base in [
            "http://jitpomi.com/account/",
            "https://evil.test/account/",
            "https://jitpomi.com/account/?next=evil",
            "https://jitpomi.com/account/#x",
            "https://user@jitpomi.com/account/",
        ] {
            assert!(invitation_body(base, &token).is_err());
        }
        assert!(invitation_body("https://jitpomi.com/account/", "bad\ntoken").is_err());
    }
}
