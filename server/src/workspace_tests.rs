// Included inside live_tests so every assertion exercises the real public router.
#[tokio::test]
#[ignore = "Requires business_dev with migrate-access applied"]
async fn hosted_workspace_policy() {
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
    let project = uuid::Uuid::new_v4().to_string();
    let foreign = uuid::Uuid::new_v4().to_string();
    let product = uuid::Uuid::new_v4().to_string();
    let password = "test-only-correct-password";
    let hash = bcrypt::hash(password, 4).unwrap();
    let mut seed = format!(
        r#"insert
      $a isa company, has company_domain "{a}"; $b isa company, has company_domain "{b}";
      $alice isa user, has email "{alice}", has password "{hash}";
      $bob isa user, has email "{bob}", has password "{hash}";
      (tenant: $a, member: $alice) isa tenant_membership;
      (tenant: $a, member: $bob) isa tenant_membership;
      (tenant: $b, member: $alice) isa tenant_membership;
      $p isa biz_product, has biz_id "{product}", has name "SEYFR";
      $project isa biz_project, has biz_id "{project}", has name "Shared";
      $foreign isa biz_project, has biz_id "{foreign}", has name "Other tenant";
      (tenant: $a, product: $p, project: $project) isa biz_project_owner;
      (tenant: $b, product: $p, project: $foreign) isa biz_project_owner;
      $team isa biz_team, has biz_id "team-{tag}", has name "Editors";
      (tenant: $a, team: $team) isa biz_team_owner;
      (team: $team, person: $bob) isa biz_team_member, has biz_id "team-member-{tag}", has biz_state "active", has biz_start 2020-01-01T00:00:00;
    "#
    );
    for (tenant, person, key) in [
        ("a", "alice", "aa"),
        ("a", "bob", "ab"),
        ("b", "alice", "ba"),
    ] {
        seed += &format!(
            r#"(tenant: ${tenant}, person: ${person}) isa biz_membership, has biz_id "{key}-{tag}", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#
        );
    }
    for tenant in ["a", "b"] {
        seed += &format!(
            r#"(tenant: ${tenant}, product: $p) isa biz_entitlement, has biz_id "ent-{tenant}-{tag}", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#
        );
    }
    for action in ["read", "create", "update", "delete"] {
        seed += &format!(
            r#"(tenant: $a, grantee: $alice, project: $project, grantor: $alice) isa biz_grant,
          has biz_id "alice-{action}-{tag}", has action "{action}", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#
        );
    }
    seed += &format!(
        r#"(tenant: $a, grantee: $team, project: $project, grantor: $alice) isa biz_grant,
      has biz_id "team-read-{tag}", has action "read", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#
    );
    adapter.write(json!({"query":seed})).await.unwrap();
    prepare_fixture(&state).await;
    let mut tokens = Vec::new();
    let mut bob_membership = String::new();
    for email in [&alice, &bob] {
        let (s, v) = call(
            http.clone(),
            &a,
            "POST",
            "/authentication",
            json!({"strategy":"local", "email":email, "password":password}),
            None,
        )
        .await;
        assert_eq!(s, 200, "Login: {v}");
        if email == &bob {
            bob_membership = crate::access::pair(
                v["user"]["tenant_id"].as_str().unwrap(),
                v["user"]["id"].as_str().unwrap(),
            )
            .unwrap();
        }
        tokens.push(v["accessToken"].as_str().unwrap().to_owned());
    }
    let (at, bt) = (tokens[0].as_str(), tokens[1].as_str());
    for token in [at, bt] {
        let (s, v) = call(http.clone(), &a, "GET", "/apps", Value::Null, Some(token)).await;
        assert_eq!(s, 200, "Apps: {v}");
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["id"], product);
    }
    let (s, projects) = call(
        http.clone(),
        &a,
        "GET",
        &format!("/apps/{product}"),
        Value::Null,
        Some(bt),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(projects["projects"][0]["id"], project);
    assert_eq!(
        call(
            http.clone(),
            &b,
            "GET",
            &format!("/apps/{product}"),
            Value::Null,
            Some(at)
        )
        .await
        .0,
        404
    );
    // Entitlement alone does not grant every employee a product. A grant alone
    // cannot unlock a product the organization has not purchased.
    let impact = uuid::Uuid::new_v4().to_string();
    let quill = uuid::Uuid::new_v4().to_string();
    adapter.write(json!({"query":format!(r#"match
      $t isa company, has company_domain "{a}"; $alice isa user, has email "{alice}"; $bob isa user, has email "{bob}";
      insert
      $impact isa biz_product, has biz_id "{impact}", has name "Impact Reports";
      $quill isa biz_product, has biz_id "{quill}", has name "Quillspace";
      $ip isa biz_project, has biz_id "{impact}", has name "Reports";
      $qp isa biz_project, has biz_id "{quill}", has name "Writing";
      (tenant: $t, product: $impact, project: $ip) isa biz_project_owner;
      (tenant: $t, product: $quill, project: $qp) isa biz_project_owner;
      (tenant: $t, product: $impact) isa biz_entitlement, has biz_id "impact-ent-{tag}", has biz_state "active", has biz_start 2020-01-01T00:00:00;
      (tenant: $t, grantee: $alice, project: $ip, grantor: $alice) isa biz_grant,
      has biz_id "impact-grant-{tag}", has action "read", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;
      (tenant: $t, grantee: $bob, project: $qp, grantor: $alice) isa biz_grant,
      has biz_id "quill-grant-{tag}", has action "read", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#)})).await.unwrap();
    let apps_a = call(http.clone(), &a, "GET", "/apps", Value::Null, Some(at))
        .await
        .1;
    let apps_b = call(http.clone(), &a, "GET", "/apps", Value::Null, Some(bt))
        .await
        .1;
    assert_eq!(apps_a.as_array().unwrap().len(), 2);
    assert!(apps_a.as_array().unwrap().iter().any(|r| r["id"] == impact));
    assert_eq!(apps_b.as_array().unwrap().len(), 1);
    assert_eq!(apps_b[0]["id"], product);
    let (s, v) = call(
        http.clone(),
        &a,
        "POST",
        "/workspace-records",
        json!({"name":"Shared record", "project_id":project}),
        Some(at),
    )
    .await;
    assert_eq!(s, 200, "Create: {v}");
    let id = v["id"].as_str().unwrap();
    let path = format!("/workspace-records/{id}");
    assert_eq!(
        call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
            .await
            .0,
        200
    );
    for verb in ["PUT", "PATCH", "DELETE"] {
        assert_eq!(
            call(
                http.clone(),
                &a,
                verb,
                &path,
                json!({"name":"Forbidden"}),
                Some(bt)
            )
            .await
            .0,
            404,
            "Read-only {verb}"
        );
    }
    assert_eq!(
        call(
            http.clone(),
            &a,
            "POST",
            "/workspace-records",
            json!({"name":"Forbidden", "project_id":project}),
            Some(bt)
        )
        .await
        .0,
        404
    );
    for verb in ["GET", "PUT", "PATCH", "DELETE"] {
        assert_eq!(
            call(
                http.clone(),
                &b,
                verb,
                &path,
                json!({"name":"Forbidden"}),
                Some(at)
            )
            .await
            .0,
            404,
            "Cross tenant {verb}"
        );
    }
    assert_eq!(
        call(
            http.clone(),
            &a,
            "POST",
            "/workspace-records",
            json!({"name":"Forbidden", "project_id":foreign}),
            Some(at)
        )
        .await
        .0,
        404
    );
    assert!(
        call(http.clone(), &b, "GET", "/apps", Value::Null, Some(at))
            .await
            .1
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(http.clone(), &a, "GET", &path, Value::Null, None)
            .await
            .0,
        401
    );
    for key in ["tenant", "email", "owner", "query", "project_id"] {
        let mut data = json!({"name":"Forged"});
        data[key] = json!(foreign);
        assert_eq!(
            call(http.clone(), &a, "PATCH", &path, data, Some(at))
                .await
                .0,
            400
        );
    }
    let visible = call(
        http.clone(),
        &a,
        "GET",
        "/workspace-records",
        Value::Null,
        Some(bt),
    )
    .await
    .1;
    assert_eq!(visible.as_array().unwrap().len(), 1);
    assert_eq!(visible[0]["id"], id);
    assert!(call(
        http.clone(),
        &b,
        "GET",
        "/workspace-records",
        Value::Null,
        Some(at)
    )
    .await
    .1
    .as_array()
    .unwrap()
    .is_empty());
    // Internal callers cannot forge authentication; service boundaries match HTTP.
    let forged = crate::BusinessParams {
        authenticated: true,
        auth_result: Some(json!({"user":{"email":alice}})),
        ..Default::default()
    };
    let ctx = dog_core::tenant::TenantContext::new(a.clone());
    assert!(app
        .service("workspace-records")
        .unwrap()
        .find(ctx, forged)
        .await
        .is_err());
    // Malformed cross-tenant grants inserted directly must not authorize access.
    adapter.write(json!({"query":format!(r#"match
      $a isa company, has company_domain "{a}"; $b isa company, has company_domain "{b}";
      $u isa user, has email "{bob}"; $project isa biz_project, has biz_id "{project}";
      insert $team isa biz_team, has biz_id "foreign-team-{tag}", has name "Foreign";
      (tenant: $b, team: $team) isa biz_team_owner;
      (team: $team, person: $u) isa biz_team_member, has biz_id "foreign-member-{tag}", has biz_state "active", has biz_start 2020-01-01T00:00:00;
      (tenant: $a, grantee: $team, project: $project, grantor: $u) isa biz_grant,
      has biz_id "foreign-team-grant-{tag}", has action "create", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;
      (tenant: $b, grantee: $u, project: $project, grantor: $u) isa biz_grant,
      has biz_id "foreign-direct-{tag}", has action "create", has effect "allow", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#)})).await.unwrap();
    assert_eq!(
        call(
            http.clone(),
            &a,
            "POST",
            "/workspace-records",
            json!({"name":"Forbidden", "project_id":project}),
            Some(bt)
        )
        .await
        .0,
        404
    );
    // Existing positive grant cannot override an applicable deny.
    adapter.write(json!({"query":format!(r#"match
      $t isa company, has company_domain "{a}"; $u isa user, has email "{bob}";
      $project isa biz_project, has biz_id "{project}";
      insert (tenant: $t, grantee: $u, project: $project, grantor: $u) isa biz_grant,
      has biz_id "deny-{tag}", has action "read", has effect "deny", has biz_state "active", has biz_start 2020-01-01T00:00:00;"#)})).await.unwrap();
    assert_eq!(
        call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
            .await
            .0,
        404
    );
    adapter.write(json!({"query":format!(r#"match $g isa biz_grant, has biz_id "deny-{tag}"; delete $g;"#)})).await.unwrap();
    // Lifecycle changes apply to new protected operations, without issuing a new token.
    for (kind, key) in [
        ("biz_membership", bob_membership),
        ("biz_entitlement", format!("ent-a-{tag}")),
        ("biz_team_member", format!("team-member-{tag}")),
        ("biz_grant", format!("team-read-{tag}")),
    ] {
        for (attr, value) in [
            ("biz_state", "\"suspended\""),
            ("biz_end", "2020-01-01T00:00:00"),
            ("biz_start", "2099-01-01T00:00:00"),
        ] {
            adapter.write(json!({"query":format!(r#"match $x isa {kind}, has biz_id "{key}"; update $x has {attr} {value};"#)})).await.unwrap();
            assert_eq!(
                call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
                    .await
                    .0,
                if kind == "biz_membership" { 401 } else { 404 },
                "{kind} {attr}"
            );
            let restore = if attr == "biz_end" {
                format!(
                    r#"match $x isa {kind}, has biz_id "{key}", has biz_end $e; delete has $e of $x;"#
                )
            } else {
                let old = if attr == "biz_state" {
                    "\"active\""
                } else {
                    "2020-01-01T00:00:00"
                };
                format!(r#"match $x isa {kind}, has biz_id "{key}"; update $x has {attr} {old};"#)
            };
            adapter.write(json!({"query":restore})).await.unwrap();
            assert_eq!(
                call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
                    .await
                    .0,
                200
            );
        }
    }
    for (now, expected) in [
        ("2019-12-31T23:59:59", false),
        ("2020-01-01T00:00:00", true),
    ] {
        let answer = adapter
            .read(json!({"query":format!(r#"match
          $u isa user, has email "{bob}"; $t isa company, has company_domain "{a}";
          $p isa biz_project, has biz_id "{project}";
          let $ok = biz_authorized($u, $t, $p, "read", {now}); fetch {{ "allowed": $ok }};"#)}))
            .await
            .unwrap();
        assert_eq!(answer["ok"]["answers"][0]["data"]["allowed"], expected);
    }
    for (kind, attr, key, status) in [
        ("user", "email", bob.as_str(), 401),
        ("company", "company_domain", a.as_str(), 401),
        ("biz_product", "biz_id", product.as_str(), 404),
    ] {
        adapter.write(json!({"query":format!(r#"match $x isa {kind}, has {attr} "{key}"; update $x has biz_state "suspended";"#)})).await.unwrap();
        assert_eq!(
            call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
                .await
                .0,
            status,
            "Suspend {kind}"
        );
        adapter.write(json!({"query":format!(r#"match $x isa {kind}, has {attr} "{key}"; update $x has biz_state "active";"#)})).await.unwrap();
        assert_eq!(
            call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
                .await
                .0,
            200
        );
    }
    // All authorized write methods preserve the linked project and creator.
    for verb in ["PUT", "PATCH"] {
        assert_eq!(
            call(
                http.clone(),
                &a,
                verb,
                &path,
                json!({"name":"Allowed"}),
                Some(at)
            )
            .await
            .0,
            200
        );
    }
    assert_eq!(
        call(http.clone(), &a, "DELETE", &path, Value::Null, Some(at))
            .await
            .0,
        200
    );
    assert_eq!(
        call(http.clone(), &a, "GET", &path, Value::Null, Some(bt))
            .await
            .0,
        404
    );
    assert_eq!(
        call(
            http.clone(),
            &a,
            "DELETE",
            "/authentication",
            Value::Null,
            Some(at)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(http, &a, "GET", "/apps", Value::Null, Some(at))
            .await
            .0,
        401
    );
}
