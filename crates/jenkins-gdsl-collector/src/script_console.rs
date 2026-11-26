use catalog_validator::redact_sensitive_fields;
use reqwest::{
    blocking::{Client, Response},
    header::ACCEPT,
    redirect::Policy,
};
use serde_json::{json, Value};
use std::{
    io::Read,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SCRIPT_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_ITEMS_PER_REGISTRY: usize = 100;
const MAX_TEXT_LENGTH: usize = 256;
const TEMPLATE_VERSION: &str = "1";
const SCRIPT_TEMPLATE: &str = include_str!("probes/script_console_v1.groovy");

/// Describe every network operation in the deep profile without contacting Jenkins.
pub fn script_console_plan() -> Value {
    json!({
        "schemaVersion": "1.0.0",
        "profile": "deep",
        "confirmationRequired": true,
        "networkRequests": [
            {
                "method": "GET",
                "path": "/api/json?tree=_class,nodeName,jenkinsVersion",
                "requestCount": 1,
                "purpose": "bounded controller metadata",
                "connectTimeoutSeconds": CONNECT_TIMEOUT.as_secs(),
                "timeoutSeconds": REQUEST_TIMEOUT.as_secs(),
                "responseCapBytes": 1_048_576
            },
            {
                "method": "GET",
                "path": "/pluginManager/api/json?tree=plugins[shortName,version,active,enabled,dependencies[shortName,version,optional]]",
                "requestCount": 1,
                "purpose": "bounded installed plugin inventory",
                "connectTimeoutSeconds": CONNECT_TIMEOUT.as_secs(),
                "timeoutSeconds": REQUEST_TIMEOUT.as_secs(),
                "responseCapBytes": 4_194_304
            },
            {
                "method": "GET",
                "path": "/pipeline-syntax/gdsl",
                "headers": { "Range": "bytes=0-0" },
                "requestCount": 1,
                "purpose": "one-byte availability probe only",
                "connectTimeoutSeconds": CONNECT_TIMEOUT.as_secs(),
                "timeoutSeconds": REQUEST_TIMEOUT.as_secs(),
                "responseReadCapBytes": 1
            },
            {
                "method": "POST",
                "path": "/scriptText",
                "formFields": ["script"],
                "probeTemplate": "src/probes/script_console_v1.groovy",
                "templateVersion": TEMPLATE_VERSION,
                "requestCount": 1,
                "purpose": "read-only Groovy/JVM/Core metadata and selected extension registries",
                "connectTimeoutSeconds": CONNECT_TIMEOUT.as_secs(),
                "timeoutSeconds": REQUEST_TIMEOUT.as_secs(),
                "responseCapBytes": MAX_SCRIPT_RESPONSE_BYTES
            }
        ],
        "requestCount": 4,
        "totalTimeoutSeconds": REQUEST_TIMEOUT.as_secs() * 4,
        "scriptConsole": {
            "readOnly": true,
            "registries": ["pipeline-steps", "pipeline-globals"],
            "maxItemsPerRegistry": MAX_ITEMS_PER_REGISTRY,
            "maxTextLength": MAX_TEXT_LENGTH,
            "maxResponseBytes": MAX_SCRIPT_RESPONSE_BYTES,
            "clientTimeoutSeconds": REQUEST_TIMEOUT.as_secs(),
            "serverSideTerminationGuaranteed": false,
            "timeoutBehavior": "client stops waiting; Jenkins may continue executing the submitted script"
        },
        "callsMade": false
    })
}

/// Execute the repository-owned, versioned probe through Jenkins Script Console.
///
/// This function never accepts caller-supplied Groovy. Permission, compatibility,
/// timeout, and response failures are represented in the returned runtime record so
/// that the REST discovery snapshot can still be preserved.
pub fn collect_script_console(
    controller: &str,
    username: Option<&str>,
    api_token: Option<&str>,
) -> Value {
    if username.is_some() != api_token.is_some() {
        return unavailable("credentials-must-be-paired");
    }

    let endpoint = match script_console_endpoint(controller) {
        Ok(endpoint) => endpoint,
        Err(()) => return unavailable("invalid-controller-url"),
    };
    let client = match Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return unavailable("request-failed"),
    };

    collect_with_client(&endpoint, username, api_token, &client)
}

fn collect_with_client(
    endpoint: &Url,
    username: Option<&str>,
    api_token: Option<&str>,
    client: &Client,
) -> Value {
    let mut request = client
        .post(endpoint.clone())
        .header(ACCEPT, "application/json");
    if let (Some(user), Some(token)) = (username, api_token) {
        request = request.basic_auth(user, Some(token));
    }

    let response = match request.form(&[("script", SCRIPT_TEMPLATE)]).send() {
        Ok(response) => response,
        Err(error) => {
            return unavailable(if error.is_timeout() {
                "request-timeout"
            } else {
                "request-failed"
            });
        }
    };

    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return unavailable("permission-denied");
    }
    if status.as_u16() == 404 || status.as_u16() == 410 {
        return unavailable("endpoint-unavailable");
    }
    if !status.is_success() {
        return unavailable("http-error");
    }

    let bytes = match read_limited(response) {
        Ok(bytes) => bytes,
        Err(code) => return unavailable(code),
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return unavailable("invalid-response"),
    };
    normalize_probe_output(&value)
}

fn read_limited(response: Response) -> Result<Vec<u8>, &'static str> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SCRIPT_RESPONSE_BYTES as u64)
    {
        return Err("response-too-large");
    }

    let mut body = Vec::with_capacity(16 * 1024);
    response
        .take((MAX_SCRIPT_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                "request-timeout"
            } else {
                "request-failed"
            }
        })?;
    if body.len() > MAX_SCRIPT_RESPONSE_BYTES {
        return Err("response-too-large");
    }
    Ok(body)
}

fn normalize_probe_output(value: &Value) -> Value {
    if value.get("schemaVersion").and_then(Value::as_str) != Some("1.0.0")
        || value.get("templateVersion").and_then(Value::as_str) != Some(TEMPLATE_VERSION)
        || value
            .get("extensionRegistries")
            .and_then(Value::as_array)
            .is_none()
    {
        return unavailable("incompatible-probe-output");
    }

    let mut errors = Vec::new();
    let mut runtime = json!({
        "schemaVersion": "1.0.0",
        "templateVersion": TEMPLATE_VERSION,
        "source": "script-console",
        "status": "partial",
        "collectedAt": timestamp(),
        "extensionRegistries": [],
        "errors": []
    });

    let metadata_fields = [
        ("groovyVersion", "groovy-version"),
        ("javaVersion", "java-version"),
        ("javaVendor", "java-vendor"),
        ("jenkinsCoreVersion", "jenkins-core-version"),
    ];
    let mut metadata_count = 0;
    for (field, probe) in metadata_fields {
        if let Some(value) = value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            runtime[field] = json!(truncate(value));
            metadata_count += 1;
        } else {
            errors.push(json!({ "probe": probe, "code": "probe-failed" }));
        }
    }

    let raw_registries = value["extensionRegistries"]
        .as_array()
        .expect("registry array was checked");
    let mut available_registries = 0;
    let mut registry_results = Vec::with_capacity(2);
    for name in ["pipeline-steps", "pipeline-globals"] {
        let raw = raw_registries
            .iter()
            .find(|registry| registry.get("name").and_then(Value::as_str) == Some(name));
        let registry = normalize_registry(name, raw, &mut errors);
        if registry["status"] == "available" {
            available_registries += 1;
        }
        registry_results.push(registry);
    }
    runtime["extensionRegistries"] = json!(registry_results);

    if let Some(raw_errors) = value.get("errors").and_then(Value::as_array) {
        for error in raw_errors.iter().take(16) {
            let probe = error
                .get("probe")
                .and_then(Value::as_str)
                .filter(|probe| allowed_probe(probe));
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .filter(|code| allowed_error_code(code));
            if let (Some(probe), Some(code)) = (probe, code) {
                errors.push(json!({ "probe": probe, "code": code }));
            }
        }
    }

    let status = if metadata_count == metadata_fields.len()
        && available_registries == 2
        && errors.is_empty()
    {
        "collected"
    } else if metadata_count > 0 || available_registries > 0 {
        "partial"
    } else {
        "unavailable"
    };
    runtime["status"] = json!(status);
    runtime["errors"] = json!(errors);
    redact_sensitive_fields(&mut runtime);
    runtime
}

fn normalize_registry(
    name: &str,
    raw: Option<&Value>,
    errors: &mut Vec<Value>,
) -> Value {
    let Some(raw) = raw else {
        errors.push(json!({ "probe": name, "code": "probe-failed" }));
        return json!({
            "name": name,
            "status": "unavailable",
            "items": [],
            "truncated": false
        });
    };

    let status = match raw.get("status").and_then(Value::as_str) {
        Some("available") => "available",
        Some("unavailable") => "unavailable",
        _ => "unknown",
    };
    let mut items = Vec::new();
    let raw_items = raw.get("items").and_then(Value::as_array);
    if let Some(raw_items) = raw_items {
        for item in raw_items.iter().take(MAX_ITEMS_PER_REGISTRY) {
            let Some(item_name) = item
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            let Some(class_name) = item
                .get("className")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            let mut normalized = json!({
                "name": truncate(item_name),
                "className": truncate(class_name)
            });
            if let Some(plugin) = item
                .get("plugin")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                normalized["plugin"] = json!(truncate(plugin));
            }
            for field in ["displayName"] {
                if let Some(value) = item.get(field).and_then(Value::as_str).filter(|value| !value.trim().is_empty()) {
                    normalized[field] = json!(truncate(value));
                }
            }
            if let Some(parameters) = item.get("parameters").and_then(Value::as_array) {
                let normalized_parameters: Vec<Value> = parameters.iter().take(50).filter_map(|parameter| {
                    let name = parameter.get("name").and_then(Value::as_str).filter(|value| !value.trim().is_empty())?;
                    let mut result = json!({ "name": truncate(name) });
                    if let Some(value) = parameter.get("type").and_then(Value::as_str).filter(|value| !value.trim().is_empty()) {
                        result["type"] = json!(truncate(value));
                    }
                    if let Some(value) = parameter.get("required").and_then(Value::as_bool) {
                        result["required"] = json!(value);
                    }
                    if let Some(value) = parameter.get("description").and_then(Value::as_str).filter(|value| !value.trim().is_empty()) {
                        result["description"] = json!(truncate(value));
                    }
                    if let Some(value) = parameter.get("default").filter(|value| value.is_string() || value.is_number() || value.is_boolean()) {
                        result["default"] = value.clone();
                    }
                    Some(result)
                }).collect();
                normalized["parameters"] = json!(normalized_parameters);
            }
            items.push(normalized);
        }
    }
    let truncated = raw.get("truncated").and_then(Value::as_bool).unwrap_or(false)
        || raw_items.is_some_and(|items| items.len() > MAX_ITEMS_PER_REGISTRY);

    if status != "available" {
        let code = raw
            .get("errorCode")
            .and_then(Value::as_str)
            .filter(|code| allowed_error_code(code))
            .unwrap_or("probe-failed");
        errors.push(json!({ "probe": name, "code": code }));
    }

    json!({
        "name": name,
        "status": status,
        "items": items,
        "truncated": truncated
    })
}

fn allowed_probe(value: &str) -> bool {
    matches!(
        value,
        "groovy-version"
            | "java-version"
            | "java-vendor"
            | "jenkins-core-version"
            | "pipeline-steps"
            | "pipeline-globals"
            | "output"
    )
}

fn allowed_error_code(value: &str) -> bool {
    matches!(
        value,
        "probe-failed"
            | "jenkins-unavailable"
            | "extension-type-unavailable"
            | "enumeration-failed"
            | "output-truncated"
    )
}

fn truncate(value: &str) -> String {
    value.chars().take(MAX_TEXT_LENGTH).collect()
}

fn unavailable(code: &str) -> Value {
    let mut runtime = json!({
        "schemaVersion": "1.0.0",
        "templateVersion": TEMPLATE_VERSION,
        "source": "script-console",
        "status": "unavailable",
        "collectedAt": timestamp(),
        "extensionRegistries": [],
        "errors": [{ "probe": "script-console", "code": code }]
    });
    redact_sensitive_fields(&mut runtime);
    runtime
}

fn script_console_endpoint(controller: &str) -> Result<Url, ()> {
    let mut url = Url::parse(controller).map_err(|_| ())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.port().is_some_and(|port| port == 0)
    {
        return Err(());
    }
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url.path_segments_mut().map_err(|_| ())?;
        segments.pop_if_empty();
        segments.push("scriptText");
    }
    Ok(url)
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        remainder / 3600,
        (remainder % 3600) / 60,
        remainder % 60
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + if month <= 2 { 1 } else { 0 }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        thread,
    };

    fn test_client() -> Client {
        Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(2))
            .redirect(Policy::none())
            .build()
            .unwrap()
    }

    fn mock_server(status: &'static str, body: Vec<u8>) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let request = Arc::new(Mutex::new(String::new()));
        let recorded = Arc::clone(&request);
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let captured = read_request(&mut stream);
            *recorded.lock().unwrap() = captured;
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(&body);
        });
        (format!("http://{address}/jenkins"), request)
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut byte = [0_u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 32 * 1024 {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            bytes.push(byte[0]);
        }
        let header_end = bytes.len();
        let headers = String::from_utf8_lossy(&bytes);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let mut chunk = vec![0_u8; (header_end + content_length - bytes.len()).min(8192)];
            let Ok(count) = stream.read(&mut chunk) else {
                break;
            };
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn valid_probe_output() -> Value {
        json!({
            "schemaVersion": "1.0.0",
            "templateVersion": "1",
            "groovyVersion": "4.0.24",
            "javaVersion": "17.0.12",
            "javaVendor": "Eclipse Adoptium",
            "jenkinsCoreVersion": "2.479.1",
            "extensionRegistries": [
                {
                    "name": "pipeline-steps",
                    "status": "available",
                    "items": [{
                        "name": "sh",
                        "className": "org.jenkinsci.plugins.workflow.steps.durable_task.ShellStep",
                        "plugin": "workflow-durable-task-step"
                    }],
                    "truncated": false
                },
                {
                    "name": "pipeline-globals",
                    "status": "available",
                    "items": [{
                        "name": "env",
                        "className": "org.jenkinsci.plugins.workflow.cps.EnvActionImpl",
                        "plugin": "workflow-cps"
                    }],
                    "truncated": false
                }
            ],
            "errors": []
        })
    }

    #[test]
    fn posts_only_the_versioned_repository_probe_to_script_text() {
        let (controller, request) = mock_server(
            "200 OK",
            serde_json::to_vec(&valid_probe_output()).unwrap(),
        );
        let endpoint = script_console_endpoint(&format!("{controller}?token=must-not-escape#fragment"))
            .unwrap();
        let result = collect_with_client(&endpoint, Some("reader"), Some("api-token"), &test_client());
        let request = request.lock().unwrap().clone();
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let form = url::form_urlencoded::parse(body.as_bytes()).collect::<Vec<_>>();

        assert!(request.starts_with("POST /jenkins/scriptText HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("authorization: basic "));
        assert!(!request.contains("must-not-escape"));
        assert_eq!(form.len(), 1);
        assert_eq!(form[0].0, "script");
        assert_eq!(form[0].1, SCRIPT_TEMPLATE);
        assert_eq!(result["status"], "collected");
        assert_eq!(result["jenkinsCoreVersion"], "2.479.1");
    }

    #[test]
    fn normalizes_bounded_step_parameters_and_ignores_non_scalar_defaults() {
        let mut output = valid_probe_output();
        output["extensionRegistries"][0]["items"][0]["displayName"] = json!("Shell");
        output["extensionRegistries"][0]["items"][0]["parameters"] = json!([{
            "name": "script",
            "type": "java.lang.String",
            "required": true,
            "description": "Command to execute",
            "default": "echo ok",
            "unsafe": "must be ignored"
        },{
            "name": "credentials",
            "default": {"secret": "must not leak"}
        }]);
        let result = normalize_probe_output(&output);
        let step = &result["extensionRegistries"][0]["items"][0];
        assert_eq!(step["displayName"], "Shell");
        assert_eq!(step["parameters"][0]["type"], "java.lang.String");
        assert_eq!(step["parameters"][0]["required"], true);
        assert_eq!(step["parameters"][0]["default"], "echo ok");
        assert!(step["parameters"][0].get("unsafe").is_none());
        assert!(step["parameters"][1].get("default").is_none());
    }

    #[test]
    fn preserves_partial_results_when_one_registry_is_unavailable() {
        let mut output = valid_probe_output();
        output["extensionRegistries"][1]["status"] = json!("unavailable");
        output["extensionRegistries"][1]["errorCode"] = json!("extension-type-unavailable");
        output["errors"] = json!([{
            "probe": "pipeline-globals",
            "code": "extension-type-unavailable"
        }]);
        let result = normalize_probe_output(&output);

        assert_eq!(result["status"], "partial");
        assert_eq!(result["groovyVersion"], "4.0.24");
        assert_eq!(result["extensionRegistries"][0]["items"][0]["name"], "sh");
        assert_eq!(result["extensionRegistries"][1]["status"], "unavailable");
        assert_eq!(result["errors"][0]["code"], "extension-type-unavailable");
    }

    #[test]
    fn rejects_incompatible_probe_versions_without_static_fallback() {
        let mut output = valid_probe_output();
        output["schemaVersion"] = json!("2.0.0");
        let result = normalize_probe_output(&output);

        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["errors"][0]["code"], "incompatible-probe-output");
        assert!(result.get("groovyVersion").is_none());
    }

    #[test]
    fn enforces_item_and_string_limits_and_redacts_unknown_fields() {
        let mut output = valid_probe_output();
        output["apiKey"] = json!("must-not-leak");
        let long_name = "x".repeat(MAX_TEXT_LENGTH + 50);
        output["extensionRegistries"][0]["items"] = json!(
            (0..MAX_ITEMS_PER_REGISTRY + 10)
                .map(|index| json!({
                    "name": long_name,
                    "className": format!("example.Type{index}")
                }))
                .collect::<Vec<_>>()
        );
        let result = normalize_probe_output(&output);

        assert_eq!(
            result["extensionRegistries"][0]["items"].as_array().unwrap().len(),
            MAX_ITEMS_PER_REGISTRY
        );
        assert_eq!(
            result["extensionRegistries"][0]["items"][0]["name"]
                .as_str()
                .unwrap()
                .len(),
            MAX_TEXT_LENGTH
        );
        assert_eq!(result["extensionRegistries"][0]["truncated"], true);
        assert!(!result.to_string().contains("must-not-leak"));
    }

    #[test]
    fn reports_script_console_permission_failures_without_response_content() {
        let (controller, _) = mock_server(
            "403 Forbidden",
            b"denied secret response".to_vec(),
        );
        let endpoint = script_console_endpoint(&controller).unwrap();
        let result = collect_with_client(&endpoint, None, None, &test_client());

        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["errors"][0]["code"], "permission-denied");
        assert!(!result.to_string().contains("secret"));
    }

    #[test]
    fn rejects_oversized_script_console_output() {
        let (controller, _) = mock_server(
            "200 OK",
            vec![b'x'; MAX_SCRIPT_RESPONSE_BYTES + 1],
        );
        let endpoint = script_console_endpoint(&controller).unwrap();
        let result = collect_with_client(&endpoint, None, None, &test_client());

        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["errors"][0]["code"], "response-too-large");
    }


    #[test]
    fn rejects_unpaired_credentials_before_contacting_jenkins() {
        let result = collect_script_console("https://jenkins.example.test", Some("reader"), None);
        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["errors"][0]["code"], "credentials-must-be-paired");
    }

    #[test]
    fn rejects_non_http_and_embedded_credential_urls() {
        for value in [
            "file:///tmp/jenkins",
            "ftp://jenkins.example.test",
            "https://reader:secret@jenkins.example.test",
            "https://jenkins.example.test:0",
        ] {
            assert!(script_console_endpoint(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn rejects_oversized_close_delimited_output_without_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = vec![b'x'; MAX_SCRIPT_RESPONSE_BYTES + 1];
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            if stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .is_err()
            {
                return;
            }
            for chunk in body.chunks(8192) {
                if stream.write_all(chunk).is_err() {
                    return;
                }
            }
        });
        let endpoint = script_console_endpoint(&format!("http://{address}/jenkins")).unwrap();
        let result = collect_with_client(&endpoint, None, None, &test_client());
        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["errors"][0]["code"], "response-too-large");
    }

    #[test]
    fn fixed_probe_source_does_not_accept_or_evaluate_caller_groovy() {
        assert!(SCRIPT_TEMPLATE.contains("StepDescriptor"));
        assert!(SCRIPT_TEMPLATE.contains("GlobalVariable"));
        assert!(!SCRIPT_TEMPLATE.contains("GroovyShell"));
        assert!(!SCRIPT_TEMPLATE.contains("evaluate("));
        assert!(!SCRIPT_TEMPLATE.contains(".toString("));
    }

    #[test]
    fn dry_run_plan_lists_the_probe_without_claiming_to_execute_it() {
        let plan = script_console_plan();

        assert_eq!(plan["profile"], "deep");
        assert_eq!(plan["confirmationRequired"], true);
        assert_eq!(plan["callsMade"], false);
        assert_eq!(plan["requestCount"], 4);
        assert_eq!(plan["networkRequests"].as_array().unwrap().len(), 4);
        assert_eq!(plan["networkRequests"][0]["responseCapBytes"], 1_048_576);
        assert_eq!(plan["networkRequests"][1]["responseCapBytes"], 4_194_304);
        assert_eq!(plan["networkRequests"][2]["responseReadCapBytes"], 1);
        assert_eq!(plan["networkRequests"][3]["method"], "POST");
        assert_eq!(plan["networkRequests"][3]["path"], "/scriptText");
        assert_eq!(plan["networkRequests"][3]["responseCapBytes"], MAX_SCRIPT_RESPONSE_BYTES);
        assert_eq!(plan["scriptConsole"]["registries"][0], "pipeline-steps");
        assert_eq!(plan["scriptConsole"]["serverSideTerminationGuaranteed"], false);
        assert!(plan["scriptConsole"]["timeoutBehavior"].as_str().unwrap().contains("may continue"));
    }
}
