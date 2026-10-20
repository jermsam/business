//! Executable-level acceptance: canonical tooling, startup, login, data and logout.
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    process::{Child, Command, Stdio},
    time::Duration,
};
use typedb_driver::TransactionType;
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn tool(command: &str, database: &str, owner: bool) -> Result<()> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_database"));
    child.arg(command).env("TYPEDB_DB", database);
    if owner {
        child
            .env("BUSINESS_OWNER_EMAIL", "owner@example.invalid")
            .env("BUSINESS_OWNER_NAME", "Owner")
            .env("BUSINESS_TENANT_NAME", "Acceptance")
            .env("BUSINESS_OWNER_PASSWORD", "acceptance-owner-password");
    }
    let output = child.output()?;
    ensure!(
        output.status.success(),
        "database {command} failed (exit {:?})",
        output.status.code()
    );
    Ok(())
}
#[tokio::test]
#[ignore = "Real TypeDB and actual database/server executables; generated disposable target and non-admin runtime credentials only"]
async fn canonical_tools_and_server_end_to_end() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == "business_dev",
        "Development credential source required"
    );
    let runtime: HashMap<String, String> =
        dotenvy::from_path_iter(std::env::var("BUSINESS_RUNTIME_ENV_FILE")?)?
            .collect::<std::result::Result<_, _>>()?;
    ensure!(
        runtime.get("TYPEDB_ADDR") == Some(&std::env::var("TYPEDB_ADDR")?),
        "Runtime credentials must target the same development cluster"
    );
    ensure!(
        runtime
            .get("TYPEDB_USERNAME")
            .is_some_and(|u| !u.is_empty() && u != "admin"),
        "Non-admin runtime credentials required"
    );
    let driver = dog_typedb::TypeDBDriverFactory::connect(
        &std::env::var("TYPEDB_ADDR")?,
        &std::env::var("TYPEDB_USERNAME")?,
        &std::env::var("TYPEDB_PASSWORD")?,
        true,
    )
    .await?;
    let database = format!("business_backend_test_{}", uuid::Uuid::new_v4().simple());
    tool("create", &database, false)?;
    let outcome=async{
        tool("verify",&database,false)?;
        ensure!(Command::new(env!("CARGO_BIN_EXE_database")).arg("create").env("TYPEDB_DB",&database).output()?.status.code()!=Some(0),"Create overwrote an existing target");
        tool("bootstrap",&database,true)?;
        ensure!(Command::new(env!("CARGO_BIN_EXE_database")).arg("bootstrap").env("TYPEDB_DB",&database)
            .env("BUSINESS_OWNER_EMAIL","duplicate@example.invalid").env("BUSINESS_OWNER_NAME","Duplicate")
            .env("BUSINESS_TENANT_NAME","Duplicate").env("BUSINESS_OWNER_PASSWORD","duplicate-test-password").output()?.status.code()!=Some(0),"Bootstrap ran twice");
        let tx=driver.transaction(&database,TransactionType::Read).await?;
        let rows=server::database::documents(&tx,"match $t isa tenant; $i isa person; fetch {'tenant':$t.tenant_id,'id':$i.identity_id};").await?;
        ensure!(rows.len()==1,"Bootstrap created ambiguous identities");
        let tenant=rows[0]["tenant"].as_str().unwrap();let identity=rows[0]["id"].as_str().unwrap();tx.close().await?;
        let port=std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let mut command=Command::new(env!("CARGO_BIN_EXE_server"));
        command.envs(&runtime).env("TYPEDB_DB",&database).env("HTTP_HOST","127.0.0.1").env("HTTP_PORT",port.to_string())
            .env("AUTH_JWT_SECRET","native-backend-acceptance-jwt-secret-more-than-32-bytes")
            .env("BUSINESS_GATEWAY_SECRET","acceptance-gateway-secret-more-than-32-bytes")
            .env_remove("MERCURY_CONNECTIONS_JSON").env_remove("STRIPE_CONNECTIONS_JSON")
            .stdout(Stdio::null()).stderr(Stdio::null());
        let mut process=Process(command.spawn()?);
        let client=reqwest::Client::builder().timeout(Duration::from_secs(60)).redirect(reqwest::redirect::Policy::none()).build()?;
        let base=format!("http://127.0.0.1:{port}");
        let mut healthy=false;
        for _ in 0..100{
            ensure!(process.0.try_wait()?.is_none(),"Canonical server exited at startup");
            if client.get(format!("{base}/health")).send().await.is_ok_and(|r|r.status().is_success()){healthy=true;break;}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }ensure!(healthy,"Canonical server did not become healthy");
        let gateway="acceptance-gateway-secret-more-than-32-bytes";
        ensure!(client.post(format!("{base}/authentication")).json(&json!({"strategy":"local","email":"owner@example.invalid","password":"acceptance-owner-password"})).send().await?.status()==401,"Gateway guard missing");
        let response=client.post(format!("{base}/authentication")).header("x-gateway-secret",gateway).header("x-tenant-id",tenant)
            .json(&json!({"strategy":"local","email":" OWNER@EXAMPLE.INVALID ","password":"acceptance-owner-password"})).send().await?;
        ensure!(response.status().is_success(),"Canonical owner login failed");
        let login:Value=response.json().await?;ensure!(login["user"]["id"]==identity&&login["user"].get("password").is_none(),"Invalid owner identity or password leak");
        let token=login["accessToken"].as_str().unwrap();
        let template=include_str!("../../public/queries/data/create-resource.tql");
        let query=template.replace("@@identity_id@@",&json!(identity).to_string()).replace("@@tenant_id@@",&json!(tenant).to_string()).replace("@@resource_id@@","\"tool-resource\"").replace("@@name@@","\"Tool resource\"").replace("@@revision@@","\"tool-revision\"");
        let write=client.post(format!("{base}/data")).header("x-gateway-secret",gateway).header("x-tenant-id",tenant).header("x-service-method","write").bearer_auth(token).json(&json!({"query":query})).send().await?;
        ensure!(write.status().is_success(),"Canonical resource write failed");
        let read=||client.post(format!("{base}/data")).header("x-gateway-secret",gateway).header("x-tenant-id",tenant).header("x-service-method","read").bearer_auth(token).json(&json!({"query":"match $r isa resource; fetch {'id':$r.resource_id};"}));
        let rows:Value=read().send().await?.error_for_status()?.json().await?;
        ensure!(rows["ok"]["answers"].as_array().is_some_and(|a|a.len()==1),"Canonical resource read failed");
        // No delete annotation is a deny-by-default decision, not an internal
        // server failure. Exercise the actual HTTP transport and verify no write.
        for query in [
            "match $r isa resource,has resource_id 'tool-resource'; delete $r;",
            "match $r isa resource,has resource_id 'tool-resource'; delete $r; fetch {'deleted':true};",
        ] {
            let denied=client.post(format!("{base}/data")).header("x-gateway-secret",gateway)
                .header("x-tenant-id",tenant).header("x-service-method","write")
                .bearer_auth(token).json(&json!({"query":query})).send().await?;
            ensure!(denied.status()==403,"Missing deletion policy did not return HTTP 403");
            let body:Value=denied.json().await?;
            ensure!(body["name"]=="Forbidden"&&body.get("source").is_none(),"Unsafe policy denial response");
            let retained:Value=read().send().await?.error_for_status()?.json().await?;
            ensure!(retained["ok"]["answers"].as_array().is_some_and(|a|a.len()==1),"Denied deletion changed persisted state");
        }
        let logged_out=client.delete(format!("{base}/authentication")).header("x-gateway-secret",gateway).header("x-tenant-id",tenant).bearer_auth(token).send().await?;
        ensure!(logged_out.status().is_success(),"Logout failed");
        ensure!(read().send().await?.status()==401,"Logged-out access token still works");
        Ok::<_,anyhow::Error>(())
    }.await;
    server::database::target(&database)?;
    driver.databases().get(&database).await?.delete().await?;
    outcome
}
