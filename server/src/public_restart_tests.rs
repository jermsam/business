#[tokio::test]
#[ignore = "Prepare private restart fixture against the approved public validation service"]
async fn public_restart_prepare() {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    assert_eq!(std::env::var("BUSINESS_TEST_BASE_URL").unwrap(), "https://jitpomi-business-validation.onrender.com");
    let (app,http)=build().await.unwrap();
    let state=app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database,"business_dev");
    let uid=uuid::Uuid::new_v4(); let tid=uuid::Uuid::new_v4().to_string();
    let email=format!("restart-{uid}@example.com"); let password=uuid::Uuid::new_v4().to_string();
    let hash=bcrypt::hash(&password,4).unwrap();
    let adapter=dog_typedb::TypeDBAdapter::new(state);
    adapter.write(json!({"query":format!(r#"insert $u isa user, has biz_id "{uid}", has email "{email}", has password "{hash}"; $t isa company, has biz_id "{tid}", has company_domain "restart-{uid}.example.com", has biz_revision "initial"; (tenant:$t,person:$u) isa biz_membership, has biz_id "{tid}:{uid}", has biz_role "owner", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#)})).await.unwrap();
    let mut tokens=Vec::new();
    for _ in 0..2 {
        let (status,login)=call(http.clone(),&tid,"POST","/authentication",json!({"strategy":"local","email":email,"password":password}),None).await;
        assert_eq!(status,200); tokens.push(login["accessToken"].as_str().unwrap().to_owned());
    }
    assert_ne!(tokens[0],tokens[1]);
    let (status,record)=call(http.clone(),&tid,"POST","/records",json!({"name":"Public restart persistence"}),Some(&tokens[0])).await;
    assert_eq!(status,200);
    let (status,_)=call(http,&tid,"DELETE","/authentication",Value::Null,Some(&tokens[1])).await; assert_eq!(status,200);
    let path=std::env::var("BUSINESS_RESTART_FIXTURE").unwrap();
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).unwrap();
    file.write_all(serde_json::to_string(&json!({"tenant":tid,"live_token":tokens[0],"revoked_token":tokens[1],"record":record["id"]})).unwrap().as_bytes()).unwrap();
    println!("Restart fixture prepared; tokens saved privately, not logged");
}
#[tokio::test]
#[ignore = "Verify previously prepared fixture after an actual Render restart"]
async fn public_restart_verify() {
    assert_eq!(std::env::var("BUSINESS_TEST_BASE_URL").unwrap(),"https://jitpomi-business-validation.onrender.com");
    let (_,http)=build().await.unwrap();
    let fixture:Value=serde_json::from_slice(&std::fs::read(std::env::var("BUSINESS_RESTART_FIXTURE").unwrap()).unwrap()).unwrap();
    let tenant=fixture["tenant"].as_str().unwrap(); let token=fixture["live_token"].as_str().unwrap();
    let path=format!("/records/{}",fixture["record"].as_str().unwrap());
    let (status,record)=call(http.clone(),tenant,"GET",&path,Value::Null,Some(token)).await;
    assert_eq!(status,200);assert_eq!(record["name"],"Public restart persistence");
    let (status,_)=call(http.clone(),tenant,"GET",&path,Value::Null,Some(fixture["revoked_token"].as_str().unwrap())).await;
    assert_eq!(status,401,"Revocation must survive web-process restart");
    let (status,_)=call(http.clone(),tenant,"DELETE",&path,Value::Null,Some(token)).await;assert_eq!(status,200);
    let (status,_)=call(http,tenant,"DELETE","/authentication",Value::Null,Some(token)).await;assert_eq!(status,200);
    println!("Public restart verified: persisted record, retained valid session, persisted logout");
}
