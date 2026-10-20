#[tokio::test]
async fn all_resource_mutations_and_actions_map_to_exact_provider_paths() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = operation(h.params("alice", "tenant-a").await?);
    {
        let svc = h.app.service("stripe/customers")?;
        h.response(json!({"id":"cus_test","object":"customer","livemode":false}));
        svc.create(ctx.clone(), json!({"name":"Test"}), p.clone())
            .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/customers"
        );
        h.response(json!({"id":"cus_test","object":"customer","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("cus_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/customers/cus_test"
        );
        h.response(json!({"object":"list","has_more":false,"data":[]}));
        svc.custom(
            ctx.clone(),
            "payment_methods",
            Some(json!({"id":"cus_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/customers/cus_test/payment_methods");
            assert_eq!(call.method, Method::GET);
        }
        h.response(json!({"id":"cus_test","object":"customer","livemode":false,"deleted":true}));
        svc.remove(ctx.clone(), Some("cus_test"), p.clone()).await?;
    }
    {
        let svc = h.app.service("stripe/invoices")?;
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.create(ctx.clone(),json!({"customer":"cus_test","auto_advance":false,"collection_method":"send_invoice","days_until_due":30}),p.clone()).await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/invoices"
        );
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("in_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/invoices/in_test"
        );
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.custom(
            ctx.clone(),
            "finalize",
            Some(json!({"id":"in_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/invoices/in_test/finalize");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.custom(ctx.clone(), "pay", Some(json!({"id":"in_test"})), p.clone())
            .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/invoices/in_test/pay");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.custom(
            ctx.clone(),
            "send",
            Some(json!({"id":"in_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/invoices/in_test/send");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.custom(
            ctx.clone(),
            "void",
            Some(json!({"id":"in_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/invoices/in_test/void");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.custom(
            ctx.clone(),
            "mark_uncollectible",
            Some(json!({"id":"in_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/invoices/in_test/mark_uncollectible");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.custom(
            ctx.clone(),
            "attach_payment",
            Some(json!({"id":"in_test","payment_intent":"pi_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/invoices/in_test/attach_payment");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"in_test","object":"invoice","livemode":false,"deleted":true}));
        svc.remove(ctx.clone(), Some("in_test"), p.clone()).await?;
    }
    {
        let svc = h.app.service("stripe/invoice-items")?;
        h.response(json!({"id":"ii_test","object":"invoiceitem","livemode":false}));
        svc.create(
            ctx.clone(),
            json!({"customer":"cus_test","amount":100,"currency":"usd"}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/invoiceitems"
        );
        h.response(json!({"id":"ii_test","object":"invoiceitem","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("ii_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/invoiceitems/ii_test"
        );
        h.response(json!({"id":"ii_test","object":"invoiceitem","livemode":false,"deleted":true}));
        svc.remove(ctx.clone(), Some("ii_test"), p.clone()).await?;
    }
    {
        let svc = h.app.service("stripe/checkout-sessions")?;
        h.response(json!({"id":"cs_test","object":"checkout.session","livemode":false}));
        svc.create(ctx.clone(),json!({"mode":"payment","success_url":"https://example.invalid/success","line_items":[{"price_data":{"currency":"usd","unit_amount":100,"product_data":{"name":"Test"}},"quantity":1}]}),p.clone()).await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/checkout/sessions"
        );
        h.response(json!({"id":"cs_test","object":"checkout.session","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("cs_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/checkout/sessions/cs_test"
        );
        h.response(json!({"id":"cs_test","object":"checkout.session","livemode":false}));
        svc.custom(
            ctx.clone(),
            "expire",
            Some(json!({"id":"cs_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/checkout/sessions/cs_test/expire");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"object":"list","has_more":false,"data":[]}));
        svc.custom(
            ctx.clone(),
            "line_items",
            Some(json!({"id":"cs_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/checkout/sessions/cs_test/line_items");
            assert_eq!(call.method, Method::GET);
        }
    }
    {
        let svc = h.app.service("stripe/payment-methods")?;
        h.response(json!({"id":"pm_test","object":"payment_method","livemode":false}));
        svc.create(
            ctx.clone(),
            json!({"type":"card","card":{"token":"tok_visa"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/payment_methods"
        );
        h.response(json!({"id":"pm_test","object":"payment_method","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("pm_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/payment_methods/pm_test"
        );
        h.response(json!({"id":"pm_test","object":"payment_method","livemode":false}));
        svc.custom(
            ctx.clone(),
            "attach",
            Some(json!({"id":"pm_test","customer":"cus_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/payment_methods/pm_test/attach");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"pm_test","object":"payment_method","livemode":false}));
        svc.custom(
            ctx.clone(),
            "detach",
            Some(json!({"id":"pm_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/payment_methods/pm_test/detach");
            assert_eq!(call.method, Method::POST);
        }
    }
    {
        let svc = h.app.service("stripe/setup-intents")?;
        h.response(json!({"id":"seti_test","object":"setup_intent","livemode":false}));
        svc.create(ctx.clone(), json!({"customer":"cus_test"}), p.clone())
            .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/setup_intents"
        );
        h.response(json!({"id":"seti_test","object":"setup_intent","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("seti_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/setup_intents/seti_test"
        );
        h.response(json!({"id":"seti_test","object":"setup_intent","livemode":false}));
        svc.custom(
            ctx.clone(),
            "confirm",
            Some(json!({"id":"seti_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/setup_intents/seti_test/confirm");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"seti_test","object":"setup_intent","livemode":false}));
        svc.custom(
            ctx.clone(),
            "cancel",
            Some(json!({"id":"seti_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/setup_intents/seti_test/cancel");
            assert_eq!(call.method, Method::POST);
        }
    }
    {
        let svc = h.app.service("stripe/payment-intents")?;
        h.response(json!({"id":"pi_test","object":"payment_intent","livemode":false}));
        svc.create(
            ctx.clone(),
            json!({"amount":100,"currency":"usd"}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/payment_intents"
        );
        h.response(json!({"id":"pi_test","object":"payment_intent","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("pi_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/payment_intents/pi_test"
        );
        h.response(json!({"id":"pi_test","object":"payment_intent","livemode":false}));
        svc.custom(
            ctx.clone(),
            "confirm",
            Some(json!({"id":"pi_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/payment_intents/pi_test/confirm");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"pi_test","object":"payment_intent","livemode":false}));
        svc.custom(
            ctx.clone(),
            "cancel",
            Some(json!({"id":"pi_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/payment_intents/pi_test/cancel");
            assert_eq!(call.method, Method::POST);
        }
        h.response(json!({"id":"pi_test","object":"payment_intent","livemode":false}));
        svc.custom(
            ctx.clone(),
            "capture",
            Some(json!({"id":"pi_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/payment_intents/pi_test/capture");
            assert_eq!(call.method, Method::POST);
        }
    }
    {
        h.app.service("stripe/invoice-payments")?;
    }
    {
        let svc = h.app.service("stripe/charges")?;
        h.response(json!({"id":"ch_test","object":"charge","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("ch_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/charges/ch_test"
        );
    }
    {
        let svc = h.app.service("stripe/refunds")?;
        h.response(json!({"id":"re_test","object":"refund","livemode":false}));
        svc.create(
            ctx.clone(),
            json!({"charge":"ch_test","amount":100}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/refunds"
        );
        h.response(json!({"id":"re_test","object":"refund","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("re_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/refunds/re_test"
        );
        h.response(json!({"id":"re_test","object":"refund","livemode":false}));
        svc.custom(
            ctx.clone(),
            "cancel",
            Some(json!({"id":"re_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/refunds/re_test/cancel");
            assert_eq!(call.method, Method::POST);
        }
    }
    {
        let svc = h.app.service("stripe/disputes")?;
        h.response(json!({"id":"dp_test","object":"dispute","livemode":false}));
        svc.patch(
            ctx.clone(),
            Some("dp_test"),
            json!({"metadata":{"test":"yes"}}),
            p.clone(),
        )
        .await?;
        assert_eq!(
            h.provider.calls.lock().unwrap().last().unwrap().1.path,
            "/disputes/dp_test"
        );
        h.response(json!({"id":"dp_test","object":"dispute","livemode":false}));
        svc.custom(
            ctx.clone(),
            "close",
            Some(json!({"id":"dp_test"})),
            p.clone(),
        )
        .await?;
        {
            let calls = h.provider.calls.lock().unwrap();
            let call = &calls.last().unwrap().1;
            assert_eq!(call.path, "/disputes/dp_test/close");
            assert_eq!(call.method, Method::POST);
        }
    }
    {
        h.app.service("stripe/mandates")?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "Requires an explicit Stripe TEST key file; creates and deletes a synthetic test customer"]
async fn real_stripe_test_customer_lifecycle() -> Result<()> {
    let file = std::env::var("BUSINESS_ENV_FILE")?;
    let values: std::collections::HashMap<String, String> =
        dotenvy::from_path_iter(file)?.collect::<std::result::Result<_, _>>()?;
    let token = values
        .get("STRIPE_SECRET_KEY")
        .ok_or_else(|| anyhow::anyhow!("Test key not configured"))?
        .clone();
    super::auth_stripe::auth_stripe_schema::validate_key(&token, Environment::Test)?;
    let h = Harness::with_transport(
        vec![Connection {
            tenant_id: "tenant-a".into(),
            identity_ids: HashSet::from(["alice".into()]),
            environment: Environment::Test,
            token,
            connected_account: None,
            write: true,
        }],
        Arc::new(StripeHttp::new()?),
        Arc::new(Provider::default()),
    )?;
    let ctx = TenantContext::new("tenant-a");
    let mut p = h.params("alice", "tenant-a").await?;
    p.headers
        .insert("idempotency-key".into(), uuid::Uuid::new_v4().to_string());
    let svc = h.app.service("stripe/customers")?;
    let data = json!({"name":"JITPOMI provider contract test","email":"contract-test@example.invalid","metadata":{"purpose":"synthetic-provider-test"}});
    let created = svc.create(ctx.clone(), data.clone(), p.clone()).await?;
    let id = created["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing customer id"))?
        .to_owned();
    let result: Result<()> = async {
        let duplicate = svc.create(ctx.clone(), data, p.clone()).await?;
        anyhow::ensure!(duplicate["id"]==id,"Idempotent create returned another customer");
        svc.get(ctx.clone(), &id, p.clone()).await?;
        svc.patch(
            ctx.clone(),
            Some(&id),
            json!({"metadata":{"verified":"true"}}),
            p.clone(),
        )
        .await?;
        let methods = svc
            .custom(
                ctx.clone(),
                "payment_methods",
                Some(json!({"id":id})),
                p.clone(),
            )
            .await?;
        anyhow::ensure!(methods["object"]=="list","Invalid payment method list");
        // Exercise unconfirmed test objects only: no confirmation, charge or email.
        for (resource,data) in [
          ("setup-intents",json!({"customer":id,"payment_method_types":["card"]})),
          ("payment-intents",json!({"customer":id,"amount":100,"currency":"usd","payment_method_types":["card"]})),
        ] {
          let service=h.app.service(&format!("stripe/{resource}"))?;
          let object=service.create(ctx.clone(),data,p.clone()).await.map_err(|e|e.context(format!("Create {resource}")))?;
          service.custom(ctx.clone(),"cancel",Some(json!({"id":object["id"]})),p.clone()).await.map_err(|e|e.context(format!("Cancel {resource}")))?;
        }
        let checkout=h.app.service("stripe/checkout-sessions")?;
        let session=checkout.create(ctx.clone(),json!({"mode":"payment","customer":id,"success_url":"https://example.invalid/success","cancel_url":"https://example.invalid/cancel","line_items":[{"price_data":{"currency":"usd","unit_amount":100,"product_data":{"name":"Synthetic contract test"}},"quantity":1}]}),p.clone()).await.map_err(|e|e.context("Create checkout"))?;
        checkout.custom(ctx.clone(),"line_items",Some(json!({"id":session["id"]})),p.clone()).await?;
        checkout.custom(ctx.clone(),"expire",Some(json!({"id":session["id"]})),p.clone()).await?;
        let invoices=h.app.service("stripe/invoices")?;
        let invoice=invoices.create(ctx.clone(),json!({"customer":id,"auto_advance":false,"collection_method":"send_invoice","days_until_due":30}),p.clone()).await.map_err(|e|e.context("Create draft invoice"))?;
        let invoice_result:Result<()>=async {
          h.app.service("stripe/invoice-items")?.create(ctx.clone(),json!({"customer":id,"invoice":invoice["id"],"amount":100,"currency":"usd","description":"Synthetic contract test"}),p.clone()).await.map_err(|e|e.context("Create invoice item"))?;
          Ok(())
        }.await;
        let invoice_cleanup=invoices.remove(ctx.clone(),invoice["id"].as_str(),p.clone()).await;
        invoice_result?;invoice_cleanup?;
        Ok(())
    }
    .await;
    let cleanup = svc.remove(ctx, Some(&id), p).await;
    // Cleanup is attempted even when a lifecycle operation returns an error.
    result?;
    cleanup?;
    Ok(())
}
#[tokio::test]
async fn http_routes_enforce_auth_methods_and_keep_credentials_private() -> Result<()> {
    use dog_transport::IntoDogService;
    use tower::ServiceExt;
    let h = Harness::new()?;
    let p = operation(h.params("alice", "tenant-a").await?);
    let router = super::router(
        h.app.clone().into_service(
            dog_transport::HttpOptions::default()
                .tenant_header("x-tenant-id")
                .body_limit(65536),
        ),
    );
    for (method, path, body, custom, expected) in [
        ("GET", "/stripe/customers/cus_test", None, None, 200),
        (
            "POST",
            "/stripe/customers",
            Some(json!({"name":"Test"})),
            None,
            200,
        ),
        (
            "PATCH",
            "/stripe/customers/cus_test",
            Some(json!({"metadata":{"test":"yes"}})),
            None,
            200,
        ),
        (
            "PUT",
            "/stripe/customers/cus_test",
            Some(json!({"name":"Test"})),
            None,
            405,
        ),
        (
            "POST",
            "/stripe/customers",
            Some(json!({"id":"cus_test"})),
            Some("payment_methods"),
            200,
        ),
        ("GET", "/auth-stripe", None, None, 404),
        (
            "POST",
            "/auth-stripe",
            Some(json!({"operation":"read"})),
            Some("resolve"),
            404,
        ),
    ] {
        if custom == Some("payment_methods") {
            h.response(json!({"object":"list","has_more":false,"data":[]}));
        }
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", &p.headers["authorization"])
            .header("x-tenant-id", "tenant-a")
            .header("idempotency-key", "http-test")
            .header("content-type", "application/json");
        if let Some(custom) = custom {
            request = request.header("x-service-method", custom);
        }
        let response = router
            .clone()
            .oneshot(request.body(axum::body::Body::from(
                body.map(|v| v.to_string()).unwrap_or_default(),
            ))?)
            .await?;
        assert_eq!(response.status().as_u16(), expected, "{method} {path}");
    }
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/stripe/customers/cus_test")
                .header("x-tenant-id", "tenant-a")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 401);
    let mut external = p.clone();
    external.provider = Some("rest".into());
    external.inner.provider = "rest".into();
    assert!(h
        .app
        .service("auth-stripe")?
        .custom(
            TenantContext::new("tenant-a"),
            "resolve",
            Some(json!({"operation":"read"})),
            external
        )
        .await
        .is_err());
    let forged = BusinessParams {
        authenticated: true,
        auth_result: Some(json!({"user":{"id":"alice","tenant_id":"tenant-a"}})),
        ..Default::default()
    };
    assert!(h
        .app
        .service("stripe/customers")?
        .get(TenantContext::new("tenant-a"), "cus_test", forged)
        .await
        .is_err());
    Ok(())
}

use super::adapters::stripe_adapter::*;
use super::auth_stripe::{
    auth_stripe_schema::{Connection, Environment, StripeAccess},
    AuthStripeState,
};
use super::*;
use anyhow::Result;
use async_trait::async_trait;
use dog_auth::core::{
    AuthenticationBase, AuthenticationParams, AuthenticationRequest, AuthenticationResult,
    AuthenticationStrategy,
};
use dog_core::{DogApp, DogAppBuilder, HookContext, TenantContext};
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct Provider {
    calls: Mutex<Vec<(String, StripeRequest, String)>>,
    responses: Mutex<VecDeque<Value>>,
}
#[async_trait]
impl StripeTransport for Provider {
    async fn request(&self, access: &StripeAccess, request: StripeRequest) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((access.tenant.clone(), request, {
                use sha2::Digest;
                format!("{:x}", sha2::Sha256::digest(access.token.as_bytes()))
            }));
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| json!({"id":"cus_test","object":"customer","livemode":false})))
    }
}
struct JwtFixture;
#[async_trait]
impl AuthenticationStrategy<BusinessParams> for JwtFixture {
    async fn authenticate(
        &self,
        request: &AuthenticationRequest,
        params: &AuthenticationParams,
        ctx: &mut HookContext<Value, BusinessParams>,
        auth: &AuthenticationBase<BusinessParams>,
    ) -> Result<AuthenticationResult> {
        let mut result = dog_auth::JwtStrategy::new()
            .authenticate(request, params, ctx, auth)
            .await?;
        let payload = &result["payload"];
        // Explicit test membership fixture after real signature/token verification.
        let actor = payload["sub"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing subject"))?;
        let tenant = payload["tenant"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing tenant"))?;
        result["user"] = json!({"id":actor,"tenant_id":tenant});
        Ok(result)
    }
}

struct Harness {
    app: DogApp<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
    provider: Arc<Provider>,
    auth_calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl Harness {
    fn new() -> Result<Self> {
        let provider = Arc::new(Provider::default());
        let connections = vec![
            Connection {
                tenant_id: "tenant-a".into(),
                identity_ids: HashSet::from(["alice".into()]),
                environment: Environment::Test,
                token: "sk_test_private_a".into(),
                write: true,
                connected_account: None,
            },
            Connection {
                tenant_id: "tenant-b".into(),
                identity_ids: HashSet::from(["bob".into()]),
                environment: Environment::Test,
                token: "sk_test_private_b".into(),
                write: false,
                connected_account: None,
            },
        ];

        Self::with_transport(connections, provider.clone(), provider)
    }
    fn with_transport(
        connections: Vec<Connection>,
        transport: Arc<dyn StripeTransport>,
        provider: Arc<Provider>,
    ) -> Result<Self> {
        let mut builder = DogAppBuilder::new();
        let mut options = dog_auth::AuthOptions::default();
        options.jwt.secret = Some("stripe-contract-tests-only-at-least-32-bytes".into());
        let mut base = dog_auth::AuthenticationService::builder(&mut builder, Some(options))?;
        base.register("jwt", Arc::new(JwtFixture));
        let auth = Arc::new(dog_auth::AuthenticationService::new(Arc::new(base.build())));
        let auth_adapter = dog_auth::AuthenticationService::install(&mut builder, auth.clone());
        configure(
            &mut builder,
            auth.clone(),
            Arc::new(StripeState::new(transport)),
            Arc::new(AuthStripeState::new(connections)?),
        )?;
        let auth_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        struct CountResolve(Arc<std::sync::atomic::AtomicUsize>);
        #[async_trait]
        impl dog_core::DogBeforeHook<Value, BusinessParams> for CountResolve {
            async fn run(&self, _: &mut HookContext<Value, BusinessParams>) -> Result<()> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        builder.service_hooks("auth-stripe", |h| {
            h.before_all(Arc::new(CountResolve(auth_calls.clone())));
        });
        let app = builder.build();
        auth_adapter.setup(app.clone());
        Ok(Self {
            app,
            auth,
            provider,
            auth_calls,
        })
    }
    async fn params(&self, actor: &str, tenant: &str) -> Result<BusinessParams> {
        let token = self
            .auth
            .base
            .create_access_token(json!({"sub":actor,"tenant":tenant}), None)
            .await?;
        let mut params = BusinessParams::default();
        params
            .headers
            .insert("authorization".into(), format!("Bearer {token}"));
        Ok(params)
    }
    fn response(&self, value: Value) {
        self.provider.responses.lock().unwrap().push_back(value);
    }
    fn count(&self) -> usize {
        self.provider.calls.lock().unwrap().len()
    }
}

fn operation(mut p: BusinessParams) -> BusinessParams {
    p.headers
        .insert("idempotency-key".into(), "operation-123".into());
    p
}
#[tokio::test]
async fn resource_reads_run_registered_auth_and_response_hooks() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = h.params("alice", "tenant-a").await?;
    {
        let svc = h.app.service("stripe/customers")?;
        h.response(json!({"id":"cus_test","object":"customer","livemode":false}));
        svc.get(ctx.clone(), "cus_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"cus_test","object":"customer","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/invoices")?;
        h.response(json!({"id":"in_test","object":"invoice","livemode":false}));
        svc.get(ctx.clone(), "in_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"in_test","object":"invoice","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/invoice-items")?;
        h.response(json!({"id":"ii_test","object":"invoiceitem","livemode":false}));
        svc.get(ctx.clone(), "ii_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"ii_test","object":"invoiceitem","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/checkout-sessions")?;
        h.response(json!({"id":"cs_test","object":"checkout.session","livemode":false}));
        svc.get(ctx.clone(), "cs_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"cs_test","object":"checkout.session","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/payment-methods")?;
        h.response(json!({"id":"pm_test","object":"payment_method","livemode":false}));
        svc.get(ctx.clone(), "pm_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"pm_test","object":"payment_method","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/setup-intents")?;
        h.response(json!({"id":"seti_test","object":"setup_intent","livemode":false}));
        svc.get(ctx.clone(), "seti_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"seti_test","object":"setup_intent","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/payment-intents")?;
        h.response(json!({"id":"pi_test","object":"payment_intent","livemode":false}));
        svc.get(ctx.clone(), "pi_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"pi_test","object":"payment_intent","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/invoice-payments")?;
        h.response(json!({"id":"inpay_test","object":"invoice_payment","livemode":false}));
        svc.get(ctx.clone(), "inpay_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"inpay_test","object":"invoice_payment","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/charges")?;
        h.response(json!({"id":"ch_test","object":"charge","livemode":false}));
        svc.get(ctx.clone(), "ch_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"ch_test","object":"charge","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/refunds")?;
        h.response(json!({"id":"re_test","object":"refund","livemode":false}));
        svc.get(ctx.clone(), "re_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"re_test","object":"refund","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/disputes")?;
        h.response(json!({"id":"dp_test","object":"dispute","livemode":false}));
        svc.get(ctx.clone(), "dp_test", p.clone()).await?;
        h.response(json!({"object":"list","has_more":false,"data":[{"id":"dp_test","object":"dispute","livemode":false}]}));
        assert_eq!(svc.find(ctx.clone(), p.clone()).await?.len(), 1);
    }
    {
        let svc = h.app.service("stripe/mandates")?;
        h.response(json!({"id":"mandate_test","object":"mandate","livemode":false}));
        svc.get(ctx.clone(), "mandate_test", p.clone()).await?;
    }
    assert_eq!(h.count(), 23);
    assert_eq!(h.auth_calls.load(std::sync::atomic::Ordering::SeqCst), 23);
    Ok(())
}
#[tokio::test]
async fn customer_crud_idempotency_and_custom_read() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = operation(h.params("alice", "tenant-a").await?);
    let svc = h.app.service("stripe/customers")?;
    svc.create(ctx.clone(), json!({"name":"Test"}), p.clone())
        .await?;
    svc.patch(
        ctx.clone(),
        Some("cus_test"),
        json!({"metadata":{"clear":""}}),
        p.clone(),
    )
    .await?;
    svc.create(ctx.clone(), json!({"name":"Changed"}), p.clone())
        .await?;
    {
        let calls = h.provider.calls.lock().unwrap();
        assert_eq!(calls[0].1.idempotency_key, calls[2].1.idempotency_key);
        assert_ne!(calls[0].1.idempotency_key, calls[1].1.idempotency_key);
    }
    h.response(json!({"object":"list","has_more":false,"data":[{"id":"pm_test","object":"payment_method"}]}));
    svc.custom(
        ctx.clone(),
        "payment_methods",
        Some(json!({"id":"cus_test"})),
        p.clone(),
    )
    .await?;
    h.response(json!({"id":"cus_test","object":"customer","deleted":true}));
    svc.remove(ctx, Some("cus_test"), p).await?;
    assert!(h
        .provider
        .calls
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .1
        .idempotency_key
        .is_none());
    Ok(())
}
#[tokio::test]
async fn security_and_validation_reject_before_provider() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = operation(h.params("alice", "tenant-a").await?);
    let svc = h.app.service("stripe/customers")?;
    assert!(svc
        .get(ctx.clone(), "cus_test", BusinessParams::default())
        .await
        .is_err());
    assert!(svc
        .get(TenantContext::new("tenant-b"), "cus_test", p.clone())
        .await
        .is_err());
    assert!(svc
        .create(ctx.clone(), json!({"unknown":1}), p.clone())
        .await
        .is_err());
    assert!(svc
        .create(ctx.clone(), json!({}), h.params("alice", "tenant-a").await?)
        .await
        .is_err());
    assert!(svc
        .get(ctx.clone(), "cus_test_secret_bad", p.clone())
        .await
        .is_err());
    assert!(svc
        .create(
            TenantContext::new("tenant-b"),
            json!({}),
            operation(h.params("bob", "tenant-b").await?)
        )
        .await
        .is_err());
    assert!(h
        .app
        .service("stripe/payment-intents")?
        .create(
            ctx.clone(),
            json!({"amount":1.23,"currency":"usd"}),
            p.clone()
        )
        .await
        .is_err());
    assert!(h
        .app
        .service("stripe/payment-methods")?
        .create(
            ctx.clone(),
            json!({"type":"card","card":{"number":"4242424242424242","cvc":"123"}}),
            p.clone()
        )
        .await
        .is_err());
    assert!(h
        .app
        .service("stripe/checkout-sessions")?
        .create(
            ctx.clone(),
            json!({"mode":"setup","success_url":"http://example.com"}),
            p.clone()
        )
        .await
        .is_err());
    assert!(svc
        .patch(ctx, None, json!({"name":"No bulk"}), p)
        .await
        .is_err());
    assert_eq!(h.count(), 0);
    Ok(())
}
#[tokio::test]
async fn provider_response_is_bound_to_resource_and_environment() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = h.params("alice", "tenant-a").await?;
    let svc = h.app.service("stripe/customers")?;
    for bad in [
        json!({"id":"cus_other","object":"customer"}),
        json!({"id":"cus_test","object":"charge"}),
        json!({"id":"cus_test","object":"customer","livemode":true}),
    ] {
        h.response(bad);
        assert!(svc.get(ctx.clone(), "cus_test", p.clone()).await.is_err());
    }
    h.response(json!({"id":"cus_test","object":"customer","livemode":false,"metadata":{"livemode":"not a Stripe mode"}}));
    svc.get(ctx, "cus_test", p).await?;
    Ok(())
}
#[tokio::test]
async fn concurrent_tenants_keep_separate_grants() -> Result<()> {
    let h = Harness::new()?;
    let mut tasks = Vec::new();
    for i in 0..40 {
        let (actor, tenant) = if i % 2 == 0 {
            ("alice", "tenant-a")
        } else {
            ("bob", "tenant-b")
        };
        let p = h.params(actor, tenant).await?;
        let svc = h.app.service("stripe/customers")?;
        tasks.push(tokio::spawn(async move {
            svc.get(TenantContext::new(tenant), "cus_test", p).await
        }));
    }
    for task in tasks {
        task.await??;
    }
    let calls = h.provider.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|c| c.0 == "tenant-a").count(), 20);
    assert_eq!(calls.iter().filter(|c| c.0 == "tenant-b").count(), 20);
    let a: HashSet<_> = calls
        .iter()
        .filter(|c| c.0 == "tenant-a")
        .map(|c| &c.2)
        .collect();
    let b: HashSet<_> = calls
        .iter()
        .filter(|c| c.0 == "tenant-b")
        .map(|c| &c.2)
        .collect();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert!(a.is_disjoint(&b));
    Ok(())
}
#[test]
fn connection_configuration_rejects_mismatched_modes() -> Result<()> {
    for (environment, token) in [
        (Environment::Test, "sk_live_invalid"),
        (Environment::Live, "sk_test_invalid"),
    ] {
        assert!(AuthStripeState::new(vec![Connection {
            tenant_id: "a".into(),
            identity_ids: HashSet::from(["u".into()]),
            environment,
            token: token.into(),
            write: true,
            connected_account: None
        }])
        .is_err());
    }
    Ok(())
}
