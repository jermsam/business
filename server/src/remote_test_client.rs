// Optional public HTTPS transport for the existing hosted acceptance suites.
// Request bodies and bearer tokens travel through stdin, never process arguments.
async fn remote_call(base: &str, tenant: &str, method: &str, path: &str, data: Value, token: Option<&str>) -> (u16, Value) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    assert!(base.starts_with("https://") && !base.contains(['\n','\r','?','#','@']));
    assert!(path.starts_with('/') && !path.contains(['\n','\r']));
    let quoted = |s: &str| serde_json::to_string(s).unwrap();
    let mut config = format!("url = {}\nrequest = {}\nheader = {}\nheader = {}\ndata-binary = {}\n", quoted(&format!("{}{path}",base.trim_end_matches('/'))),quoted(method),quoted("content-type: application/json"),quoted(&format!("x-tenant-id: {tenant}")),quoted(&data.to_string()));
    if let Some(token)=token {config += &format!("header = {}\n",quoted(&format!("authorization: Bearer {token}")));}
    if path=="/subjects" {config += "header = \"x-service-method: read\"\n";}
    tokio::task::spawn_blocking(move || {
        let mut child=Command::new("curl").args(["--silent","--show-error","--proto","=https","--max-time","90","--connect-timeout","15","--write-out","\n%{http_code}","--config","-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("curl is required for public HTTPS validation");
        child.stdin.take().unwrap().write_all(config.as_bytes()).unwrap();
        let output=child.wait_with_output().unwrap();
        assert!(output.status.success(),"Public HTTPS request failed (curl status {})",output.status);
        let body=String::from_utf8(output.stdout).unwrap();
        let (body,status)=body.rsplit_once('\n').unwrap();
        (status.parse().unwrap(),serde_json::from_str(body).unwrap_or(Value::Null))
    }).await.unwrap()
}
