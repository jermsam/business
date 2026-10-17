#[tokio::test]
#[ignore = "Requires business_dev identity migration"]
async fn hosted_identity_uniqueness_and_concurrency() {
    use crate::access::{documents, guard_write, pair};
    use typedb_driver::TransactionType;
    let (app, http) = build().await.unwrap();
    let state = app
        .get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb")
        .unwrap();
    assert_eq!(state.database, "business_dev");
    let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let old_email = format!("identity-{tag}@example.com");
    let new_email = format!("renamed-{tag}@example.com");
    let email_b = format!("owner-b-{tag}@example.com");
    let tenant = format!("identity-{tag}.example.com");
    let renamed = format!("renamed-{tag}.example.com");
    let pass = "test-only-password";
    let hash = bcrypt::hash(pass, 4).unwrap();
    adapter.write(json!({"query":format!(r#"insert
      $a isa user, has email "{old_email}", has password "{hash}";
      $b isa user, has email "{email_b}", has password "{hash}";
      $t isa company, has company_domain "{tenant}";
      (tenant:$t,member:$a) isa tenant_membership; (tenant:$t,member:$b) isa tenant_membership;"#)})).await.unwrap();
    prepare_fixture(&state).await;
    let (_, login) = call(
        http.clone(),
        &tenant,
        "POST",
        "/authentication",
        json!({"strategy":"local","email":old_email,"password":pass}),
        None,
    )
    .await;
    let token = login["accessToken"].as_str().unwrap();
    let uid = login["user"]["id"].as_str().unwrap();
    let tid = login["user"]["tenant_id"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(uid).is_ok());
    let (_, other) = call(
        http.clone(),
        tid,
        "POST",
        "/authentication",
        json!({"strategy":"local","email":email_b,"password":pass}),
        None,
    )
    .await;
    let bid = other["user"]["id"].as_str().unwrap();
    let btoken = other["accessToken"].as_str().unwrap();
    let (status, record) = call(
        http.clone(),
        tid,
        "POST",
        "/records",
        json!({"name":"Before rename"}),
        Some(token),
    )
    .await;
    assert_eq!(status, 200, "{record}");
    let path = format!("/records/{}", record["id"].as_str().unwrap());
    adapter.write(json!({"query":format!(r#"match $u isa user, has biz_id "{uid}"; $t isa company, has biz_id "{tid}";
        update $u has email "{new_email}"; update $t has company_domain "{renamed}";"#)})).await.unwrap();
    assert_eq!(
        call(http.clone(), tid, "GET", &path, Value::Null, Some(token))
            .await
            .0,
        200,
        "Token and ownership survive rename"
    );
    assert_eq!(
        call(
            http.clone(),
            &tenant,
            "GET",
            &path,
            Value::Null,
            Some(token)
        )
        .await
        .0,
        401,
        "Old alias no longer identifies tenant"
    );
    let (_, renamed_login) = call(
        http.clone(),
        tid,
        "POST",
        "/authentication",
        json!({"strategy":"local","email":new_email,"password":pass}),
        None,
    )
    .await;
    assert_eq!(renamed_login["user"]["id"], uid);
    // Reusing the old email must never transfer historical record ownership.
    adapter.write(json!({"query":format!(r#"match $t isa company, has biz_id "{tid}";
      insert $u isa user, has email "{old_email}", has password "{hash}"; (tenant:$t,member:$u) isa tenant_membership;"#)})).await.unwrap();
    prepare_fixture(&state).await;
    let (_, reuse) = call(
        http.clone(),
        tid,
        "POST",
        "/authentication",
        json!({"strategy":"local","email":old_email,"password":pass}),
        None,
    )
    .await;
    let reused_id = reuse["user"]["id"].as_str().unwrap();
    let reused_token = reuse["accessToken"].as_str().unwrap();
    assert_ne!(reused_id, uid);
    assert_eq!(
        call(
            http.clone(),
            tid,
            "GET",
            &path,
            Value::Null,
            Some(reused_token)
        )
        .await
        .0,
        404
    );
    // Trusted bootstrap establishes two owners. Public requests cannot self-promote.
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/memberships/{reused_id}"),
            json!({"role":"owner","state":"active"}),
            Some(reused_token)
        )
        .await
        .0,
        404
    );
    for person in [uid, bid] {
        adapter.write(json!({"query":format!(r#"match $m isa biz_membership, has biz_id "{}"; update $m has biz_role "owner";"#,pair(tid,person).unwrap())})).await.unwrap();
    }
    assert_eq!(
        call(
            http.clone(),
            tid,
            "GET",
            "/memberships",
            Value::Null,
            Some(token)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "GET",
            &format!("/memberships/{bid}"),
            Value::Null,
            Some(token)
        )
        .await
        .0,
        200
    );
    // Two already-open snapshots attempt to remove different owners.
    let tx1 = state
        .driver
        .transaction(&state.database, TransactionType::Write)
        .await
        .unwrap();
    let tx2 = state
        .driver
        .transaction(&state.database, TransactionType::Write)
        .await
        .unwrap();
    for (tx, person) in [(&tx1, uid), (&tx2, bid)] {
        let q = guard_write(&format!(
            r#"match $t isa company, has biz_id "{tid}", has biz_revision $revision;
        $person isa user, has biz_id "{person}";
        let $other = biz_other_owner($person,$t,{}); $other == true;
        $m isa biz_membership, has biz_id "{}";
        update $m has biz_state "suspended";
        fetch {{ "id": $m.biz_id }};"#,
            crate::auth::local::now(),
            pair(tid, person).unwrap()
        ))
        .unwrap();
        assert_eq!(documents(tx, &q).await.unwrap().len(), 1);
    }
    let (one, two) = tokio::join!(tx1.commit(), tx2.commit());
    assert_ne!(
        one.is_ok(),
        two.is_ok(),
        "Exactly one owner removal may commit"
    );
    let (survivor, survivor_token) = if one.is_ok() {
        (bid, btoken)
    } else {
        (uid, token)
    };
    let (status, _) = call(
        http.clone(),
        tid,
        "PATCH",
        &format!("/memberships/{survivor}"),
        json!({"role":"member","state":"suspended"}),
        Some(survivor_token),
    )
    .await;
    assert_eq!(status, 404, "Last owner must remain");
    // A revoked member cannot be resurrected by rerunning the legacy backfill.
    let removed = if one.is_ok() { uid } else { bid };
    adapter.write(json!({"query":format!(r#"match $m isa biz_membership, has biz_id "{}"; delete $m;"#,pair(tid,removed).unwrap())})).await.unwrap();
    prepare_fixture(&state).await;
    let out = adapter.read(json!({"query":format!(r#"match $m isa biz_membership, has biz_id "{}"; fetch {{"id":$m.biz_id}};"#,pair(tid,removed).unwrap())})).await.unwrap();
    assert!(out["ok"]["answers"].as_array().unwrap().is_empty());
    assert_eq!(
        call(
            http.clone(),
            tid,
            "POST",
            "/memberships",
            json!({"person_id":removed,"role":"member"}),
            Some(survivor_token)
        )
        .await
        .0,
        200,
        "Explicit owner action may re-enrol a removed member"
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/memberships/{removed}"),
            json!({"role":"member","state":"suspended"}),
            Some(survivor_token)
        )
        .await
        .0,
        200
    );
    // Concurrent insertion of one canonical membership cannot produce two rows.
    let new_id = uuid::Uuid::new_v4().to_string();
    adapter.write(json!({"query":format!(r#"insert $u isa user, has biz_id "{new_id}", has email "new-{tag}@example.com", has password "{hash}";"#)})).await.unwrap();
    let tx1 = state
        .driver
        .transaction(&state.database, TransactionType::Write)
        .await
        .unwrap();
    let tx2 = state
        .driver
        .transaction(&state.database, TransactionType::Write)
        .await
        .unwrap();
    let key = pair(tid, &new_id).unwrap();
    let insert = format!(
        r#"match $t isa company, has biz_id "{tid}"; $u isa user, has biz_id "{new_id}";
      insert $m isa biz_membership, links (tenant:$t,person:$u), has biz_id "{key}", has biz_state "active", has biz_role "member", has biz_start 2020-01-01T00:00:00; fetch {{ "id":$m.biz_id }};"#
    );
    documents(&tx1, &insert).await.unwrap();
    documents(&tx2, &insert).await.unwrap();
    let (one, two) = tokio::join!(tx1.commit(), tx2.commit());
    assert_ne!(
        one.is_ok(),
        two.is_ok(),
        "Key arbitration must reject the duplicate"
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "POST",
            "/memberships",
            json!({"person_id":new_id,"role":"member"}),
            Some(survivor_token)
        )
        .await
        .0,
        404,
        "Repeated creation must not change existing membership"
    );
    // Malformed alternate IDs are rejected by the policy even with active state.
    adapter.write(json!({"query":format!(r#"match $t isa company, has biz_id "{tid}"; $u isa user, has biz_id "{new_id}";
      $m isa biz_membership, has biz_id "{key}"; update $m has biz_state "suspended";
      insert (tenant:$t,person:$u) isa biz_membership, has biz_id "invalid-{tag}", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#)})).await.unwrap();
    assert_eq!(
        call(
            http.clone(),
            tid,
            "POST",
            "/authentication",
            json!({"strategy":"local","email":format!("new-{tag}@example.com"),"password":pass}),
            None
        )
        .await
        .0,
        401
    );
    // Keep future migration tests unambiguous; remove only our deliberate malformed fixture.
    adapter.write(json!({"query":format!(r#"match $m isa biz_membership, has biz_id "invalid-{tag}"; delete $m;"#)})).await.unwrap();
    // A committed grant revocation invalidates an overlapping authorized write.
    let product_id = uuid::Uuid::new_v4().to_string();
    let project_id = uuid::Uuid::new_v4().to_string();
    let grant_id = uuid::Uuid::new_v4().to_string();
    let entitlement_id = uuid::Uuid::new_v4().to_string();
    let record_id = uuid::Uuid::new_v4().to_string();
    adapter.write(json!({"query":format!(r#"match
      $t isa company, has biz_id "{tid}"; $u isa user, has biz_id "{survivor}";
      insert $p isa biz_product, has biz_id "{product_id}", has name "Concurrency";
      $project isa biz_project, has biz_id "{project_id}", has name "Race";
      (tenant:$t,product:$p,project:$project) isa biz_project_owner;
      (tenant:$t,product:$p) isa biz_entitlement, has biz_id "{entitlement_id}", has biz_state "active", has biz_start 2020-01-01T00:00:00;
      (tenant:$t,grantee:$u,project:$project,grantor:$u) isa biz_grant, has biz_id "{grant_id}", has action "update", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;
      $r isa biz_record, has biz_id "{record_id}", has name "Original";
      (record:$r,project:$project,creator:$u) isa biz_record_owner;"#)})).await.unwrap();
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/access-grants/{grant_id}"),
            json!({"state":"suspended"}),
            Some(reused_token)
        )
        .await
        .0,
        404
    );
    let pending = state
        .driver
        .transaction(&state.database, TransactionType::Write)
        .await
        .unwrap();
    let mutation = guard_write(&format!(
        r#"match
      $t isa company, has biz_id "{tid}", has biz_revision $revision;
      $u isa user, has biz_id "{survivor}"; $project isa biz_project, has biz_id "{project_id}";
      let $ok = biz_authorized($u,$t,$project,"update",{}); $ok == true;
      $r isa biz_record, has biz_id "{record_id}";
      update $r has name "Stale write"; fetch {{ "id": $r.biz_id }};"#,
        crate::auth::local::now()
    ))
    .unwrap();
    assert_eq!(documents(&pending, &mutation).await.unwrap().len(), 1);
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/access-grants/{grant_id}"),
            json!({"state":"suspended"}),
            Some(survivor_token)
        )
        .await
        .0,
        200
    );
    assert!(
        pending.commit().await.is_err(),
        "Stale authorized write must conflict after revocation"
    );
    let unchanged=adapter.read(json!({"query":format!(r#"match $r isa biz_record, has biz_id "{record_id}"; fetch {{"name":$r.name}};"#)})).await.unwrap();
    assert_eq!(unchanged["ok"]["answers"][0]["data"]["name"], "Original");
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/workspace-records/{record_id}"),
            json!({"name":"Denied"}),
            Some(survivor_token)
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/access-grants/{grant_id}"),
            json!({"state":"active"}),
            Some(survivor_token)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/workspace-records/{record_id}"),
            json!({"name":"Fresh write"}),
            Some(survivor_token)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/entitlements/{entitlement_id}"),
            json!({"state":"suspended"}),
            Some(survivor_token)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/workspace-records/{record_id}"),
            json!({"name":"Denied"}),
            Some(survivor_token)
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            http.clone(),
            tid,
            "PATCH",
            &format!("/entitlements/{entitlement_id}"),
            json!({"state":"active"}),
            Some(survivor_token)
        )
        .await
        .0,
        403,
        "Owners cannot restore a purchased entitlement without trusted provisioning"
    );
}
