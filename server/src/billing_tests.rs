// Exercise the public routes and real TypeDB policy functions without provider writes.
#[tokio::test]
#[ignore = "Requires business_dev with migrate-billing applied"]
async fn hosted_billing_isolation_and_schedule() {
    use crate::services::billing::{engine::Engine, providers::Providers};
    let (app, http) = build().await.unwrap();
    let state = app
        .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
        .unwrap();
    assert_eq!(state.database, "business_dev");
    assert!(
        std::env::var("BUSINESS_TEST_BASE_URL").is_err(),
        "Exercise current local code"
    );
    let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let hash = bcrypt::hash("billing-test-only", 4).unwrap();
    let mut identities = Vec::new();
    for prefix in ["seller", "buyer", "stranger"] {
        let domain = format!("{prefix}-{tag}.example.com");
        let email = format!("{prefix}-{tag}@example.com");
        adapter.write(json!({"query":format!(r#"insert $u isa user,has email "{email}",has password "{hash}";$t isa company,has company_domain "{domain}";(tenant:$t,member:$u) isa tenant_membership;"#)})).await.unwrap();
        prepare_fixture(&state).await;
        let (s, v) = call(
            http.clone(),
            &domain,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":email,"password":"billing-test-only"}),
            None,
        )
        .await;
        assert_eq!(s, 200, "{v}");
        let tenant = v["user"]["tenant_id"].as_str().unwrap().to_owned();
        let person = v["user"]["id"].as_str().unwrap();
        adapter.write(json!({"query":format!(r#"match $m isa biz_membership,has biz_id "{}";update $m has biz_role "owner";"#,crate::access::pair(&tenant,person).unwrap())})).await.unwrap();
        identities.push((
            domain,
            tenant,
            v["accessToken"].as_str().unwrap().to_owned(),
        ));
    }
    let seller = &identities[0];
    let buyer = &identities[1];
    let stranger = &identities[2];
    let (s,customer)=call(http.clone(),&seller.0,"POST","/billing-customers",json!({"buyer_id":buyer.1,"name":"Billing test buyer","email":format!("buyer-{tag}@example.com"),"policy":"customer","manual_provider":"mercury"}),Some(&seller.2)).await;
    assert_eq!(s, 200, "Create customer: {customer}");
    let id = customer["id"].as_str().unwrap();
    let path = format!("/billing-customers/{id}");
    for actor in [seller, buyer] {
        let (s, v) = call(
            http.clone(),
            &actor.0,
            "GET",
            &path,
            Value::Null,
            Some(&actor.2),
        )
        .await;
        assert_eq!(s, 200, "{v}");
        assert!(v.get("payment_method").is_none());
    }
    let (s, _) = call(
        http.clone(),
        &stranger.0,
        "GET",
        &path,
        Value::Null,
        Some(&stranger.2),
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = call(
        http.clone(),
        &buyer.0,
        "PATCH",
        &path,
        json!({"policy":"automatic"}),
        Some(&buyer.2),
    )
    .await;
    assert_eq!(s, 404, "Buyer cannot set merchant policy");
    let (s, v) = call(
        http.clone(),
        &buyer.0,
        "PATCH",
        &path,
        json!({"choice":"automatic"}),
        Some(&buyer.2),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = call(
        http.clone(),
        &buyer.0,
        "PATCH",
        &path,
        json!({"payment_method":"pm_attacker","consent":true}),
        Some(&buyer.2),
    )
    .await;
    assert_eq!(s, 400);
    let start = chrono::Utc::now() - chrono::Duration::seconds(1);
    let plan = json!({"customer_id":id,"description":"One-off test","amount_cents":1250,"start_at":start.to_rfc3339(),"interval":"once","due_days":7});
    let (s, _) = call(
        http.clone(),
        &buyer.0,
        "POST",
        "/billing-plans",
        plan.clone(),
        Some(&buyer.2),
    )
    .await;
    assert_eq!(s, 404, "Buyer cannot create charge schedules");
    let (s, p) = call(
        http.clone(),
        &seller.0,
        "POST",
        "/billing-plans",
        plan,
        Some(&seller.2),
    )
    .await;
    assert_eq!(s, 200, "Create plan: {p}");
    let engine = Engine {
        state: state.clone(),
        merchant: seller.1.clone(),
        providers: Providers::new().unwrap(),
    };
    let c = engine.customer(id).await.unwrap();
    assert_eq!(c["consent"], "");
    let now = chrono::Utc::now().timestamp();
    let (first, second) = tokio::join!(engine.enqueue_due(now), engine.enqueue_due(now));
    assert!(
        first.is_ok() || second.is_ok(),
        "Neither worker succeeded: {first:?} {second:?}"
    );
    assert_eq!(
        engine.enqueue_due(now).await.unwrap(),
        0,
        "Sequence must not be reissued"
    );
    let (s, list) = call(
        http.clone(),
        &seller.0,
        "GET",
        "/billing-invoices",
        Value::Null,
        Some(&seller.2),
    )
    .await;
    assert_eq!(s, 200, "List invoices: {list}");
    assert_eq!(list.as_array().unwrap().len(), 1, "Exactly one occurrence");
    let invoice = &list[0];
    assert_eq!(invoice["amount_cents"].as_f64(), Some(1250.0));
    assert_eq!(invoice["status"], "queued");
    let (s, foreign) = call(
        http.clone(),
        &stranger.0,
        "GET",
        "/billing-invoices",
        Value::Null,
        Some(&stranger.2),
    )
    .await;
    assert_eq!(s, 200, "{foreign}");
    assert_eq!(foreign, json!([]));
    let (s, _) = call(
        http.clone(),
        &seller.0,
        "POST",
        "/billing-invoices",
        json!({"status":"paid"}),
        Some(&seller.2),
    )
    .await;
    assert!(!(200..300).contains(&s));
    // Exercise due-payment query with no consent: no external call is permitted.
    engine
        .collect_due(invoice["id"].as_str().unwrap(), now + 86400 * 8)
        .await
        .unwrap();
    if std::env::var("BILLING_PROVIDER_TEST").as_deref() == Ok("true") {
        use crate::services::billing::{billing_schema::Provider, providers::pairs};
        dotenvy::from_path(
            std::env::var("BILLING_TEST_ENV")
                .expect("Set BILLING_TEST_ENV to the protected sandbox configuration"),
        )
        .unwrap();
        assert!(!engine.providers.live);
        let customer = engine.customer(id).await.unwrap();
        let external = engine
            .ensure_customer(&customer, Provider::Stripe)
            .await
            .unwrap();
        let setup = engine
            .providers
            .stripe(
                reqwest::Method::POST,
                "/setup_intents",
                &format!("fixture-{id}"),
                &pairs(&[
                    ("customer", &external),
                    ("payment_method", "pm_card_visa"),
                    ("payment_method_types[0]", "card"),
                    ("usage", "off_session"),
                    ("confirm", "true"),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(setup["status"], "succeeded");
        let card = setup["payment_method"].as_str().unwrap();
        let revision = customer["revision"].as_str().unwrap();
        // Trusted test fixture only: the public API cannot insert payment references.
        engine.write(format!(r#"{} select $t,$c;distinct;update $c has bill_consent "{revision}",has bill_payment_method "{card}";fetch {{"id":$c.bill_key}};"#,engine.customer_match(id).unwrap())).await.unwrap();
        let invoice_id = invoice["id"].as_str().unwrap();
        engine.issue(invoice_id).await.unwrap();
        let (s, v) = call(
            http.clone(),
            &seller.0,
            "GET",
            &format!("/billing-invoices/{invoice_id}"),
            Value::Null,
            Some(&seller.2),
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(v["status"], "open");
        assert_eq!(v["mode"], "automatic");
        let (a, b) = tokio::join!(
            engine.collect_due(invoice_id, now + 8 * 86400),
            engine.collect_due(invoice_id, now + 8 * 86400)
        );
        assert!(a.is_ok() || b.is_ok(), "{a:?} {b:?}");
        engine.reconcile(invoice_id).await.unwrap();
        let (s, v) = call(
            http.clone(),
            &buyer.0,
            "GET",
            &format!("/billing-invoices/{invoice_id}"),
            Value::Null,
            Some(&buyer.2),
        )
        .await;
        assert_eq!(s, 200);
        assert_eq!(v["status"], "paid");
        engine
            .collect_due(invoice_id, now + 8 * 86400)
            .await
            .unwrap();
        assert_eq!(engine.notification_candidates().await.unwrap().len(), 1);
    }

    if let Ok(path) = std::env::var("BILLING_UI_FIXTURE_FILE") {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let value = json!({"seller_domain":seller.0,"merchant_id":seller.1,"seller_email":format!("seller-{tag}@example.com"),"buyer_domain":buyer.0,"buyer_id":buyer.1,"buyer_email":format!("buyer-{tag}@example.com"),"customer_id":id});
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(serde_json::to_string(&value).unwrap().as_bytes())
            .unwrap();
    }
}

/// Real Cloud database + Stripe sandbox + real test-recipient email delivery.
/// Saved-card consent is a trusted fixture; interactive Checkout is a separate test.
#[tokio::test]
#[ignore = "Requires approved sandbox providers, business_dev, and test email recipient"]
async fn hosted_billing_payment_and_mail_flows() {
    use crate::services::billing::{
        billing_schema::Provider,
        engine::Engine,
        providers::{pairs, Providers},
    };
    dotenvy::from_path(std::env::var("BILLING_TEST_ENV").unwrap()).unwrap();
    assert_eq!(std::env::var("BILLING_MAIL_ENABLED").as_deref(), Ok("true"));
    assert_eq!(
        std::env::var("BILLING_TEST_EMAIL").as_deref(),
        Ok("dev@jitpomi.com")
    );
    let (app, http) = build().await.unwrap();
    let state = app
        .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
        .unwrap();
    assert_eq!(state.database, "business_dev");
    assert!(std::env::var("BUSINESS_TEST_BASE_URL").is_err());
    let providers = Providers::new().unwrap();
    assert!(!providers.live, "Sandbox only");
    let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let hash = bcrypt::hash("billing-test-only", 4).unwrap();
    let mut identities = Vec::new();
    for prefix in ["seller", "buyer"] {
        let domain = format!("flow-{prefix}-{tag}.example.com");
        let email = format!("{prefix}-{tag}@example.com");
        adapter.write(json!({"query":format!(r#"insert $u isa user,has email "{email}",has password "{hash}";$t isa company,has company_domain "{domain}";(tenant:$t,member:$u) isa tenant_membership;"#)})).await.unwrap();
        prepare_fixture(&state).await;
        let (s, v) = call(
            http.clone(),
            &domain,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":email,"password":"billing-test-only"}),
            None,
        )
        .await;
        assert_eq!(s, 200, "{v}");
        let tenant = v["user"]["tenant_id"].as_str().unwrap().to_owned();
        let person = v["user"]["id"].as_str().unwrap();
        adapter.write(json!({"query":format!(r#"match $m isa biz_membership,has biz_id "{}";update $m has biz_role "owner";"#,crate::access::pair(&tenant,person).unwrap())})).await.unwrap();
        identities.push((
            domain,
            tenant,
            v["accessToken"].as_str().unwrap().to_owned(),
        ));
    }
    let seller = &identities[0];
    let buyer = &identities[1];
    let engine = Engine {
        state: state.clone(),
        merchant: seller.1.clone(),
        providers,
    };
    for (scenario, card_fixture, expected, notice) in [
        ("successful", "pm_card_visa", "paid", "receipt"),
        (
            "declined",
            "pm_card_chargeCustomerFail",
            "open",
            "reminder-0",
        ),
    ] {
        let (s,c)=call(http.clone(),&seller.0,"POST","/billing-customers",json!({"buyer_id":buyer.1,"name":format!("Sandbox {scenario} flow"),"email":"dev@jitpomi.com","policy":"customer","manual_provider":"stripe"}),Some(&seller.2)).await;
        assert_eq!(s, 200, "{c}");
        let id = c["id"].as_str().unwrap();
        let (s, v) = call(
            http.clone(),
            &buyer.0,
            "PATCH",
            &format!("/billing-customers/{id}"),
            json!({"choice":"automatic"}),
            Some(&buyer.2),
        )
        .await;
        assert_eq!(s, 200, "{v}");
        let start = chrono::Utc::now();
        let (s,p)=call(http.clone(),&seller.0,"POST","/billing-plans",json!({"customer_id":id,"description":format!("TEST {scenario} scheduled payment — no real funds"),"amount_cents":1250,"start_at":start.to_rfc3339(),"interval":"once","due_days":0}),Some(&seller.2)).await;
        assert_eq!(s, 200, "{p}");
        assert_eq!(
            engine.enqueue_due(start.timestamp() - 1).await.unwrap(),
            0,
            "No early invoice"
        );
        let customer = engine.customer(id).await.unwrap();
        let external = engine
            .ensure_customer(&customer, Provider::Stripe)
            .await
            .unwrap();
        let setup = engine
            .providers
            .stripe(
                reqwest::Method::POST,
                "/setup_intents",
                &format!("flow-setup-{id}"),
                &pairs(&[
                    ("customer", &external),
                    ("payment_method", card_fixture),
                    ("payment_method_types[0]", "card"),
                    ("usage", "off_session"),
                    ("confirm", "true"),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(setup["status"], "succeeded");
        let card = setup["payment_method"].as_str().unwrap();
        let revision = customer["revision"].as_str().unwrap();
        engine.write(format!(r#"{} select $t,$c;distinct;update $c has bill_consent "{revision}",has bill_payment_method "{card}";fetch {{"id":$c.bill_key}};"#,engine.customer_match(id).unwrap())).await.unwrap();
        let result = engine.tick().await.unwrap();
        assert_eq!(result["scheduled"], 1, "{result}");
        assert_eq!(result["issued"], 1, "{result}");
        assert_eq!(result["errors"], 0, "{result}");
        assert_eq!(result["notices"], 1, "{result}");
        let (s, invoices) = call(
            http.clone(),
            &buyer.0,
            "GET",
            "/billing-invoices",
            Value::Null,
            Some(&buyer.2),
        )
        .await;
        assert_eq!(s, 200, "{invoices}");
        // Query by customer to avoid relying on the public projection including plan_id.
        let rows=engine.read(format!(r#"{} (customer:$c,plan:$p) isa bill_plan_owner;(plan:$p,invoice:$i) isa bill_invoice_owner;fetch {{"id":$i.bill_key,"status":$i.bill_status,"external":$i.bill_external_id}};"#,engine.customer_match(id).unwrap())).await.unwrap();
        assert_eq!(rows.len(), 1);
        let invoice_id = rows[0]["id"].as_str().unwrap();
        assert_eq!(rows[0]["status"], expected, "{rows:?}");
        let visible = invoices
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == invoice_id)
            .expect("Buyer must see their invoice");
        assert_eq!(visible["status"], expected);
        let provider = engine
            .providers
            .observe(Provider::Stripe, rows[0]["external"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(provider["status"], expected);
        assert_eq!(
            provider["amount_paid"],
            if scenario == "successful" { 1250 } else { 0 }
        );
        assert_eq!(
            provider["attempted"], true,
            "Payment must have been attempted"
        );
        let notices=engine.read(format!(r#"{} (invoice:$i,notice:$n) isa bill_notice_owner;fetch {{"key":$n.bill_key,"status":$n.bill_status}};"#,engine.invoice_match(invoice_id).unwrap())).await.unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["key"], format!("{invoice_id}-{notice}"));
        assert!(notices[0]["status"]
            .as_str()
            .unwrap()
            .starts_with("accepted:"));
        let attempts = provider["attempt_count"].clone();
        let replay = engine.tick().await.unwrap();
        assert_eq!(replay["scheduled"], 0);
        assert_eq!(replay["notices"], 0);
        assert_eq!(replay["errors"], 0);
        let again = engine
            .providers
            .observe(Provider::Stripe, rows[0]["external"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(again["attempt_count"], attempts, "No automatic recharge");
        println!("FLOW {scenario}: invoice={invoice_id}, Stripe={}, state={expected}, notice={}, replay=no duplicate",rows[0]["external"],notices[0]["status"]);
    }
}

#[tokio::test]
#[ignore = "Real business_dev invitation acceptance and isolation"]
async fn hosted_customer_onboarding() {
    let (app, http) = build().await.unwrap();
    let state = app
        .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
        .unwrap();
    assert_eq!(state.database, "business_dev");
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("BILLING_UI_FIXTURE_FILE").unwrap()).unwrap(),
    )
    .unwrap();
    let domain = fixture["seller_domain"].as_str().unwrap();
    assert_eq!(
        std::env::var("BILLING_MERCHANT_ID").unwrap(),
        fixture["merchant_id"].as_str().unwrap()
    );
    let (s, login) = call(
        http.clone(),
        domain,
        "POST",
        "/authentication",
        json!({"strategy":"local","email":fixture["seller_email"],"password":"billing-test-only"}),
        None,
    )
    .await;
    assert_eq!(s, 200, "{login}");
    let auth = login["accessToken"].as_str().unwrap();
    let email = format!("onboard-{}@example.com", uuid::Uuid::new_v4().simple());
    let (s, invite) = call(
        http.clone(),
        domain,
        "POST",
        "/onboarding",
        json!({"action":"invite","name":"Sandbox invited company","email":email}),
        Some(auth),
    )
    .await;
    assert_eq!(s, 200, "{invite}");
    let token = invite["token"].as_str().unwrap();
    let tenant = invite["tenant_id"].as_str().unwrap();
    let accept = json!({"action":"accept","token":token,"password":"test-only-password-123"});
    let (s, result) = call(
        http.clone(),
        tenant,
        "POST",
        "/onboarding",
        accept.clone(),
        None,
    )
    .await;
    assert_eq!(s, 200, "Accept: {result}");
    let (s, _) = call(http.clone(), tenant, "POST", "/onboarding", accept, None).await;
    assert_eq!(s, 401, "Invitation replay denied");
    let (s, buyer) = call(
        http.clone(),
        tenant,
        "POST",
        "/authentication",
        json!({"strategy":"local","email":email,"password":"test-only-password-123"}),
        None,
    )
    .await;
    assert_eq!(s, 200, "New account login: {buyer}");
    let bearer = buyer["accessToken"].as_str().unwrap();
    let (s, accounts) = call(
        http.clone(),
        tenant,
        "GET",
        "/billing-customers",
        Value::Null,
        Some(bearer),
    )
    .await;
    assert_eq!(s, 200, "{accounts}");
    assert_eq!(accounts.as_array().unwrap().len(), 1);
    assert_eq!(accounts[0]["id"], invite["customer_id"]);
    let (s, _) = call(
        http.clone(),
        domain,
        "GET",
        "/billing-customers",
        Value::Null,
        Some(bearer),
    )
    .await;
    assert_eq!(s, 401, "Buyer cannot enter seller tenant");
    let (s, _) = call(
        http.clone(),
        tenant,
        "POST",
        "/onboarding",
        json!({"action":"invite","name":"Forbidden","email":"no@example.com"}),
        Some(bearer),
    )
    .await;
    assert!(!(200..300).contains(&s));
    // Existing email must authenticate its existing password, never be overwritten.
    let (s, next) = call(
        http.clone(),
        domain,
        "POST",
        "/onboarding",
        json!({"action":"invite","name":"Second workspace","email":email}),
        Some(auth),
    )
    .await;
    assert_eq!(s, 200, "{next}");
    let (s, _) = call(
        http.clone(),
        tenant,
        "POST",
        "/onboarding",
        json!({"action":"accept","token":next["token"],"password":"wrong-password-1234"}),
        None,
    )
    .await;
    assert!(!(200..300).contains(&s));
    let (s, result) = call(
        http.clone(),
        tenant,
        "POST",
        "/onboarding",
        json!({"action":"accept","token":next["token"],"password":"test-only-password-123"}),
        None,
    )
    .await;
    assert_eq!(s, 200, "Existing account invitation: {result}");
    println!("Onboarding passed: new account, existing identity, single-use invitation, merchant-only invitation and cross-tenant rejection");
}
