use super::*;
use anyhow::{ensure, Context, Result};
use futures::FutureExt;
use serde_json::{json, Value};
use std::sync::Arc;
use typedb_driver::TransactionType;

pub(super) fn render(name: &str, bindings: &Value, user: &Value) -> String {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("public/queries/data")
            .join(format!("{name}.tql")),
    )
    .expect("Frontend query fixture");
    regex::Regex::new(r"@@([a-z_]+)@@")
        .unwrap()
        .replace_all(&source, |c: &regex::Captures<'_>| match &c[1] {
            "identity_id" => user["id"].to_string(),
            "tenant_id" => user["tenant_id"].to_string(),
            "revision" => json!(uuid::Uuid::new_v4().to_string()).to_string(),
            "starts_at" => "2025-01-01T00:00:00".into(),
            "ends_at" => "2099-01-01T00:00:00".into(),
            key => bindings[key].to_string(),
        })
        .into_owned()
}
async fn call(
    client: &reqwest::Client,
    base: &str,
    tenant: &str,
    method: &str,
    token: &str,
    query: &str,
) -> Result<(u16, Value)> {
    let started = std::time::Instant::now();
    let response = client
        .post(format!("{base}/data"))
        .header("x-tenant-id", tenant)
        .header("x-service-method", method)
        .bearer_auth(token)
        .json(&json!({"query":query}))
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.json().await?;
    eprintln!(
        "Data HTTP {method}: status={status}, elapsed={:?}",
        started.elapsed()
    );
    Ok((status, body))
}
pub(super) async fn login(
    client: &reqwest::Client,
    base: &str,
    tenant: &str,
    email: &str,
) -> Result<Value> {
    Ok(client
        .post(format!("{base}/authentication"))
        .header("x-tenant-id", tenant)
        .json(&json!({"strategy":"local","email":email,"password":"rebuild-test-password"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}
fn answers(value: &Value) -> Result<&Vec<Value>> {
    ensure!(value.pointer("/ok/query").is_none(), "Query leaked");
    value
        .pointer("/ok/answers")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Bad data response: {value}"))
}

#[tokio::test]
#[ignore = "Real TypeDB, JWT and loopback HTTP; disposable database only"]
async fn schema_policy_real_http_migration() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == "business_dev",
        "Development credentials only"
    );
    let driver = Arc::new(
        dog_typedb::TypeDBDriverFactory::connect(
            &std::env::var("TYPEDB_ADDR")?,
            &std::env::var("TYPEDB_USERNAME")?,
            &std::env::var("TYPEDB_PASSWORD")?,
            true,
        )
        .await?,
    );
    let database = format!("business_backend_test_{}", uuid::Uuid::new_v4().simple());
    driver.databases().create(&database).await?;
    eprintln!("Disposable schema-policy database: {database}");
    let outcome = std::panic::AssertUnwindSafe(async {
        let tx=driver.transaction(&database,TransactionType::Schema).await?;
        tx.query(SCHEMA).await.context("base schema")?;
        tx.query(FUNCTIONS).await.context("domain functions")?;
        tx.query(SCHEMA_POLICY).await.context("schema policy batch")?;
        tx.query(BILLING).await.context("native billing model")?;
        tx.commit().await?;
        let state=Arc::new(server::typedb::TypeDBState{driver:driver.clone(),database:database.clone()});
        let trusted=dog_typedb::TypeDBAdapter::new(state.clone());
        trusted.write(json!({"query":include_str!("fixtures.tql").replace("@@password_hash@@", &bcrypt::hash("rebuild-test-password",4)?)})).await?;
        trusted.write(json!({"query":include_str!("billing-fixtures.tql")})).await?;
        let (app,http)=build_with_state(state,"schema-policy-test-secret-more-than-thirty-two-bytes".into())?;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base=format!("http://{}",listener.local_addr()?);
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server { fn drop(&mut self) { self.0.abort(); } }
        let _server=Server(tokio::spawn(async move {axum::serve(listener,router(http)).await.unwrap();}));
        let client=reqwest::Client::new();
        let alice=login(&client,&base,"tenant-a","alice@example.com").await?;
        let bob=login(&client,&base,"tenant-a","bob@example.com").await?;
        let carol=login(&client,&base,"tenant-a","carol@example.com").await?;
        let dave=login(&client,&base,"tenant-b","dave@example.com").await?;
        for user in [&alice,&bob,&carol,&dave] { ensure!(user.pointer("/user/password").is_none(), "Credential leaked"); }
        let a=alice["accessToken"].as_str().unwrap(); let b=bob["accessToken"].as_str().unwrap(); let c=carol["accessToken"].as_str().unwrap(); let d=dave["accessToken"].as_str().unwrap();
        // Arbitrary frontend query, absent from every asset: no query registry.
        let raw="match $r isa resource; fetch {'id':$r.resource_id};";
        let customers=include_str!("../../public/queries/data/billing-customers.tql");
        for (tenant,token,expected) in [("tenant-a",a,1),("tenant-b",d,1),("tenant-a",b,0),("tenant-a",c,0)] {
            let (status,body)=call(&client,&base,tenant,"read",token,customers).await?;
            ensure!(status==200 && answers(&body)?.len()==expected,"Native billing visibility: {body}");
        }
        for query in [
            "match $c isa bill_customer; fetch {'secret':$c.bill_stripe_id};",
            "match $c isa bill_customer,has bill_consent $secret; fetch {'secret':$secret};",
            "match $c isa bill_customer; update $c has bill_choice 'automatic'; fetch {'id':$c.bill_key};",
            "match $c isa bill_customer; delete $c;",
        ] {
            let (status,body)=call(&client,&base,"tenant-a","write",a,query).await?;
            ensure!(status>=400 || answers(&body)?.is_empty(),"Frontend forged provider state: {query}: {body}");
        }
        let plan="match $c isa bill_customer,has bill_key 'customer-a'; put $p isa bill_plan,has bill_key 'plan-a',has name 'Subscription',has bill_amount 1000,has bill_due_days 7,has bill_anchor 1800000000,has bill_next 1800000000,has bill_sequence 0,has bill_interval 'month',has bill_status 'active';(customer:$c,plan:$p) isa bill_plan_owner;fetch {'id':$p.bill_key};";
        ensure!(call(&client,&base,"tenant-b","write",d,plan).await?.0>=400,"Buyer created seller schedule");
        let (status,body)=call(&client,&base,"tenant-a","write",a,plan).await?;
        ensure!(status==200 && answers(&body)?.len()==1,"Native plan create: {body}");
        ensure!(call(&client,&base,"tenant-a","write",a,&plan.replace("'plan-a'","'invalid-money'").replace("bill_amount 1000","bill_amount 1")).await?.0>=400,"Invalid amount bypassed schema");
        let pause="match $p isa bill_plan,has bill_key 'plan-a';update $p has bill_status 'paused';fetch {'id':$p.bill_key};";
        ensure!(call(&client,&base,"tenant-a","write",a,pause).await?.0==200,"Seller could not pause plan");
        trusted.write(json!({"query":"match $c isa bill_customer; update $c has bill_consent 'initial';"})).await?;
        let (status,body)=call(&client,&base,"tenant-a","write",a,&pause.replace("'paused'","'active'")).await?;
        ensure!(status>=400 || answers(&body)?.is_empty(),"Schedule changed without invalidating consent: {body}");
        let stored=trusted.read(json!({"query":"match $p isa bill_plan,has bill_key 'plan-a';fetch {'status':$p.bill_status};"})).await?;
        ensure!(stored["ok"]["answers"][0]["data"]["status"]=="paused","Denied schedule update persisted");
        let invalidate="match $c isa bill_customer;update $c has bill_revision 'new-revision';fetch {'id':$c.bill_key};";
        ensure!(call(&client,&base,"tenant-a","write",a,invalidate).await?.0==200,"Consent invalidation failed");
        ensure!(call(&client,&base,"tenant-a","write",a,&pause.replace("'paused'","'active'")).await?.0==200,"Schedule failed after consent invalidation");
        // Removed legacy lifecycle services must not remain publicly reachable.
        // Recovery/onboarding acceptance belongs to their replacement when built,
        // not to this authentication/data/provider composition.
        for path in ["onboarding","recovery"] {
            let response=client.post(format!("{base}/{path}"))
                .header("x-tenant-id","tenant-a").bearer_auth(a)
                .json(&json!({"action":"request","email":"bob@example.com"})).send().await?;
            ensure!(response.status()==reqwest::StatusCode::NOT_FOUND,
                "Removed lifecycle endpoint is still exposed: {path}");
        }
        // Rotate credentials in the isolated fixture, then verify both public
        // login and per-request JWT invalidation. This is not a recovery test.
        let replacement_hash=bcrypt::hash("replacement-test-password",4)?;
        trusted.write(json!({"query":format!(
            "match $p isa person,has identity_id 'bob'; update $p has password_hash {},has credential_version 'test-rotation';",
            serde_json::to_string(&replacement_hash)?
        )})).await?;
        ensure!(call(&client,&base,"tenant-a","read",b,raw).await?.0>=400,
            "Credential rotation did not invalidate the previous session");
        let stale=client.post(format!("{base}/authentication"))
            .header("x-tenant-id","tenant-a")
            .json(&json!({"strategy":"local","email":"bob@example.com","password":"rebuild-test-password"}))
            .send().await?;
        ensure!(!stale.status().is_success(),"Old password still authenticates");
        let renewed:Value=client.post(format!("{base}/authentication"))
            .header("x-tenant-id","tenant-a")
            .json(&json!({"strategy":"local","email":"bob@example.com","password":"replacement-test-password"}))
            .send().await?.error_for_status()?.json().await?;
        let b=renewed["accessToken"].as_str().unwrap();
        let data=app.service("data")?;
        let mut internal=server::BusinessParams {
            authenticated: true,
            auth_result: Some(json!({"user":{"id":"alice","tenant_id":"tenant-a"}})),
            ..Default::default()
        };
        internal.inner.schema_policy_scope=Some(Arc::new(dog_typedb::isolation::AuthenticatedScope::new("tenant-a","alice")?));
        ensure!(data.custom(dog_core::TenantContext::new("tenant-a"),"read",Some(json!({"query":raw})),internal.clone()).await.is_err(),"Cached internal identity bypassed bearer authentication");
        internal.headers.insert("authorization".into(),format!("Bearer {b}"));
        let rows=data.custom(dog_core::TenantContext::new("tenant-a"),"read",Some(json!({"query":raw})),internal.clone()).await?;
        ensure!(answers(&rows)?.len()==1,"Internal call used forged cached admin scope");
        ensure!(data.custom(dog_core::TenantContext::new("tenant-b"),"read",Some(json!({"query":raw})),internal.clone()).await.is_err(),"Internal call crossed tenant membership");
        let (ar,br)=tokio::join!(call(&client,&base,"tenant-a","read",b,raw),call(&client,&base,"tenant-b","read",d,raw));
        let (status,rows)=ar?; ensure!(status==200,"Bob read: {rows}"); ensure!(answers(&rows)?.len()==1,"Bob isolation: {rows}");
        let (status,rows)=br?; ensure!(status==200,"Dave read: {rows}"); ensure!(answers(&rows)?.iter().all(|r|r["data"]["id"]!="files"),"Foreign data leaked");
        for query in [
            "match $p isa person; fetch {'hash':$p.password_hash};",
            "match $p isa person, has password_hash $hash; fetch {'hash':$hash};",
            "match $p isa auth_revocation; fetch {'key':$p.token_key};",
            "match let $seed=1; fetch {'now':request_now('tenant-b','dave',2000-01-01T00:00:00)};",
            "match $t isa tenant; fetch {'unguarded_objects':[request_unreviewed($t)]};",
        ] { ensure!(call(&client,&base,"tenant-a","read",b,query).await?.0>=400,"Unsafe query accepted"); }
        ensure!(call(&client,&base,"tenant-b","read",b,raw).await?.0>=400,"Forged tenant accepted");
        let mut bindings=json!({"resource_id":"preview-resource","team_id":"preview-team","policy_id":"preview-policy","revision_id":"preview-version","name":"Preview","description":"Preview policy","required_role":"member","permission":"read","effect":"allow","target_id":"bob","grant_id":"preview-grant","review_id":"preview-review","outcome":"certified","reason":"Explicit migration test"});
        for name in ["create-resource","create-team","create-policy","grant-policy","grant-exception"] {
            eprintln!("Schema-policy HTTP write: {name}");
            if name=="grant-exception" { bindings["grant_id"]=json!("preview-exception"); }
            let query=render(name,&bindings,&alice["user"]);
            let (status,value)=call(&client,&base,"tenant-a","write",a,&query).await?;
            ensure!(status==200,"{name} failed: {value}"); ensure!(answers(&value)?.len()==1,"{name} result: {value}");
        }
        bindings["grant_id"]=json!("preview-grant");
        let query=render("review-grant",&bindings,&carol["user"]);
        let (status,value)=call(&client,&base,"tenant-a","write",c,&query).await?;
        ensure!(status==200 && answers(&value)?.len()==1,"Review failed: {value}");
        // A successful operation can be intentionally repeated without another
        // review row or a new reviewed_at value. Never replay a failed write.
        let (status,repeated)=call(&client,&base,"tenant-a","write",c,&query).await?;
        ensure!(status==200 && answers(&repeated)?.len()==1,"Idempotent review failed: {repeated}");
        let reviews=trusted.read(json!({"query":"match $r isa access_review,has review_id 'preview-review';fetch {'id':$r.review_id};"})).await?;
        ensure!(reviews["ok"]["answers"].as_array().is_some_and(|r|r.len()==1),"Repeated review duplicated history");
        // Independent requests exercise the new-review path, not just the
        // already-existing row branch. Never retry a failed mutation.
        for index in 0..8 {
            bindings["review_id"]=json!(format!("preview-review-{index}"));
            let query=render("review-grant",&bindings,&carol["user"]);
            let (status,value)=call(&client,&base,"tenant-a","write",c,&query).await?;
            ensure!(status==200 && answers(&value)?.len()==1,"Fresh review {index} failed: {value}");
            ensure!(value["ok"]["answers"][0]["data"]["review_id"]==bindings["review_id"],"Wrong review returned");
        }
        let reviews=trusted.read(json!({"query":"match $r isa access_review;fetch {'id':$r.review_id};"})).await?;
        ensure!(reviews["ok"]["answers"].as_array().unwrap().len()==9,"Review history incomplete or duplicated");
        bindings["review_id"]=json!("preview-review");
        // A review by its own grantor must not create history or advance the
        // tenant revision, even though the grantor can otherwise manage access.
        bindings["review_id"]=json!("denied-self-review");
        let query=render("review-grant",&bindings,&alice["user"]);
        let before=trusted.read(json!({"query":"match $t isa tenant,has tenant_id 'tenant-a';fetch {'revision':$t.revision};"})).await?;
        let (status,value)=call(&client,&base,"tenant-a","write",a,&query).await?;
        ensure!(status>=400 || answers(&value)?.is_empty(),"Grantor certified their own grant");
        let history=trusted.read(json!({"query":"match $r isa access_review,has review_id 'denied-self-review';fetch {'id':$r.review_id};"})).await?;
        ensure!(history["ok"]["answers"].as_array().unwrap().is_empty(),"Denied review persisted");
        let after=trusted.read(json!({"query":"match $t isa tenant,has tenant_id 'tenant-a';fetch {'revision':$t.revision};"})).await?;
        ensure!(before["ok"]["answers"]==after["ok"]["answers"],"Denied review changed tenant revision");
        bindings["review_id"]=json!("preview-review");
        for name in ["resources","explain-access","missing-access","excess-access","unreviewed-grants"] {
            eprintln!("Schema-policy HTTP read: {name}");
            let query=render(name,&bindings,&alice["user"]);
            let (status,value)=call(&client,&base,"tenant-a","read",a,&query).await?;
            ensure!(status==200,"{name} failed: {value}"); answers(&value)?;
        }
        // A rejected later revision update must undo the graph creation.
        bindings["resource_id"]=json!("rollback-resource");
        let rollback=render("create-resource",&bindings,&alice["user"]).replace("update $t has revision", "update $t has name");
        ensure!(call(&client,&base,"tenant-a","write",a,&rollback).await?.0>=400,"Unsupported update accepted");
        let persisted=trusted.read(json!({"query":"match $r isa resource,has resource_id 'rollback-resource'; fetch {'id':$r.resource_id};"})).await?;
        ensure!(persisted["ok"]["answers"].as_array().unwrap().is_empty(),"Earlier graph write committed after failure");
        let denied_query=render("create-resource",&bindings,&bob["user"]);
        let (status,denied)=call(&client,&base,"tenant-a","write",b,&denied_query).await?;
        ensure!(status>=400 || answers(&denied)?.is_empty(),"Reader created resource");
        let bypass=denied_query.replace("let $admin=request_manage($t);$admin==true;", "");
        ensure!(call(&client,&base,"tenant-a","write",b,&bypass).await?.0>=400,"Omitting frontend permission predicate bypassed state policy");
        bindings["grant_id"]=json!("forged-grantor");
        let forged_grantor=render("grant-exception",&bindings,&alice["user"])
            .replace("has identity_id \"alice\"", "has identity_id \"carol\"")
            .replace("has resource_id \"rollback-resource\"", "has resource_id \"preview-resource\"");
        ensure!(call(&client,&base,"tenant-a","write",a,&forged_grantor).await?.0>=400,"Forged grantor accepted");
        let persisted=trusted.read(json!({"query":"match $g isa access_grant,has grant_id 'forged-grantor'; fetch {'id':$g.grant_id};"})).await?;
        ensure!(persisted["ok"]["answers"].as_array().unwrap().is_empty(),"Forged grant persisted");
        let orphan="put $r isa resource,has resource_id 'orphan-resource',has name 'Orphan',has status 'active'; fetch {'id':$r.resource_id};";
        ensure!(call(&client,&base,"tenant-a","write",a,orphan).await?.0>=400,"Orphan creation accepted");
        let forged=client.post(format!("{base}/data")).header("x-tenant-id","tenant-a").header("x-service-method","read").bearer_auth(b)
            .json(&json!({"query":raw,"actor":"alice","tenant":"tenant-b"})).send().await?;
        ensure!(!forged.status().is_success(),"Body scope spoof accepted");
        // Independent tenants must progress concurrently. Shared-key retries
        // either converge on the existing put or conflict atomically, never
        // duplicate the resource or its ownership relation.
        let qa=render("create-resource",&json!({"resource_id":"concurrent-a","name":"Concurrent A"}),&alice["user"]);
        let qb=render("create-resource",&json!({"resource_id":"concurrent-b","name":"Concurrent B"}),&dave["user"]);
        let (wa,wb)=tokio::join!(call(&client,&base,"tenant-a","write",a,&qa),call(&client,&base,"tenant-b","write",d,&qb));
        let (status,body)=wa?; ensure!(status==200,"Concurrent A: {body}");
        let (status,body)=wb?; ensure!(status==200,"Concurrent B: {body}");
        let qr=render("create-resource",&json!({"resource_id":"same-key","name":"Same key"}),&alice["user"]);
        let (one,two)=tokio::join!(call(&client,&base,"tenant-a","write",a,&qr),call(&client,&base,"tenant-a","write",a,&qr));
        let one=one?; let two=two?; ensure!(one.0==200 || two.0==200,"Both concurrent puts failed: {one:?} {two:?}");
        let persisted=trusted.read(json!({"query":"match $r isa resource,has resource_id 'same-key'; (tenant:$t,resource:$r) isa resource_ownership; fetch {'id':$r.resource_id,'tenant':$t.tenant_id};"})).await?;
        ensure!(persisted["ok"]["answers"].as_array().unwrap().len()==1,"Concurrent put duplicated ownership");
        let (status,foreign)=call(&client,&base,"tenant-b","read",d,"match $r isa resource,has resource_id 'concurrent-a'; fetch {'id':$r.resource_id};").await?;
        ensure!(status==200 && answers(&foreign)?.is_empty(),"Concurrent data crossed tenants");
        // JWT authentication still rechecks current credential versions.
        trusted.write(json!({"query":"match $p isa person,has identity_id 'bob'; update $p has credential_version 'rotated';"})).await?;
        ensure!(call(&client,&base,"tenant-a","read",b,raw).await?.0>=400,"Rotated token accepted");
        ensure!(data.custom(dog_core::TenantContext::new("tenant-a"),"read",Some(json!({"query":raw})),internal).await.is_err(),"Cached internal call accepted rotated credentials");
        trusted.write(json!({"query":"match $p isa person,has identity_id 'carol'; $m isa membership,links(member:$p); update $m has end_at 2000-01-01T00:00:00;"})).await?;
        ensure!(call(&client,&base,"tenant-a","read",c,raw).await?.0>=400,"Ended membership accepted");
        Ok::<(),anyhow::Error>(())
    }).catch_unwind().await;
    if let Ok(Err(error)) = &outcome {
        eprintln!("Migration test failed before cleanup: {error:#}");
    }
    let mut cleanup = driver.databases().get(&database).await?.delete().await;
    for _ in 0..5 {
        if cleanup.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        cleanup = driver.databases().get(&database).await?.delete().await;
    }
    cleanup?;
    match outcome {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
