//! Provider authentication service contract, exercised through registered hooks.
use super::*;

#[tokio::test]
#[ignore = "Read-only real Mercury sandbox using the existing external secret file"]
async fn existing_mercury_sandbox_service_chain() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    let original_token = std::env::var("MERCURY_SANDBOX_TOKEN")?;
    let credentials = vec![Connection {
        tenant_id: "sandbox-smoke".into(),
        identity_ids: HashSet::from(["sandbox-smoke".into()]),
        environment: Environment::Sandbox,
        token: original_token.clone(),
        write: false,
    }];
    let h = Harness::with_transport(
        credentials,
        Arc::new(MercuryHttp::new()?),
        Arc::new(Provider::default()),
    )?;
    let mut failures = Vec::new();
    let resolved = h
        .app
        .service("auth-mercury")?
        .custom(
            TenantContext::new("sandbox-smoke"),
            "resolve",
            Some(json!({"operation":"read"})),
            h.params("sandbox-smoke", "sandbox-smoke").await?,
        )
        .await?;
    anyhow::ensure!(
        resolved["accessToken"].as_str() == Some(original_token.as_str())
            && resolved["environment"] == "sandbox",
        "Sandbox credential resolution changed token or environment"
    );
    for name in ["accounts", "customers", "invoices", "transactions"] {
        let service = h.app.service(&format!("mercury/{name}"))?;
        let mut params = h.params("sandbox-smoke", "sandbox-smoke").await?;
        params.inner.query.insert("limit".into(), "1".into());
        match service
            .find(TenantContext::new("sandbox-smoke"), params)
            .await
        {
            Ok(rows) => {
                println!("PASS: registered mercury/{name} find through auth-mercury");
                if let Some(id) = rows.first().and_then(|row| row["id"].as_str()) {
                    match service
                        .get(
                            TenantContext::new("sandbox-smoke"),
                            id,
                            h.params("sandbox-smoke", "sandbox-smoke").await?,
                        )
                        .await
                    {
                        Ok(_) => {
                            println!("PASS: registered mercury/{name} get through auth-mercury")
                        }
                        Err(error) => failures.push(format!("mercury/{name} get: {error}")),
                    }
                }
            }
            Err(error) => failures.push(format!("mercury/{name} find: {error}")),
        }
    }
    if !failures.is_empty() {
        let response = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .get("https://api-sandbox.mercury.com/api/v1/accounts")
            .query(&[("limit", "1")])
            .bearer_auth(&original_token)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?.to_lowercase();
        let reason = if body.contains("ip")
            && (body.contains("allowlist")
                || body.contains("whitelist")
                || body.contains("allowed"))
        {
            "IP restriction"
        } else if body.contains("scope") || body.contains("permission") {
            "permission restriction"
        } else {
            "reason not classified"
        };
        println!("Direct baseline accounts read: HTTP {status}, {reason}; token/environment equality verified");
    }
    anyhow::ensure!(
        failures.is_empty(),
        "Sandbox restrictions: {}",
        failures.join("; ")
    );
    Ok(())
}

#[tokio::test]
async fn auth_mercury_resolves_only_for_internal_verified_tenant_calls() -> Result<()> {
    let h = Harness::new()?;
    let service = h.app.service("auth-mercury")?;
    let ctx = TenantContext::new("tenant-a");
    let p = h.params("alice", "tenant-a").await?;
    let result = service
        .custom(
            ctx.clone(),
            "resolve",
            Some(json!({"operation":"read"})),
            p.clone(),
        )
        .await?;
    assert_eq!(result["tenant_id"], "tenant-a");
    assert_eq!(result["identity_id"], "alice");
    assert!(result["accessToken"]
        .as_str()
        .is_some_and(|v| v == "private-a"));
    assert_eq!(result["environment"], "sandbox");
    assert_eq!(
        h.count(),
        0,
        "Credential resolution must not call provider endpoints"
    );
    for provider in ["rest", "socketio"] {
        let mut external = p.clone();
        external.provider = Some(provider.into());
        assert!(service
            .custom(
                ctx.clone(),
                "resolve",
                Some(json!({"operation":"read"})),
                external
            )
            .await
            .is_err());
        let mut external = p.clone();
        external.inner.provider = provider.into();
        assert!(service
            .custom(
                ctx.clone(),
                "resolve",
                Some(json!({"operation":"read"})),
                external
            )
            .await
            .is_err());
    }
    let forged = BusinessParams {
        authenticated: true,
        auth_result: Some(json!({"user":{"id":"alice","tenant_id":"tenant-a"}})),
        ..Default::default()
    };
    assert!(service
        .custom(
            ctx.clone(),
            "resolve",
            Some(json!({"operation":"read"})),
            forged
        )
        .await
        .is_err());
    assert!(service
        .custom(
            TenantContext::new("tenant-b"),
            "resolve",
            Some(json!({"operation":"read"})),
            p.clone()
        )
        .await
        .is_err());
    assert!(service
        .custom(
            ctx.clone(),
            "resolve",
            Some(json!({"operation":"read"})),
            h.params("mallory", "tenant-a").await?
        )
        .await
        .is_err());
    for body in [
        json!({}),
        json!({"operation":"admin"}),
        json!({"operation":"read","tenant_id":"tenant-b"}),
        json!({"operation":"read","token":"override"}),
    ] {
        assert!(service
            .custom(ctx.clone(), "resolve", Some(body), p.clone())
            .await
            .is_err());
    }
    assert!(service
        .custom(
            TenantContext::new("tenant-b"),
            "resolve",
            Some(json!({"operation":"write"})),
            h.params("bob", "tenant-b").await?
        )
        .await
        .is_err());
    assert_eq!(h.count(), 0);
    Ok(())
}

#[tokio::test]
async fn resource_calls_run_auth_mercury_hooks_and_do_not_expose_credentials() -> Result<()> {
    use std::sync::atomic::Ordering;
    let h = Harness::new()?;
    let result = h
        .app
        .service("mercury/customers")?
        .get(
            TenantContext::new("tenant-a"),
            ID,
            h.params("alice", "tenant-a").await?,
        )
        .await?;
    assert_eq!(
        h.auth_calls.load(Ordering::SeqCst),
        1,
        "Resource bypassed registered auth-mercury hooks"
    );
    let text = result.to_string();
    assert!(!text.contains("private-a") && !text.contains("accessToken"));
    Ok(())
}

#[tokio::test]
async fn concurrent_tenants_never_share_provider_credentials() -> Result<()> {
    use sha2::Digest;
    let h = Harness::new()?;
    let mut calls = Vec::new();
    for index in 0..40 {
        let (actor, tenant) = if index % 2 == 0 {
            ("alice", "tenant-a")
        } else {
            ("bob", "tenant-b")
        };
        let service = h.app.service("mercury/customers")?;
        let params = h.params(actor, tenant).await?;
        calls.push(tokio::spawn(async move {
            service.get(TenantContext::new(tenant), ID, params).await
        }));
    }
    for call in calls {
        call.await??;
    }
    let rows = h.provider.calls.lock().unwrap();
    assert_eq!(rows.len(), 40);
    for row in rows.iter() {
        let expected = match row.tenant.as_str() {
            "tenant-a" => "private-a",
            "tenant-b" => "private-b",
            _ => panic!("unexpected tenant"),
        };
        assert_eq!(
            row.credential_digest,
            format!("{:x}", sha2::Sha256::digest(expected.as_bytes()))
        );
    }
    assert_eq!(h.auth_calls.load(std::sync::atomic::Ordering::SeqCst), 40);
    Ok(())
}

#[tokio::test]
async fn auth_mercury_has_no_public_http_route() -> Result<()> {
    use dog_transport::IntoDogService;
    use tower::ServiceExt;
    let h = Harness::new()?;
    let p = h.params("alice", "tenant-a").await?;
    let http = h
        .app
        .clone()
        .into_service(dog_transport::HttpOptions::default().tenant_header("x-tenant-id"));
    let router = super::super::router(http);
    for path in [
        "/auth-mercury",
        "/mercury/auth-mercury",
        "/mercury/auth-mercury/resolve",
    ] {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", &p.headers["authorization"])
                    .header("x-tenant-id", "tenant-a")
                    .header("x-service-method", "resolve")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({"operation":"read"}).to_string(),
                    ))?,
            )
            .await?;
        assert_eq!(response.status(), 404);
    }
    assert_eq!(h.count(), 0);
    Ok(())
}
