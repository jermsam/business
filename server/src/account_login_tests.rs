#[tokio::test]
#[ignore = "Requires real business_dev TypeDB"]
async fn account_login_discovers_only_active_memberships() {
    let (_, http) = build().await.unwrap();
    let (app, _) = build().await.unwrap();
    let state=app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database,"business_dev");
    let adapter=dog_typedb::TypeDBAdapter::new(state.clone());
    let tag=uuid::Uuid::new_v4().simple().to_string();
    let email=format!("account-{tag}@example.com");
    let a=format!("account-a-{tag}.example.com");let b=format!("account-b-{tag}.example.com");let c=format!("account-c-{tag}.example.com");
    let password="test-only-account-password";let hash=bcrypt::hash(password,4).unwrap();
    adapter.write(json!({"query":format!(r#"insert $u isa user,has email "{email}",has password "{hash}";
      $a isa company,has company_domain "{a}"; $b isa company,has company_domain "{b}"; $c isa company,has company_domain "{c}";
      (tenant:$a,member:$u) isa tenant_membership; (tenant:$b,member:$u) isa tenant_membership;"#)})).await.unwrap();
    prepare_fixture(&state).await;
    let (bad, result)=call(http.clone(),"account","POST","/authentication",json!({"strategy":"account","email":email,"password":"wrong"}),None).await;
    assert_eq!(bad,401,"{result}"); assert!(result.get("user").is_none());
    let (status,login)=call(http.clone(),"account","POST","/authentication",json!({"strategy":"account","email":email,"password":password}),None).await;
    assert_eq!(status,200,"{login}");assert!(login["user"].get("password").is_none());
    let token=login["accessToken"].as_str().unwrap();let tenant=login["user"]["tenant_id"].as_str().unwrap();
    std::env::set_var("BILLING_MERCHANT_ID",tenant);
    let (status, profile)=call(http.clone(),tenant,"GET","/onboarding",Value::Null,Some(token)).await;
    assert_eq!(status,200,"{profile}");
    let companies=profile[0]["companies"].as_array().unwrap();assert_eq!(companies.len(),2,"{profile}");
    let domains:Vec<_>=companies.iter().map(|x|x["domain"].as_str().unwrap()).collect();assert!(domains.contains(&a.as_str())&&domains.contains(&b.as_str()));assert!(!domains.contains(&c.as_str()));
    assert_eq!(call(http.clone(),&c,"GET","/onboarding",Value::Null,Some(token)).await.0,401);
    adapter.write(json!({"query":format!(r#"match $u isa user,has email "{email}"; $t isa company,has company_domain "{b}"; $m isa biz_membership,links (tenant:$t,person:$u);delete $m;"#)})).await.unwrap();
    assert_eq!(call(http.clone(),&b,"GET","/onboarding",Value::Null,Some(token)).await.0,401);
    let (status,again)=call(http.clone(),"account","POST","/authentication",json!({"strategy":"account","email":email,"password":password}),None).await;
    assert_eq!(status,200,"{again}");let token2=again["accessToken"].as_str().unwrap();
    let (_,profile)=call(http.clone(),&a,"GET","/onboarding",Value::Null,Some(token2)).await;assert_eq!(profile[0]["companies"].as_array().unwrap().len(),1);
    assert_eq!(call(http.clone(),&a,"DELETE","/authentication",json!({}),Some(token2)).await.0,200);
    assert_eq!(call(http.clone(),&a,"GET","/onboarding",Value::Null,Some(token2)).await.0,401);
}
