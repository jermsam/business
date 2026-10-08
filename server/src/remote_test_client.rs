static REMOTE_TIMINGS: std::sync::Mutex<Vec<(f64,f64)>> = std::sync::Mutex::new(Vec::new());
// Reuse an HTTPS connection pool as recommended by reqwest. Keep the original
// curl mode available for comparisons; neither mode retries failed mutations.
async fn remote_call(base: &str, tenant: &str, method: &str, path: &str, data: Value, token: Option<&str>) -> (u16, Value) {
    if std::env::var("BUSINESS_TEST_HTTP_CLIENT").as_deref()==Ok("curl") {
        return remote_call_curl(base,tenant,method,path,data,token).await;
    }
    static CLIENT:std::sync::LazyLock<reqwest::Client>=std::sync::LazyLock::new(||reqwest::Client::builder()
        .https_only(true).redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never()).timeout(std::time::Duration::from_secs(90))
        .connect_timeout(std::time::Duration::from_secs(15)).pool_max_idle_per_host(10)
        .build().expect("HTTPS test client"));
    assert!(base.starts_with("https://") && !base.contains(['\n','\r','?','#','@']));
    assert!(path.starts_with('/') && !path.contains(['\n','\r']));
    let mut request=CLIENT.request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(),format!("{}{path}",base.trim_end_matches('/')))
        .header("x-tenant-id",tenant).json(&data);
    if let Some(token)=token {request=request.bearer_auth(token);}
    if path=="/subjects" {request=request.header("x-service-method","read");}
    let response=request.send().await.expect("Public HTTPS request failed");
    let status=response.status().as_u16();
    if let Some(timing)=response.headers().get("server-timing").and_then(|v|v.to_str().ok()) {
        let mut parts=timing.split(", ").filter_map(|p|p.split_once(";dur=").and_then(|(_,v)|v.parse::<f64>().ok()));
        if let (Some(queue),Some(service))=(parts.next(),parts.next()) {REMOTE_TIMINGS.lock().unwrap().push((queue,service));}
    }
    let bytes=response.bytes().await.expect("Public HTTPS response body failed");
    (status,serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
// Optional public HTTPS transport for the existing hosted acceptance suites.
// Request bodies and bearer tokens travel through stdin, never process arguments.
async fn remote_call_curl(base: &str, tenant: &str, method: &str, path: &str, data: Value, token: Option<&str>) -> (u16, Value) {
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
