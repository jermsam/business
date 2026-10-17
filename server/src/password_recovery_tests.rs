#[tokio::test]
#[ignore = "Requires Cloud business_dev recovery migration; synthetic identities only"]
async fn hosted_password_recovery() {
 use crate::services::recovery::RecoveryService;
 let (app,http)=build().await.unwrap();let state=app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();assert_eq!(state.database,"business_dev");
 let adapter=dog_typedb::TypeDBAdapter::new(state.clone());let tag=uuid::Uuid::new_v4().simple().to_string();let email=format!("recovery-{tag}@example.com");let domain=format!("recovery-{tag}.example.com");
 let old="old-recovery-password";let new="new-recovery-password";let hash=bcrypt::hash(old,4).unwrap();
 adapter.write(json!({"query":format!(r#"insert $u isa user,has email "{email}",has password "{hash}";$t isa company,has company_domain "{domain}";(tenant:$t,member:$u) isa tenant_membership;"#)})).await.unwrap();prepare_fixture(&state).await;
 let (s,login)=call(http.clone(),&domain,"POST","/authentication",json!({"strategy":"local","email":email,"password":old}),None).await;assert_eq!(s,200);let old_token=login["accessToken"].as_str().unwrap();let user=login["user"]["id"].as_str().unwrap();
 let recovery=RecoveryService::new(state.clone());
 for address in [&email,&format!("unknown-{tag}@example.com")] {let (s,v)=call(http.clone(),&domain,"POST","/recovery",json!({"action":"request","email":address}),None).await;assert_eq!(s,200);assert_eq!(v["requested"],true);assert!(v.get("token").is_none());}
 recovery.request(&email).await.unwrap();
 let rows=adapter.read(json!({"query":format!(r#"match $r isa portal_recovery,has email "{email}";fetch {{"id":$r.biz_id}};"#)})).await.unwrap();let answers=rows["ok"]["answers"].as_array().unwrap();assert_eq!(answers.len(),1,"Per-account request cooldown is durable");let id=answers[0]["data"]["id"].as_str().unwrap();
 recovery.claim_link(id,user,"initial").await.unwrap();
 let token=crate::services::recovery::recovery_shared::token(id,"initial").unwrap();
 let (s,_)=call(http.clone(),&domain,"POST","/recovery",json!({"action":"reset","token":token,"password":new,"confirm_password":"different-password"}),None).await;assert!(s>=400);
 let second=uuid::Uuid::new_v4().to_string();let expires=chrono::Utc::now().timestamp()+3600;
 adapter.write(json!({"query":format!(r#"insert $r isa portal_recovery,has biz_id "{second}",has auth_request_key "{second}",has email "{email}",has portal_expires {expires},has portal_used false,has auth_status "queued";"#)})).await.unwrap();
 recovery.claim_link(&second,user,"initial").await.unwrap();let second_token=crate::services::recovery::recovery_shared::token(&second,"initial").unwrap();
 let payload=json!({"action":"reset","token":token,"password":new,"confirm_password":new});
 let (a,b)=tokio::join!(call(http.clone(),&domain,"POST","/recovery",payload.clone(),None),call(http.clone(),&domain,"POST","/recovery",payload.clone(),None));
 assert_eq!(usize::from(a.0==200)+usize::from(b.0==200),1,"Exactly one concurrent reset can consume a token: {} {}",a.0,b.0);
 let (s,_)=call(http.clone(),&domain,"POST","/recovery",payload,None).await;assert!(s>=400);
 let (s,_)=call(http.clone(),&domain,"POST","/recovery",json!({"action":"reset","token":second_token,"password":new,"confirm_password":new}),None).await;assert_eq!(s,401,"Reset invalidates every older recovery link");
 let (s,_)=call(http.clone(),&domain,"GET","/records",Value::Null,Some(old_token)).await;assert_eq!(s,401,"Old session invalidated");
 let (s,_)=call(http.clone(),&domain,"POST","/authentication",json!({"strategy":"local","email":email,"password":old}),None).await;assert_eq!(s,401);
 let (s,fresh)=call(http.clone(),&domain,"POST","/authentication",json!({"strategy":"local","email":email,"password":new}),None).await;assert_eq!(s,200,"{fresh}");
 let (s,_)=call(http,&domain,"GET","/records",Value::Null,fresh["accessToken"].as_str()).await;assert_eq!(s,200);
 println!("Recovery passed: generic response, durable cooldown, atomic consumption, concurrent reset, session invalidation, old password rejection, fresh login");
}
