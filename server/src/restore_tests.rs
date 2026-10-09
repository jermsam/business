#[tokio::test]
#[ignore = "business_dev export/restore drill; temporary database on existing cluster"]
async fn hosted_backup_restore_drill() {
    use crate::access::documents;
    use typedb_driver::TransactionType;
    let (app, _) = build().await.unwrap();
    let state=app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database,"business_dev");
    // tempfile directories are private; exports contain authentication data.
    let folder=tempfile::tempdir().unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder.path(),std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let schema_path=folder.path().join("schema.tql");
    let data_path=folder.path().join("data.typedb");
    let source=state.driver.databases().get(&state.database).await.unwrap();
    source.export_to_file(&schema_path,&data_path).await.unwrap();
    let schema=std::fs::read_to_string(&schema_path).unwrap();
    let name=format!("business_restore_{}",uuid::Uuid::new_v4().simple());
    assert!(!state.driver.databases().contains(&name).await.unwrap());
    let result=async {
        state.driver.databases().import_from_file(&name,schema,&data_path).await?;
        let original=state.driver.transaction(&state.database,TransactionType::Read).await?;
        let restored=state.driver.transaction(&name,TransactionType::Read).await?;
        let mut checked=0;
        for kind in ["user","company","biz_membership","biz_entitlement","biz_grant","biz_team","biz_project","biz_record"] {
            let query=format!("match $x isa {kind}, has biz_id $id; fetch {{\"id\":$id}};");
            let mut a:Vec<_>=documents(&original,&query).await?.into_iter().map(|v|v.to_string()).collect();
            let mut b:Vec<_>=documents(&restored,&query).await?.into_iter().map(|v|v.to_string()).collect();
            a.sort(); b.sort(); anyhow::ensure!(a==b,"Restored identity set differs for {kind}"); checked+=a.len();
        }
        for query in [
            r#"match $r isa portal_recovery; fetch {"id":$r.biz_id,"key":$r.auth_request_key,"hash":[$r.portal_token_hash],"version":[$r.auth_version],"status":$r.auth_status,"used":$r.portal_used,"expiry":$r.portal_expires};"#,
            r#"match (recovery:$r,person:$u) isa portal_recovery_owner; fetch {"id":$r.biz_id,"person":$u.biz_id};"#,
            r#"match $u isa user; fetch {"id":$u.biz_id,"version":[$u.auth_version]};"#,
            r#"match $i isa bill_invoice; fetch {"id":$i.bill_key,"history":[$i.bill_history]};"#,
            r#"match $c isa bill_customer; fetch {"id":$c.bill_key,"name":$c.name,"email":$c.bill_email,"policy":$c.bill_policy,"choice":$c.bill_choice,"provider":$c.bill_provider,"stripe":$c.bill_stripe_id,"mercury":$c.bill_mercury_id,"card":$c.bill_payment_method,"revision":$c.bill_revision,"consent":$c.bill_consent};"#,
            r#"match $p isa bill_plan; fetch {"id":$p.bill_key,"amount":$p.bill_amount,"anchor":$p.bill_anchor,"next":$p.bill_next,"sequence":$p.bill_sequence,"interval":$p.bill_interval,"days":$p.bill_due_days,"status":$p.bill_status};"#,
            r#"match $i isa bill_invoice; fetch {"id":$i.bill_key,"amount":$i.bill_amount,"provider":$i.bill_provider,"external":$i.bill_external_id,"revision":$i.bill_revision,"choice":$i.bill_choice,"status":$i.bill_status,"url":$i.bill_url,"due":$i.bill_next};"#,
            r#"match $n isa bill_notice; fetch {"id":$n.bill_key,"status":$n.bill_status};"#,
            r#"match (merchant:$m,buyer:$b,customer:$c) isa bill_account; fetch {"merchant":$m.biz_id,"buyer":$b.biz_id,"customer":$c.bill_key};"#,
            r#"match (customer:$c,plan:$p) isa bill_plan_owner; fetch {"customer":$c.bill_key,"plan":$p.bill_key};"#,
            r#"match (plan:$p,invoice:$i) isa bill_invoice_owner; fetch {"plan":$p.bill_key,"invoice":$i.bill_key};"#,
            r#"match (invoice:$i,notice:$n) isa bill_notice_owner; fetch {"invoice":$i.bill_key,"notice":$n.bill_key};"#,
            r#"match $i isa portal_invitation;(invitation:$i,tenant:$t) isa portal_invitation_owner; fetch {"id":$i.biz_id,"token_hash":$i.portal_token_hash,"used":$i.portal_used,"expiry":$i.portal_expires,"email":$i.email,"tenant":$t.biz_id};"#,
            r#"match $r isa auth_revocation; fetch {"key":$r.token_key};"#,
            r#"match $u isa user, has biz_id $id; fetch {"id":$id,"email":$u.email,"password":$u.password,"state":[$u.biz_state]};"#,
            r#"match $e isa biz_entitlement, links (tenant:$t,product:$p); fetch {"id":$e.biz_id,"tenant":$t.biz_id,"product":$p.biz_id,"state":$e.biz_state,"start":$e.biz_start,"end":[$e.biz_end]};"#,
            r#"match $g isa biz_grant, links (tenant:$t,grantee:$u,grantor:$v,project:$p); fetch {"id":$g.biz_id,"tenant":$t.biz_id,"grantee":$u.biz_id,"grantor":$v.biz_id,"project":$p.biz_id,"action":$g.action,"effect":$g.effect,"state":$g.biz_state,"start":$g.biz_start,"end":[$g.biz_end]};"#,
            r#"match (tenant:$t,product:$p,project:$r) isa biz_project_owner; fetch {"tenant":$t.biz_id,"product":$p.biz_id,"project":$r.biz_id};"#,
            r#"match (tenant:$t,team:$g) isa biz_team_owner; fetch {"tenant":$t.biz_id,"team":$g.biz_id};"#,
            r#"match $m isa biz_team_member, links (team:$t,person:$u); fetch {"id":$m.biz_id,"team":$t.biz_id,"person":$u.biz_id,"state":$m.biz_state,"start":$m.biz_start,"end":[$m.biz_end]};"#,
            r#"match (record:$r,project:$p,creator:$u) isa biz_record_owner; fetch {"record":$r.biz_id,"project":$p.biz_id,"creator":$u.biz_id};"#,
            r#"match $r isa business_record; fetch {"id":$r.record_id,"owner":$r.biz_person_id,"tenant":$r.biz_tenant_id,"name":$r.name};"#,
            r#"match $m isa biz_membership, links (tenant:$t,person:$u); let $ok=biz_private_authorized($u,$t,2026-10-08T00:00:00); fetch {"id":$m.biz_id,"role":$m.biz_role,"state":$m.biz_state,"allowed":$ok};"#,
        ] {
            let mut a:Vec<_>=documents(&original,query).await?.into_iter().map(|v|v.to_string()).collect();
            let mut b:Vec<_>=documents(&restored,query).await?.into_iter().map(|v|v.to_string()).collect();
            a.sort(); b.sort(); anyhow::ensure!(a==b,"Restored ownership/policy state differs"); checked+=a.len();
        }
        original.close().await?; restored.close().await?;
        Ok::<_,anyhow::Error>(checked)
    }.await;
    // Delete only the randomly named database this drill just created, even on failure.
    if state.driver.databases().contains(&name).await.unwrap() {
        state.driver.databases().get(&name).await.unwrap().delete().await.unwrap();
    }
    assert!(!state.driver.databases().contains(&name).await.unwrap());
    let checked=result.unwrap();
    println!("BACKUP_RESTORE checked_rows={checked} temporary_database_removed=true private_export_removed_on_exit=true");
}
