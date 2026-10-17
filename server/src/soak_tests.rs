#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Five-minute business_dev acceptance run; ten concurrent tenants"]
async fn hosted_business_soak() { run_business_soak(300,1000).await; }
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Thirty-second admission diagnostic, not an acceptance gate"]
async fn hosted_admission_diagnostics() { run_business_soak(30,0).await; }
async fn run_business_soak(seconds:u64, minimum_requests:usize) {
    use std::time::{Duration, Instant};
    let (app, http) = build().await.unwrap();
    let state = app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database,"business_dev");
    let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
    let tag = uuid::Uuid::new_v4();
    let hash = bcrypt::hash("soak-only-password",4).unwrap();
    let mut clients = Vec::new();
    for i in 0..10 {
        let uid=uuid::Uuid::new_v4(); let tid=uuid::Uuid::new_v4(); let pid=uuid::Uuid::new_v4();
        let product=uuid::Uuid::new_v4();
        let email=format!("soak-{tag}-{i}@example.com");
        let mut query=format!(r#"insert
          $u isa user, has biz_id "{uid}", has email "{email}", has password "{hash}";
          $t isa company, has biz_id "{tid}", has company_domain "soak-{tag}-{i}.example.com", has biz_revision "initial";
          (tenant:$t,person:$u) isa biz_membership, has biz_id "{tid}:{uid}", has biz_role "owner", has biz_state "active", has biz_start 2020-01-01T00:00:00;
          $p isa biz_product, has biz_id "{product}", has name "Soak product";
          $project isa biz_project, has biz_id "{pid}", has name "Soak project";
          (tenant:$t,product:$p,project:$project) isa biz_project_owner;
          (tenant:$t,product:$p) isa biz_entitlement, has biz_id "ent-{tag}-{i}", has biz_state "active", has biz_start 2020-01-01T00:00:00;
        "#);
        for action in ["read","create","update","delete"] {
            query+=&format!(r#"(tenant:$t,grantee:$u,grantor:$u,project:$project) isa biz_grant, has biz_id "{tag}-{i}-{action}", has action "{action}", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#);
        }
        adapter.write(json!({"query":query})).await.unwrap();
        let (status,login)=call(http.clone(),&tid.to_string(),"POST","/authentication",json!({"strategy":"local","email":email,"password":"soak-only-password"}),None).await;
        assert_eq!(status,200);
        clients.push((tid.to_string(),pid.to_string(),login["accessToken"].as_str().unwrap().to_owned()));
    }
    REMOTE_TIMINGS.lock().unwrap().clear();
    let start=Instant::now();
    let mut workers=Vec::new();
    for (i,(tenant,project,token)) in clients.iter().cloned().enumerate() {
        let http=http.clone();
        let foreign=clients[(i+1)%clients.len()].0.clone();
        workers.push(tokio::spawn(async move {
            let mut times=Vec::new(); let mut cycles=0;
            while start.elapsed()<Duration::from_secs(seconds) {
                let at=Instant::now();
                let (status,data)=call(http.clone(),&tenant,"POST","/workspace-records",json!({"name":"Soak record","project_id":project}),Some(&token)).await;
                times.push(at.elapsed().as_millis() as u64); assert_eq!(status,200,"create tenant {i}");
                let path=format!("/workspace-records/{}",data["id"].as_str().unwrap());
                for (method,body,expected) in [
                    ("GET",Value::Null,200),
                    ("PATCH",json!({"name":"Soak changed"}),200),
                    ("GET",Value::Null,200),
                    ("DELETE",Value::Null,200),
                    ("GET",Value::Null,404),
                ] {
                    let at=Instant::now();
                    let (status,result)=call(http.clone(),&tenant,method,&path,body,Some(&token)).await;
                    times.push(at.elapsed().as_millis() as u64); assert_eq!(status,expected,"{method} tenant {i}");
                    if method=="PATCH" { assert_eq!(result["name"],"Soak changed"); }
                }
                let at=Instant::now();
                let (status,_)=call(http.clone(),&foreign,"GET","/workspace-records",Value::Null,Some(&token)).await;
                times.push(at.elapsed().as_millis() as u64); assert_eq!(status,401,"cross tenant");
                cycles+=1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            (cycles,times)
        }));
    }
    let joined=futures::future::join_all(workers).await;
    {
        let timing=REMOTE_TIMINGS.lock().unwrap();
        if !timing.is_empty() {
            let mut queue:Vec<f64>=timing.iter().map(|t|t.0).collect();
            let mut service:Vec<f64>=timing.iter().map(|t|t.1).collect();
            queue.sort_by(f64::total_cmp); service.sort_by(f64::total_cmp);
            println!("ADMISSION_TIMING count={} queue_p95_ms={:.1} service_p95_ms={:.1}",timing.len(),queue[(queue.len()-1)*95/100],service[(service.len()-1)*95/100]);
        }
    }
    let results:Vec<_>=joined.into_iter().map(Result::unwrap).collect();
    let cycles:usize=results.iter().map(|r|r.0).sum();
    let mut latencies:Vec<u64>=results.into_iter().flat_map(|r|r.1).collect(); latencies.sort_unstable();
    let p95=latencies[(latencies.len()-1)*95/100]; let p99=latencies[(latencies.len()-1)*99/100];
    let report=json!({"tenants":10,"concurrent_users":10,"http_client":std::env::var("BUSINESS_TEST_HTTP_CLIENT").unwrap_or_else(|_|"reqwest-pooled".into()),"duration_seconds":start.elapsed().as_secs_f64(),"cycles":cycles,"requests":latencies.len(),"unexpected_statuses":0,"p95_ms":p95,"p99_ms":p99,"max_ms":latencies.last(),"database":"business_dev","transport":std::env::var("BUSINESS_TEST_BASE_URL").map(|base|format!("public HTTPS {base}; real TypeDB Cloud")).unwrap_or_else(|_|"in-process Axum router; real TypeDB Cloud".into()),"model":"closed-loop, one user per tenant, 100 ms pause per cycle"});
    println!("BUSINESS_SOAK {report}");
    // Provisional bounded launch gate, not the unrelated DogRS queue benchmark.
    assert!(latencies.len()>=minimum_requests,"Insufficient throughput for provisional baseline");
    assert!(p95<=5000 && p99<=10000,"Latency gate failed");
}
