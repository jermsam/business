use anyhow::{ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};
use server::billing::engine::Engine;
use sha2::{Digest, Sha256};
use tower::ServiceExt;
const PASSWORD: &str = "sandbox-link-test-password";
const GATEWAY: &str = "sandbox-link-test-gateway-at-least-32-characters";
async fn call(
    r: &axum::Router,
    tenant: &str,
    token: &str,
    path: &str,
    data: Value,
) -> Result<(u16, Value)> {
    let mut req = Request::post(path)
        .header("content-type", "application/json")
        .header("x-tenant-id", tenant)
        .header("x-gateway-secret", GATEWAY);
    if !token.is_empty() {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = r
        .clone()
        .oneshot(req.body(Body::from(data.to_string()))?)
        .await?;
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), 1_000_000).await?;
    Ok((status, serde_json::from_slice(&body)?))
}
pub async fn check(e: &Engine) -> Result<()> {
    std::env::set_var("BUSINESS_GATEWAY_SECRET", GATEWAY);
    let hash = bcrypt::hash(PASSWORD, 4)?;
    let id = uuid::Uuid::new_v4();
    e.write(format!(r#"{} insert $u isa person,has identity_id "{id}",has email "owner-links@example.com",has name "TEST owner",has password_hash "{hash}",has credential_version "fixture",has status "active";(tenant:$t,member:$u) isa membership,has membership_role "owner",has start_at 2020-01-01T00:00:00;fetch {{"done":true}};"#,e.prefix())).await?;
    let (_, http) = server::app::build_with_state(
        e.state.clone(),
        "account-links-sandbox-signing-secret-long-enough".into(),
    )?;
    let r = server::app::router(http).merge(server::billing::http::router(e.clone())?);
    let (s, login) = call(
        &r,
        "account",
        "",
        "/authentication",
        json!({"strategy":"account","email":"owner-links@example.com","password":PASSWORD}),
    )
    .await?;
    ensure!(s == 200, "Owner login {s}");
    let owner = login["accessToken"].as_str().unwrap();
    let (s, invite) = call(
        &r,
        &e.merchant,
        owner,
        "/portal",
        json!({"action":"invite","name":"TEST invited company","email":"new-links@example.com"}),
    )
    .await?;
    ensure!(s == 200, "Invite failed {s}: {invite}");
    ensure!(
        invite["delivery"]["status"] == "unconfirmed",
        "Unapproved mailbox was sent email"
    );
    let customer = invite["customer_id"].as_str().unwrap();
    let buyer = invite["tenant_id"].as_str().unwrap();
    let (s, renew) = call(
        &r,
        &e.merchant,
        owner,
        "/portal",
        json!({"action":"renew_invite","customer_id":customer}),
    )
    .await?;
    ensure!(s == 200, "Renew failed {s}: {renew}");
    ensure!(
        call(
            &r,
            "account",
            "",
            "/onboarding",
            json!({"action":"accept","token":invite["token"],"password":PASSWORD})
        )
        .await?
        .0 == 400,
        "Old invitation survived renewal"
    );
    let accept = json!({"action":"accept","token":renew["token"],"password":PASSWORD});
    let (s, result) = call(&r, "account", "", "/onboarding", accept.clone()).await?;
    ensure!(s == 200, "Accept failed {s}: {result}");
    ensure!(
        call(&r, "account", "", "/onboarding", accept).await?.0 == 400,
        "Invitation replay accepted"
    );
    let (s, login) = call(
        &r,
        "account",
        "",
        "/authentication",
        json!({"strategy":"account","email":"new-links@example.com","password":PASSWORD}),
    )
    .await?;
    ensure!(s == 200, "New account login failed {s}");
    let buyer_token = login["accessToken"].as_str().unwrap();
    ensure!(
        call(
            &r,
            buyer,
            buyer_token,
            "/portal",
            json!({"action":"invite","name":"denied","email":"denied@example.com"})
        )
        .await?
        .0 >= 400,
        "Buyer invited merchant customer"
    );
    let known = call(
        &r,
        "account",
        "",
        "/recovery",
        json!({"action":"request","email":"new-links@example.com"}),
    )
    .await?;
    let unknown = call(
        &r,
        "account",
        "",
        "/recovery",
        json!({"action":"request","email":"unknown-links@example.com"}),
    )
    .await?;
    ensure!(
        known == unknown && known.0 == 200,
        "Recovery reveals account existence"
    );
    // Known token is seeded only in this disposable database, never an existing account.
    let token = "f".repeat(64);
    let key = format!("{:x}", Sha256::digest(token.as_bytes()));
    let expiry = chrono::Utc::now().timestamp() + 1800;
    let person=e.read("match $u isa person,has email 'new-links@example.com';fetch {'id':$u.identity_id,'version':$u.credential_version};".into()).await?;
    let target = person[0]["id"].as_str().unwrap();
    let version = person[0]["version"].as_str().unwrap();
    e.write(format!(r#"{} insert $l isa account_link,has token_key "{key}",has email "new-links@example.com",has link_kind "reset",has link_target "{target}",has link_version "{version}",has link_expires {expiry},has link_used false;fetch {{"done":true}};"#,e.prefix())).await?;
    let reset = json!({"action":"reset","token":token,"password":"new-sandbox-link-password","confirm_password":"new-sandbox-link-password"});
    let (a, b) = tokio::join!(
        call(&r, "account", "", "/recovery", reset.clone()),
        call(&r, "account", "", "/recovery", reset)
    );
    let (a, b) = (a?, b?);
    ensure!(
        (a.0 == 200) != (b.0 == 200),
        "Reset race did not have exactly one winner: {a:?} {b:?}"
    );
    ensure!(
        call(
            &r,
            buyer,
            buyer_token,
            "/portal",
            json!({"action":"payment_methods","customer_id":customer})
        )
        .await?
        .0 == 401,
        "Reset did not invalidate old session"
    );
    ensure!(
        call(
            &r,
            "account",
            "",
            "/authentication",
            json!({"strategy":"account","email":"new-links@example.com","password":PASSWORD})
        )
        .await?
        .0 == 401,
        "Old password still works"
    );
    ensure!(call(&r,"account","","/authentication",json!({"strategy":"account","email":"new-links@example.com","password":"new-sandbox-link-password"})).await?.0==200,"New password fails");
    // A second company invitation must verify the existing password, never replace it.
    let (_,existing)=call(&r,&e.merchant,owner,"/portal",json!({"action":"invite","name":"TEST existing identity","email":"owner-links@example.com"})).await?;
    ensure!(call(&r,"account","","/onboarding",json!({"action":"accept","token":existing["token"],"password":"wrong-but-long-password"})).await?.0==400,"Existing identity was taken over");
    ensure!(
        call(
            &r,
            "account",
            "",
            "/onboarding",
            json!({"action":"accept","token":existing["token"],"password":PASSWORD})
        )
        .await?
        .0 == 200,
        "Existing identity cannot join"
    );
    let (_, revoked) = call(
        &r,
        &e.merchant,
        owner,
        "/portal",
        json!({"action":"invite","name":"TEST revoked","email":"revoked-links@example.com"}),
    )
    .await?;
    ensure!(
        call(
            &r,
            &e.merchant,
            owner,
            "/portal",
            json!({"action":"revoke_invite","customer_id":revoked["customer_id"]})
        )
        .await?
        .0 == 200,
        "Revocation failed"
    );
    ensure!(
        call(
            &r,
            "account",
            "",
            "/onboarding",
            json!({"action":"accept","token":revoked["token"],"password":PASSWORD})
        )
        .await?
        .0 == 400,
        "Revoked link accepted"
    );
    let (_, expired) = call(
        &r,
        &e.merchant,
        owner,
        "/portal",
        json!({"action":"invite","name":"TEST expired","email":"expired-links@example.com"}),
    )
    .await?;
    let expired_key = format!(
        "{:x}",
        Sha256::digest(expired["token"].as_str().unwrap().as_bytes())
    );
    e.write(format!(r#"{} $l isa account_link,has token_key "{expired_key}";update $l has link_expires 1;fetch {{"done":true}};"#,e.prefix())).await?;
    ensure!(
        call(
            &r,
            "account",
            "",
            "/onboarding",
            json!({"action":"accept","token":expired["token"],"password":PASSWORD})
        )
        .await?
        .0 == 400,
        "Expired link accepted"
    );
    let denied = r
        .clone()
        .oneshot(
            Request::post("/recovery")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"action":"request","email":"owner-links@example.com"}).to_string(),
                ))?,
        )
        .await?;
    ensure!(
        denied.status() == 401,
        "Direct recovery bypassed the gateway"
    );
    println!("Verified invitation renewal, acceptance, replay protection, tenant permissions, generic recovery, reset race, password change and session revocation.");
    Ok(())
}
