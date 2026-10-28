use anyhow::{ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};
use server::billing::engine::Engine;
use tower::ServiceExt;
async fn call(
    router: &axum::Router,
    tenant: &str,
    token: &str,
    path: &str,
    data: Option<Value>,
) -> Result<(u16, Value)> {
    let mut request = Request::builder()
        .uri(path)
        .method(if data.is_some() { "POST" } else { "GET" })
        .header("x-tenant-id", tenant)
        .header("content-type", "application/json");
    if !token.is_empty() {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(data.map(|d| d.to_string()).unwrap_or_default()))?)
        .await?;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1_000_000).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}
pub async fn check(engine: &Engine) -> Result<()> {
    std::env::set_var("BILLING_ENABLED", "true");
    let customers=engine.read(format!("{} (merchant:$t,buyer:$buyer,customer:$c) isa bill_account;fetch {{'id':$c.bill_key,'buyer':$buyer.tenant_id}};",engine.prefix())).await?;
    let user = uuid::Uuid::new_v4().to_string();
    let buyer = customers[0]["buyer"].as_str().unwrap();
    let customer = customers[0]["id"].as_str().unwrap();
    let other = customers[1]["id"].as_str().unwrap();
    let hash = bcrypt::hash("portal-sandbox-test-password", 4)?;
    engine.write(format!(r#"{} $b isa tenant,has tenant_id "{buyer}";insert $u isa person,has identity_id "{user}",has name "TEST portal buyer",has email "portal@example.com",has password_hash "{hash}",has credential_version "fixture",has status "active";(tenant:$b,member:$u) isa membership,has membership_role "owner",has start_at 2020-01-01T00:00:00;fetch {{"id":$u.identity_id}};"#,engine.prefix())).await?;
    let (_, http) = server::app::build_with_state(
        engine.state.clone(),
        "portal-sandbox-test-signing-secret-long-enough".into(),
    )?;
    let router = server::app::router(http);
    ensure!(
        call(&router, buyer, "", "/portal", None).await?.0 == 401,
        "Anonymous portal allowed"
    );
    let (status,login)=call(&router,"account","","/authentication",Some(json!({"strategy":"account","email":"portal@example.com","password":"portal-sandbox-test-password"}))).await?;
    ensure!(status == 200, "Portal login failed {status}");
    let token = login["accessToken"].as_str().unwrap();
    let (status, profile) = call(&router, buyer, token, "/portal", None).await?;
    ensure!(
        status == 200
            && profile[0]["email"] == "portal@example.com"
            && profile[0]["companies"]
                .as_array()
                .is_some_and(|v| v.len() == 1),
        "Profile failed: {status} {profile}"
    );
    let (status, methods) = call(
        &router,
        buyer,
        token,
        "/portal",
        Some(json!({"action":"payment_methods","customer_id":customer})),
    )
    .await?;
    ensure!(
        status == 200 && methods["methods"].as_array().is_some_and(|v| !v.is_empty()),
        "Payment listing failed: {status} {methods}"
    );
    ensure!(
        call(
            &router,
            buyer,
            token,
            "/portal",
            Some(json!({"action":"payment_methods","customer_id":other}))
        )
        .await?
        .0 == 403,
        "Cross-customer payment access allowed"
    );
    ensure!(
        call(&router, &engine.merchant, token, "/portal", None)
            .await?
            .0
            >= 400,
        "Cross-tenant profile allowed"
    );
    let invoices=engine.read(format!("{} (customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;fetch {{'id':$i.bill_key}};",engine.customer_match(customer)?)).await?;
    let (status, history) = call(
        &router,
        buyer,
        token,
        "/portal",
        Some(json!({"action":"history","invoice_id":invoices[0]["id"]})),
    )
    .await?;
    ensure!(
        status == 200 && history["supported"] == true,
        "Invoice history failed: {status} {history}"
    );
    for (asset, variable, extra) in [
        ("billing-customers", "c", ""),
        ("billing-plans", "p", ",$c"),
        ("billing-invoices", "i", ",$c"),
    ] {
        let source = std::fs::read_to_string(format!(
            "{}/public/queries/data/{asset}.tql",
            env!("CARGO_MANIFEST_DIR")
        ))?;
        let query=source.replace("fetch {",&format!("${variable} has bill_key $cursor;select ${variable}{extra},$cursor;distinct;sort $cursor;limit 100;fetch {{"));
        let response = router
            .clone()
            .oneshot(
                Request::post("/data")
                    .header("x-tenant-id", buyer)
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-service-method", "read")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"query":query}).to_string()))?,
            )
            .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1_000_000).await?;
        let data: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            status == 200
                && data["ok"]["answers"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()),
            "Portal {asset} query failed: {status} {data}"
        );
    }
    let method = methods["methods"][0]["id"].as_str().unwrap();
    let current = engine.customer(customer).await?;
    let (status,result)=call(&router,buyer,token,"/portal",Some(json!({"action":"set_default","customer_id":customer,"method_id":method,"expected_revision":current["revision"]}))).await?;
    ensure!(status == 200, "Default update failed: {status} {result}");
    let current = engine.customer(customer).await?;
    ensure!(
        current["choice"] == "manual" && current["consent"] == "",
        "Changing default retained consent"
    );
    let (status,result)=call(&router,buyer,token,"/portal",Some(json!({"action":"remove_method","customer_id":customer,"method_id":method,"expected_revision":current["revision"]}))).await?;
    ensure!(status == 200, "Remove method failed: {status} {result}");
    Ok(())
}
