use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::{DogApp, DogAppBuilder};
use dog_transport::{http::DogHttpService, HttpOptions, IntoDogService};
use serde_json::Value;
pub async fn build() -> Result<(
    DogApp<Value, BusinessParams>,
    DogHttpService<Value, BusinessParams>,
)> {
    if let Err(error) = match std::env::var("BUSINESS_ENV_FILE") {
        Ok(path) => dotenvy::from_path(path),
        Err(_) => dotenvy::dotenv().map(|_| ()),
    } {
        if !error.not_found() {
            return Err(error.into());
        }
    }
    let mut builder = DogAppBuilder::new();
    crate::config::config(&mut builder)?;
    crate::typedb::TypeDBState::initialize(&mut builder).await?;
    let state = builder
        .get("typedb")
        .ok_or_else(|| anyhow::anyhow!("TypeDB state missing"))?;
    let auth = crate::auth::strategies(&mut builder)?;
    crate::hooks::global_hook(&mut builder)?;
    crate::channels::channels(&mut builder)?;
    crate::services::configure(&mut builder, auth.clone(), state)?;
    let app = builder.build();
    auth.setup(app.clone());
    let http = app.clone().into_service(
        HttpOptions::default()
            .tenant_header("x-tenant-id")
            .body_limit(64 * 1024)
            .route("/authentication", "authentication"),
    );
    Ok((app, http))
}

dog_transport::declare_adapter!(axum, endpoint, crate::BusinessParams);
/// The only public HTTP routes. Do not add a fallback exposing arbitrary services.
pub fn router(http: DogHttpService<Value, BusinessParams>) -> axum::Router {
    axum::Router::new()
        .route_service("/authentication", endpoint(http.clone()))
        .route_service("/onboarding", endpoint(http.clone()))
        .route_service("/recovery", endpoint(http.clone()))
        .route_service("/billing-actions", endpoint(http.clone()))
        .route_service("/billing-customers", endpoint(http.clone()))
        .route_service("/billing-customers/{id}", endpoint(http.clone()))
        .route_service("/billing-plans", endpoint(http.clone()))
        .route_service("/billing-plans/{id}", endpoint(http.clone()))
        .route_service("/billing-invoices", endpoint(http.clone()))
        .route_service("/billing-invoices/{id}", endpoint(http.clone()))
        .route_service("/records", endpoint(http.clone()))
        .route_service("/records/{id}", endpoint(http.clone()))
        .route_service("/workspace-records", endpoint(http.clone()))
        .route_service("/workspace-records/{id}", endpoint(http.clone()))
        .route_service("/apps", endpoint(http.clone()))
        .route_service("/apps/{id}", endpoint(http.clone()))
        .route_service("/memberships", endpoint(http.clone()))
        .route_service("/memberships/{id}", endpoint(http.clone()))
        .route_service("/access-grants/{id}", endpoint(http.clone()))
        .route_service("/entitlements/{id}", endpoint(http.clone()))
        .route_service("/team-memberships/{id}", endpoint(http))
        .route_layer(axum::middleware::from_fn(crate::admission::guard))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/billing",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("../public/billing.html"))
            }),
        )
        .route(
            "/billing.js",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/javascript")],
                    include_str!("../public/billing.js"),
                )
            }),
        )
        .route(
            "/billing.css",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/css")],
                    include_str!("../public/billing.css"),
                )
            }),
        )
}

/// Billing ingress is opt-in so existing deployments remain unchanged until configured.
pub fn billing_router(app: &DogApp<Value, BusinessParams>) -> Result<axum::Router> {
    if std::env::var("BILLING_ENABLED").as_deref() != Ok("true") {
        return Ok(axum::Router::new());
    }
    let state = app
        .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
        .ok_or_else(|| anyhow::anyhow!("Missing database"))?;
    crate::services::billing::http::router(crate::services::billing::engine::Engine::configured(
        state,
    )?)
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use dog_auth::core::TokenStore;
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;
    async fn call(
        service: DogHttpService<Value, BusinessParams>,
        tenant: &str,
        method: &str,
        path: &str,
        data: Value,
        token: Option<&str>,
    ) -> (u16, Value) {
        if let Ok(base) = std::env::var("BUSINESS_TEST_BASE_URL") {
            return remote_call(&base, tenant, method, path, data, token).await;
        }
        let mut req = http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("x-tenant-id", tenant);
        if path == "/subjects" {
            req = req.header("x-service-method", "read");
        }
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let result = router(service)
            .oneshot(
                req.body(axum::body::Body::from(serde_json::to_vec(&data).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = result.status().as_u16();
        let body = result.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }
    include!("remote_test_client.rs");
    include!("billing_tests.rs");
    include!("password_recovery_tests.rs");
    #[cfg(unix)]
    include!("public_restart_tests.rs");
    #[tokio::test]
    #[ignore = "Requires explicitly configured business_dev on real TypeDB Cloud"]
    async fn hosted_auth_and_revocation() {
        let (app, http) = build().await.unwrap();
        let state = app
            .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
            .unwrap();
        assert_eq!(state.database, "business_dev", "Never seed production");
        let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let email = format!("test-{tag}@example.com");
        let tenant = format!("test-{tag}.example.com");
        let password = "test-only-correct-password";
        let hash = bcrypt::hash(password, 4).unwrap();
        adapter.write(json!({"query":format!(r#"insert $u isa user, has email "{email}", has password "{hash}", has first_name "Test", has last_name "Only"; $t isa company, has company_domain "{tenant}"; (tenant: $t, member: $u) isa tenant_membership;"#)})).await.unwrap();
        let (status, _) = call(
            http.clone(),
            &tenant,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":email,"password":"wrong"}),
            None,
        )
        .await;
        assert_eq!(status, 401);
        let (status, _) = call(
            http.clone(),
            "other.example.com",
            "POST",
            "/authentication",
            json!({"strategy":"local","email":email,"password":password}),
            None,
        )
        .await;
        assert_eq!(status, 401, "Membership must be checked");
        prepare_fixture(&state).await;
        let (status, login) = call(
            http.clone(),
            &tenant,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":email,"password":password}),
            None,
        )
        .await;
        assert!(
            (200..300).contains(&status),
            "Login failed with status {status}"
        );
        assert!(login["user"].get("password").is_none());
        let token = login["accessToken"].as_str().unwrap();
        let (status, _) = call(
            http.clone(),
            "other.example.com",
            "POST",
            "/authentication",
            json!({"strategy":"jwt","accessToken":token}),
            None,
        )
        .await;
        assert_eq!(status, 401, "JWT must not authorize a different tenant");
        let (status, verified) = call(
            http.clone(),
            &tenant,
            "POST",
            "/authentication",
            json!({"strategy":"jwt","accessToken":token}),
            None,
        )
        .await;
        assert!((200..300).contains(&status));
        assert!(verified["user"].get("password").is_none());

        let (status, _) = call(
            http.clone(),
            &tenant,
            "DELETE",
            "/authentication",
            Value::Null,
            Some(token),
        )
        .await;
        assert!((200..300).contains(&status));
        let (status, _) = call(
            http.clone(),
            &tenant,
            "POST",
            "/authentication",
            json!({"strategy":"jwt","accessToken":token}),
            None,
        )
        .await;
        assert_eq!(status, 401, "Revoked token must fail");
        for method in ["GET", "POST"] {
            let (status, _) = call(
                http.clone(),
                &tenant,
                method,
                "/subjects",
                json!({"query":"match $u isa user; fetch {\"u\":$u};"}),
                Some(token),
            )
            .await;
            assert!(
                (400..500).contains(&status),
                "Raw queries must not be exposed"
            );
        }
        let a = crate::auth::token_store::TypeDbTokenStore::new(state.clone());
        let b = crate::auth::token_store::TypeDbTokenStore::new(state);
        let until = chrono::Utc::now().timestamp() + 60;
        let (x, y) = tokio::join!(
            a.consume_refresh("test", &tag, until),
            b.consume_refresh("test", &tag, until)
        );
        assert_ne!(x.unwrap(), y.unwrap(), "Only one refresh consumer may win");
        assert!(b.is_revoked("test", &tag).await.unwrap());
        // Keep uniquely named synthetic fixtures for inspection; no production data is touched.
    }
    #[tokio::test]
    #[ignore = "Requires business_dev on real TypeDB Cloud with the records schema"]
    async fn hosted_business_data_isolation() {
        let (app, http) = build().await.unwrap();
        let state = app
            .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
            .unwrap();
        assert_eq!(state.database, "business_dev", "Never seed production");
        let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let a = format!("a-{tag}.example.com");
        let b = format!("b-{tag}.example.com");
        let alice = format!("alice-{tag}@example.com");
        let bob = format!("bob-{tag}@example.com");
        let password = "test-only-correct-password";
        let hash = bcrypt::hash(password, 4).unwrap();
        adapter
            .write(json!({"query":format!(r#"insert
            $a isa company, has company_domain "{a}"; $b isa company, has company_domain "{b}";
            $alice isa user, has email "{alice}", has password "{hash}";
            $bob isa user, has email "{bob}", has password "{hash}";
            (tenant:$a,member:$alice) isa tenant_membership;
            (tenant:$b,member:$alice) isa tenant_membership;
            (tenant:$b,member:$bob) isa tenant_membership;"#)}))
            .await
            .unwrap();
        prepare_fixture(&state).await;
        let (s, login) = call(
            http.clone(),
            &a,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":alice,"password":password}),
            None,
        )
        .await;
        assert_eq!(s, 200);
        let token = login["accessToken"].as_str().unwrap();
        let (s, login_b) = call(
            http.clone(),
            &b,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":bob,"password":password}),
            None,
        )
        .await;
        assert_eq!(s, 200);
        let bob_token = login_b["accessToken"].as_str().unwrap();
        let (s, ra) = call(
            http.clone(),
            &a,
            "POST",
            "/records",
            json!({"name":"A private"}),
            Some(token),
        )
        .await;
        assert_eq!(s, 200, "Create A: {ra}");
        let (s, rb) = call(
            http.clone(),
            &b,
            "POST",
            "/records",
            json!({"name":"B private"}),
            Some(token),
        )
        .await;
        assert_eq!(s, 200, "Create B: {rb}");
        // Direct service calls cannot forge an already-authenticated principal or
        // bypass checks by omitting the external provider flag.
        let forged = BusinessParams {
            authenticated: true,
            auth_result: Some(json!({"user":{"email":alice}})),
            ..Default::default()
        };
        let records = app.service("records").unwrap();
        assert!(records
            .find(dog_core::tenant::TenantContext::new(&a), forged.clone())
            .await
            .is_err());
        assert!(records
            .create(
                dog_core::tenant::TenantContext::new(&a),
                json!({"name":"Forged"}),
                forged
            )
            .await
            .is_err());
        assert!(
            app.service("subjects").is_err(),
            "Raw query service must not be registered"
        );
        let aid = ra["id"].as_str().unwrap();
        let bid = rb["id"].as_str().unwrap();
        for (tenant, own, foreign) in [(&a, aid, bid), (&b, bid, aid)] {
            let (s, list) = call(
                http.clone(),
                tenant,
                "GET",
                "/records",
                Value::Null,
                Some(token),
            )
            .await;
            assert_eq!(s, 200, "List: {list}");
            let rows = list.as_array().expect("Record list");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["id"], own);
            for verb in ["GET", "PUT", "PATCH", "DELETE"] {
                let (s, v) = call(
                    http.clone(),
                    tenant,
                    verb,
                    &format!("/records/{foreign}"),
                    json!({"name":"Intrusion"}),
                    Some(token),
                )
                .await;
                assert_eq!(s, 404, "Cross-tenant {verb}: {v}");
            }
            let (s, v) = call(
                http.clone(),
                tenant,
                "GET",
                &format!("/records/{own}"),
                Value::Null,
                Some(token),
            )
            .await;
            assert_eq!(s, 200);
            assert_eq!(v["id"], own);
            assert_ne!(v["name"], "Intrusion");
        }
        // Same-tenant membership is not blanket permission to someone else's records.
        for verb in ["GET", "PUT", "PATCH", "DELETE"] {
            assert_eq!(
                call(
                    http.clone(),
                    &b,
                    verb,
                    &format!("/records/{bid}"),
                    json!({"name":"Intrusion"}),
                    Some(bob_token)
                )
                .await
                .0,
                404
            );
        }
        assert_eq!(
            call(
                http.clone(),
                &a,
                "GET",
                "/records",
                Value::Null,
                Some(bob_token)
            )
            .await
            .0,
            401
        );
        assert_eq!(
            call(http.clone(), &a, "GET", "/records", Value::Null, None)
                .await
                .0,
            401
        );
        assert_eq!(
            call(
                http.clone(),
                "",
                "GET",
                "/records",
                Value::Null,
                Some(token)
            )
            .await
            .0,
            401
        );
        for field in ["tenant", "company_domain", "email", "owner", "id", "query"] {
            let mut body = json!({"name":"Forged"});
            body[field] = json!(b);
            assert_eq!(
                call(
                    http.clone(),
                    &a,
                    "POST",
                    "/records",
                    body.clone(),
                    Some(token)
                )
                .await
                .0,
                400
            );
            assert_eq!(
                call(
                    http.clone(),
                    &a,
                    "PATCH",
                    &format!("/records/{aid}"),
                    body,
                    Some(token)
                )
                .await
                .0,
                400
            );
        }
        for verb in ["PUT", "PATCH"] {
            let (s, v) = call(
                http.clone(),
                &a,
                verb,
                &format!("/records/{aid}"),
                json!({"name":"Allowed edit"}),
                Some(token),
            )
            .await;
            assert_eq!(s, 200, "Own {verb}: {v}");
            assert_eq!(v["name"], "Allowed edit");
        }
        // Remove one membership while keeping the identity token and the other membership.
        adapter.write(json!({"query":format!(r#"match $u isa user, has email "{alice}"; $t isa company, has company_domain "{a}"; $m isa biz_membership, links (tenant:$t, person:$u); delete $m;"#)})).await.unwrap();
        for (verb, path) in [
            ("GET", "/records".to_string()),
            ("POST", "/records".to_string()),
            ("GET", format!("/records/{aid}")),
            ("PUT", format!("/records/{aid}")),
            ("PATCH", format!("/records/{aid}")),
            ("DELETE", format!("/records/{aid}")),
        ] {
            assert_eq!(
                call(
                    http.clone(),
                    &a,
                    verb,
                    &path,
                    json!({"name":"Revoked"}),
                    Some(token)
                )
                .await
                .0,
                401,
                "Revoked membership {verb}"
            );
        }
        assert_eq!(
            call(
                http.clone(),
                &b,
                "GET",
                &format!("/records/{bid}"),
                Value::Null,
                Some(token)
            )
            .await
            .0,
            200
        );
        let (s, v) = call(
            http.clone(),
            &b,
            "DELETE",
            &format!("/records/{bid}"),
            Value::Null,
            Some(token),
        )
        .await;
        assert_eq!(s, 200, "Own delete: {v}");
        assert_eq!(
            call(
                http.clone(),
                &b,
                "GET",
                &format!("/records/{bid}"),
                Value::Null,
                Some(token)
            )
            .await
            .0,
            404
        );
        assert_eq!(
            call(
                http.clone(),
                &b,
                "DELETE",
                "/authentication",
                Value::Null,
                Some(token)
            )
            .await
            .0,
            200
        );
        assert_eq!(
            call(http, &b, "GET", "/records", Value::Null, Some(token))
                .await
                .0,
            401
        );
    }
    async fn prepare_fixture(state: &std::sync::Arc<crate::typedb::TypeDBState>) {
        assert_eq!(state.database, "business_dev");
        let tx = state
            .driver
            .transaction(&state.database, typedb_driver::TransactionType::Write)
            .await
            .unwrap();
        crate::access::backfill(&tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    include!("workspace_tests.rs");
    include!("identity_tests.rs");
    include!("transaction_tests.rs");
    include!("recovery_tests.rs");
    include!("soak_tests.rs");
    include!("restore_tests.rs");
    include!("query_diagnostics.rs");
}
