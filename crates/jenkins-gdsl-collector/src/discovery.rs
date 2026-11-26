use catalog_validator::{sha256_hex, stamp_catalog_identity, validate_catalog};
use reqwest::{
    blocking::{Client, Response},
    header::{HeaderMap, ACCEPT, RANGE},
    redirect::Policy,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Read,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CORE_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_PLUGIN_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PLUGINS: usize = 5_000;
const MAX_PLUGIN_DEPENDENCIES: usize = 256;

const CORE_ENDPOINT: &str = "GET /api/json";
const PLUGIN_ENDPOINT: &str = "GET /pluginManager/api/json";
const GDSL_ENDPOINT: &str = "GET /pipeline-syntax/gdsl";

#[derive(Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    InvalidControllerUrl,
    CredentialsMustBePaired,
    RequestFailed { endpoint: &'static str },
    RequestTimeout { endpoint: &'static str },
    ResponseTooLarge { endpoint: &'static str, limit: usize },
    AuthenticationFailed { endpoint: &'static str },
    Unavailable { endpoint: &'static str, status: u16 },
    MalformedJson { endpoint: &'static str },
    InvalidPluginInventory,
    TooManyPlugins { limit: usize },
    TooManyDependencies { plugin: String, limit: usize },
    InvalidCatalog(String),
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidControllerUrl => write!(
                f,
                "controller URL must be HTTP(S) and must not contain embedded credentials"
            ),
            Self::CredentialsMustBePaired => write!(
                f,
                "set both JENKINS_USERNAME and JENKINS_API_TOKEN, or neither"
            ),
            Self::RequestFailed { endpoint } => {
                write!(f, "{endpoint} request failed (request details are redacted)")
            }
            Self::RequestTimeout { endpoint } => write!(f, "{endpoint} timed out"),
            Self::ResponseTooLarge { endpoint, limit } => {
                write!(f, "{endpoint} response exceeded the {limit}-byte limit")
            }
            Self::AuthenticationFailed { endpoint } => {
                write!(f, "{endpoint} rejected the supplied credentials (HTTP 401/403)")
            }
            Self::Unavailable { endpoint, status } => {
                write!(f, "{endpoint} returned HTTP {status}")
            }
            Self::MalformedJson { endpoint } => {
                write!(f, "{endpoint} returned malformed JSON")
            }
            Self::InvalidPluginInventory => {
                write!(f, "Jenkins plugin inventory has an unsupported structure")
            }
            Self::TooManyPlugins { limit } => {
                write!(f, "Jenkins plugin inventory exceeded the {limit}-plugin limit")
            }
            Self::TooManyDependencies { plugin, limit } => write!(
                f,
                "plugin '{plugin}' exceeded the {limit}-dependency limit"
            ),
            Self::InvalidCatalog(message) => {
                write!(f, "generated controller snapshot is invalid: {message}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

enum Probe<T> {
    Available(T),
    Unavailable(u16),
    Unknown(String),
}

/// Runs the bounded quick discovery profile against a Jenkins controller.
///
/// The profile performs three sequential requests: bounded core metadata, bounded
/// plugin inventory, and a one-byte Range probe of the Pipeline GDSL endpoint.
/// Credentials are supplied separately and never enter URLs, diagnostics, or output.
pub fn discover(
    controller: &str,
    username: Option<&str>,
    api_token: Option<&str>,
) -> Result<Value, DiscoveryError> {
    if username.is_some() != api_token.is_some() {
        return Err(DiscoveryError::CredentialsMustBePaired);
    }

    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .build()
        .map_err(|_| DiscoveryError::RequestFailed {
            endpoint: CORE_ENDPOINT,
        })?;

    discover_with_client(controller, username, api_token, &client)
}

fn discover_with_client(
    controller: &str,
    username: Option<&str>,
    api_token: Option<&str>,
    client: &Client,
) -> Result<Value, DiscoveryError> {
    if username.is_some() != api_token.is_some() {
        return Err(DiscoveryError::CredentialsMustBePaired);
    }

    let controller_url = normalized_controller(controller)?;
    let mut core_url = endpoint_for(&controller_url, &["api", "json"])?;
    let mut plugin_url = endpoint_for(&controller_url, &["pluginManager", "api", "json"])?;
    let gdsl_url = endpoint_for(&controller_url, &["pipeline-syntax", "gdsl"])?;
    core_url
        .query_pairs_mut()
        .append_pair("tree", "_class,nodeName,jenkinsVersion");
    plugin_url.query_pairs_mut().append_pair(
        "tree",
        "plugins[shortName,version,active,enabled,dependencies[shortName,version,optional]]",
    );

    let started_at = timestamp();
    let (core_headers, core_json) = required_json(
        client,
        &core_url,
        username,
        api_token,
        CORE_ENDPOINT,
        MAX_CORE_RESPONSE_BYTES,
    )?;

    let core_version = core_headers
        .get("X-Jenkins")
        .and_then(|value| value.to_str().ok())
        .or_else(|| core_json.get("jenkinsVersion").and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("unknown")
        .to_owned();

    let controller_id = format!("jenkins-controller-{}", stable_id(&controller_url));
    let plugin_probe = optional_json_probe(
        client,
        &plugin_url,
        username,
        api_token,
        PLUGIN_ENDPOINT,
        MAX_PLUGIN_RESPONSE_BYTES,
    )?;

    let (plugins, mut plugin_sources, plugin_capability, plugin_coverage) = match plugin_probe {
        Probe::Available(value) => match parse_plugin_inventory(&value, &controller_id) {
            Ok((plugins, sources)) => (
                plugins,
                sources,
                json!({
                    "name": "plugin-manager-api",
                    "status": "available",
                    "details": "GET /pluginManager/api/json returned a valid bounded plugin inventory"
                }),
                json!({
                    "sourceId": controller_id,
                    "surface": "plugin-inventory",
                    "status": "inspected"
                }),
            ),
            Err(DiscoveryError::TooManyPlugins { limit }) => {
                return Err(DiscoveryError::TooManyPlugins { limit });
            }
            Err(DiscoveryError::TooManyDependencies { plugin, limit }) => {
                return Err(DiscoveryError::TooManyDependencies { plugin, limit });
            }
            Err(_) => (
                Vec::new(),
                Vec::new(),
                json!({
                    "name": "plugin-manager-api",
                    "status": "unknown",
                    "details": "plugin inventory response did not match the supported structure"
                }),
                json!({
                    "sourceId": controller_id,
                    "surface": "plugin-inventory",
                    "status": "not-collected",
                    "details": "plugin inventory response did not match the supported structure"
                }),
            ),
        },
        Probe::Unavailable(status) => (
            Vec::new(),
            Vec::new(),
            json!({
                "name": "plugin-manager-api",
                "status": "unavailable",
                "details": format!("GET /pluginManager/api/json returned HTTP {status}")
            }),
            json!({
                "sourceId": controller_id,
                "surface": "plugin-inventory",
                "status": "unavailable",
                "details": format!("GET /pluginManager/api/json returned HTTP {status}")
            }),
        ),
        Probe::Unknown(reason) => (
            Vec::new(),
            Vec::new(),
            json!({
                "name": "plugin-manager-api",
                "status": "unknown",
                "details": reason
            }),
            json!({
                "sourceId": controller_id,
                "surface": "plugin-inventory",
                "status": "not-collected",
                "details": reason
            }),
        ),
    };

    let gdsl_probe = availability_probe(
        client,
        &gdsl_url,
        username,
        api_token,
        GDSL_ENDPOINT,
    );
    let (gdsl_capability, gdsl_coverage) = match gdsl_probe {
        Probe::Available(()) => (
            json!({
                "name": "pipeline-gdsl",
                "status": "available",
                "details": "Pipeline GDSL endpoint accepted a Range: bytes=0-0 probe"
            }),
            json!({
                "sourceId": controller_id,
                "surface": "pipeline-gdsl",
                "status": "not-collected",
                "details": "quick profile checked endpoint availability only; GDSL content was not downloaded"
            }),
        ),
        Probe::Unavailable(status) => (
            json!({
                "name": "pipeline-gdsl",
                "status": "unavailable",
                "details": format!("GET /pipeline-syntax/gdsl returned HTTP {status}")
            }),
            json!({
                "sourceId": controller_id,
                "surface": "pipeline-gdsl",
                "status": "unavailable",
                "details": format!("GET /pipeline-syntax/gdsl returned HTTP {status}")
            }),
        ),
        Probe::Unknown(reason) => (
            json!({
                "name": "pipeline-gdsl",
                "status": "unknown",
                "details": reason
            }),
            json!({
                "sourceId": controller_id,
                "surface": "pipeline-gdsl",
                "status": "not-collected",
                "details": reason
            }),
        ),
    };

    let completed_at = timestamp();
    let mut identity = json!({
        "id": controller_id,
        "baseUrl": controller_url
    });
    if let Some(node_name) = core_json
        .get("nodeName")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        identity["displayName"] = json!(node_name);
    }

    let mut capabilities = vec![
        json!({
            "name": "core-api",
            "status": "available",
            "details": "GET /api/json returned valid controller metadata"
        }),
        plugin_capability,
        gdsl_capability
    ];
    capabilities.sort_by_key(|capability| capability["name"].as_str().unwrap_or_default().to_owned());
    let controller_revision_input = json!({
        "identity": identity,
        "coreVersion": core_version,
        "plugins": plugins,
        "capabilities": capabilities
    });
    let controller_revision_bytes = serde_json::to_vec(&controller_revision_input)
        .map_err(|_| DiscoveryError::InvalidCatalog("cannot serialize controller revision input".into()))?;
    let controller_revision = sha256_hex(&controller_revision_bytes);

    let mut sources = vec![json!({
        "id": controller_id,
        "kind": "jenkins-controller",
        "name": controller_url,
        "revision": controller_revision,
        "url": controller_url,
        "metadata": {
            "collectionProfile": "quick",
            "endpoints": [
                "GET /api/json?tree=_class,nodeName,jenkinsVersion",
                "GET /pluginManager/api/json?tree=plugins[shortName,version,active,enabled,dependencies[shortName,version,optional]]",
                "GET /pipeline-syntax/gdsl (Range: bytes=0-0)"
            ]
        }
    })];
    sources.append(&mut plugin_sources);

    let mut catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": controller_id,
        "generatedAt": completed_at,
        "sources": sources,
        "symbols": [],
        "coverage": [
            {
                "sourceId": controller_id,
                "surface": "controller-metadata",
                "status": "inspected"
            },
            plugin_coverage,
            gdsl_coverage
        ],
        "controller": {
            "identity": identity,
            "coreVersion": core_version,
            "plugins": plugins,
            "capabilities": capabilities,
            "collection": {
                "profile": "quick",
                "startedAt": started_at,
                "retrievedAt": completed_at,
                "completedAt": completed_at,
                "requestCount": 3
            }
        }
    });

    stamp_catalog_identity(&mut catalog)
        .map_err(|error| DiscoveryError::InvalidCatalog(error))?;
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
        .map_err(|error| DiscoveryError::InvalidCatalog(error.to_string()))?;
    validate_catalog(&catalog, &schema)
        .map_err(|error| DiscoveryError::InvalidCatalog(error.to_string()))?;
    Ok(catalog)
}

fn required_json(
    client: &Client,
    url: &Url,
    username: Option<&str>,
    api_token: Option<&str>,
    endpoint: &'static str,
    limit: usize,
) -> Result<(HeaderMap, Value), DiscoveryError> {
    let response = authenticated_get(client, url, username, api_token)
        .send()
        .map_err(|error| request_error(endpoint, error.is_timeout()))?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(DiscoveryError::AuthenticationFailed { endpoint });
    }
    if !status.is_success() {
        return Err(DiscoveryError::Unavailable {
            endpoint,
            status: status.as_u16(),
        });
    }

    let headers = response.headers().clone();
    let bytes = read_limited(response, endpoint, limit)?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| DiscoveryError::MalformedJson { endpoint })?;
    Ok((headers, value))
}

fn optional_json_probe(
    client: &Client,
    url: &Url,
    username: Option<&str>,
    api_token: Option<&str>,
    endpoint: &'static str,
    limit: usize,
) -> Result<Probe<Value>, DiscoveryError> {
    let response = match authenticated_get(client, url, username, api_token).send() {
        Ok(response) => response,
        Err(error) => {
            return Ok(Probe::Unknown(if error.is_timeout() {
                format!("{endpoint} timed out")
            } else {
                format!("{endpoint} request failed; details redacted")
            }));
        }
    };

    let status = response.status();
    if status.as_u16() == 401
        || status.as_u16() == 403
        || status.as_u16() == 404
        || status.as_u16() == 410
    {
        return Ok(Probe::Unavailable(status.as_u16()));
    }
    if !status.is_success() {
        return Ok(Probe::Unknown(format!(
            "{endpoint} returned HTTP {}",
            status.as_u16()
        )));
    }

    let bytes = match read_limited(response, endpoint, limit) {
        Ok(bytes) => bytes,
        Err(error @ DiscoveryError::ResponseTooLarge { .. }) => return Err(error),
        Err(DiscoveryError::RequestTimeout { .. }) => {
            return Ok(Probe::Unknown(format!("{endpoint} timed out")));
        }
        Err(_) => {
            return Ok(Probe::Unknown(format!(
                "{endpoint} response could not be read"
            )));
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Probe::Available(value)),
        Err(_) => Ok(Probe::Unknown(format!("{endpoint} returned malformed JSON"))),
    }
}

fn availability_probe(
    client: &Client,
    url: &Url,
    username: Option<&str>,
    api_token: Option<&str>,
    endpoint: &'static str,
) -> Probe<()> {
    let response = match authenticated_get(client, url, username, api_token)
        .header(RANGE, "bytes=0-0")
        .send()
    {
        Ok(response) => response,
        Err(error) => {
            return Probe::Unknown(if error.is_timeout() {
                format!("{endpoint} timed out")
            } else {
                format!("{endpoint} request failed; details redacted")
            });
        }
    };

    let status = response.status();
    if status.as_u16() == 401
        || status.as_u16() == 403
        || status.as_u16() == 404
        || status.as_u16() == 410
    {
        return Probe::Unavailable(status.as_u16());
    }
    if !status.is_success() && status.as_u16() != 416 {
        return Probe::Unknown(format!(
            "{endpoint} returned HTTP {}",
            status.as_u16()
        ));
    }

    // Read at most one byte even when the server ignores the Range header.
    let mut response = response;
    let mut byte = [0_u8; 1];
    if let Err(error) = response.read(&mut byte) {
        return Probe::Unknown(if error.kind() == std::io::ErrorKind::TimedOut {
            format!("{endpoint} timed out while reading the probe response")
        } else {
            format!("{endpoint} probe response could not be read")
        });
    }
    Probe::Available(())
}

fn authenticated_get(
    client: &Client,
    url: &Url,
    username: Option<&str>,
    api_token: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    let request = client.get(url.clone()).header(ACCEPT, "application/json");
    match (username, api_token) {
        (Some(username), Some(token)) => request.basic_auth(username, Some(token)),
        _ => request,
    }
}

fn read_limited(
    response: Response,
    endpoint: &'static str,
    limit: usize,
) -> Result<Vec<u8>, DiscoveryError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(DiscoveryError::ResponseTooLarge { endpoint, limit });
    }

    let mut body = Vec::with_capacity(64 * 1024);
    response
        .take((limit + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                DiscoveryError::RequestTimeout { endpoint }
            } else {
                DiscoveryError::RequestFailed { endpoint }
            }
        })?;
    if body.len() > limit {
        return Err(DiscoveryError::ResponseTooLarge { endpoint, limit });
    }
    Ok(body)
}

fn request_error(endpoint: &'static str, timeout: bool) -> DiscoveryError {
    if timeout {
        DiscoveryError::RequestTimeout { endpoint }
    } else {
        DiscoveryError::RequestFailed { endpoint }
    }
}

fn parse_plugin_inventory(
    value: &Value,
    controller_id: &str,
) -> Result<(Vec<Value>, Vec<Value>), DiscoveryError> {
    let entries = value
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or(DiscoveryError::InvalidPluginInventory)?;
    if entries.len() > MAX_PLUGINS {
        return Err(DiscoveryError::TooManyPlugins {
            limit: MAX_PLUGINS,
        });
    }

    let mut names = HashSet::new();
    let mut plugins = Vec::with_capacity(entries.len());
    let mut sources = Vec::with_capacity(entries.len());

    for entry in entries {
        let short_name = entry
            .get("shortName")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or(DiscoveryError::InvalidPluginInventory)?;
        if !names.insert(short_name.to_owned()) {
            return Err(DiscoveryError::InvalidPluginInventory);
        }

        let version = entry
            .get("version")
            .and_then(Value::as_str)
            .filter(|version| !version.trim().is_empty())
            .map(|version| json!(version))
            .unwrap_or(Value::Null);

        let mut dependencies = match entry.get("dependencies") {
            Some(value) => {
                let values = value
                    .as_array()
                    .ok_or(DiscoveryError::InvalidPluginInventory)?;
                if values.len() > MAX_PLUGIN_DEPENDENCIES {
                    return Err(DiscoveryError::TooManyDependencies {
                        plugin: short_name.to_owned(),
                        limit: MAX_PLUGIN_DEPENDENCIES,
                    });
                }
                values
                    .iter()
                    .map(|dependency| {
                        let dependency_name = dependency
                            .get("shortName")
                            .and_then(Value::as_str)
                            .filter(|name| !name.trim().is_empty())
                            .ok_or(DiscoveryError::InvalidPluginInventory)?;
                        let mut result = json!({ "shortName": dependency_name });
                        if let Some(version) = dependency
                            .get("version")
                            .and_then(Value::as_str)
                            .filter(|version| !version.trim().is_empty())
                        {
                            result["minimumVersion"] = json!(version);
                        }
                        if let Some(optional) =
                            dependency.get("optional").and_then(Value::as_bool)
                        {
                            result["optional"] = json!(optional);
                        }
                        Ok(result)
                    })
                    .collect::<Result<Vec<_>, DiscoveryError>>()?
            }
            None => Vec::new(),
        };
        dependencies.sort_by_key(|dependency| dependency["shortName"].as_str().unwrap_or_default().to_owned());

        let source_id = format!(
            "plugin-{}",
            stable_id(&format!("{controller_id}:{short_name}"))
        );
        sources.push(json!({
            "id": source_id,
            "kind": "plugin",
            "name": short_name,
            "version": version,
            "revision": version.as_str().unwrap_or("unknown")
        }));
        plugins.push(json!({
            "sourceId": source_id,
            "shortName": short_name,
            "version": version,
            "enabled": entry.get("enabled").and_then(Value::as_bool),
            "active": entry.get("active").and_then(Value::as_bool),
            "dependencies": dependencies
        }));
    }

    Ok((plugins, sources))
}

fn normalized_controller(controller: &str) -> Result<String, DiscoveryError> {
    let mut url = Url::parse(controller).map_err(|_| DiscoveryError::InvalidControllerUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(DiscoveryError::InvalidControllerUrl);
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn endpoint_for(controller: &str, suffix: &[&str]) -> Result<Url, DiscoveryError> {
    let mut url = Url::parse(controller).map_err(|_| DiscoveryError::InvalidControllerUrl)?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| DiscoveryError::InvalidControllerUrl)?;
    segments.pop_if_empty();
    for segment in suffix {
        segments.push(segment);
    }
    drop(segments);
    Ok(url)
}

fn stable_id(value: &str) -> String {
    let digest = sha256_hex(value.as_bytes());
    digest[7..23].to_owned()
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
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    struct MockResponse {
        status: &'static str,
        headers: &'static str,
        body: Vec<u8>,
        delay: Duration,
    }

    fn response(status: &'static str, headers: &'static str, body: Vec<u8>) -> MockResponse {
        MockResponse {
            status,
            headers,
            body,
            delay: Duration::ZERO,
        }
    }

    fn mock_sequence(
        responses: Vec<MockResponse>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);

        thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let request = read_request(&mut stream);
                recorded.lock().unwrap().push(request);
                if !response.delay.is_zero() {
                    thread::sleep(response.delay);
                }
                let headers = format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
                    response.status,
                    response.body.len(),
                    response.headers
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&response.body);
            }
        });

        (format!("http://{address}/jenkins"), requests)
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut byte = [0_u8; 1];
        while bytes.len() < 16 * 1024 {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn test_client(timeout: Duration) -> Client {
        Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .timeout(timeout)
            .redirect(Policy::none())
            .build()
            .unwrap()
    }

    fn core_response() -> MockResponse {
        response(
            "200 OK",
            "X-Jenkins: 2.479.1\r\n",
            br#"{"_class":"hudson.model.Hudson","nodeName":"controller"}"#.to_vec(),
        )
    }

    fn plugin_response() -> MockResponse {
        response(
            "200 OK",
            "",
            br#"{"plugins":[{"shortName":"workflow-cps","version":"4000.v123","enabled":true,"active":true,"dependencies":[{"shortName":"workflow-api","version":"2.0","optional":false}]},{"shortName":"workflow-api","version":"2.0","enabled":true,"active":true}]}"#.to_vec(),
        )
    }

    #[test]
    fn quick_discovery_collects_bounded_controller_and_plugin_metadata() {
        let (controller, requests) = mock_sequence(vec![
            core_response(),
            plugin_response(),
            response("206 Partial Content", "", vec![b'x']),
        ]);

        let catalog = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap();

        assert_eq!(catalog["controller"]["coreVersion"], "2.479.1");
        assert_eq!(catalog["controller"]["identity"]["displayName"], "controller");
        assert_eq!(catalog["controller"]["plugins"].as_array().unwrap().len(), 2);
        assert_eq!(
            catalog["controller"]["plugins"][0]["dependencies"][0]["shortName"],
            "workflow-api"
        );
        assert_eq!(catalog["controller"]["collection"]["profile"], "quick");
        assert_eq!(catalog["controller"]["collection"]["requestCount"], 3);
        assert_eq!(catalog["controller"]["capabilities"][2]["status"], "available");
        assert_eq!(catalog["sources"][1]["kind"], "plugin");
        assert_eq!(requests.lock().unwrap().len(), 3);
        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("GET /jenkins/api/json?tree="));
        assert!(requests[1].contains("GET /jenkins/pluginManager/api/json?tree="));
        assert!(requests[2].contains("GET /jenkins/pipeline-syntax/gdsl"));
        assert!(requests[2].to_ascii_lowercase().contains("range: bytes=0-0"));
    }

    #[test]
    fn records_missing_optional_endpoints_without_failing_discovery() {
        let (controller, _) = mock_sequence(vec![
            core_response(),
            response("404 Not Found", "", b"not found".to_vec()),
            response("404 Not Found", "", b"not found".to_vec()),
        ]);

        let catalog = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap();

        assert_eq!(catalog["controller"]["plugins"].as_array().unwrap().len(), 0);
        assert_eq!(catalog["controller"]["capabilities"][1]["status"], "unavailable");
        assert_eq!(catalog["controller"]["capabilities"][2]["status"], "unavailable");
        assert_eq!(catalog["coverage"][1]["status"], "unavailable");
    }

    #[test]
    fn records_forbidden_optional_endpoints_explicitly() {
        let (controller, _) = mock_sequence(vec![
            core_response(),
            response("403 Forbidden", "", b"forbidden".to_vec()),
            response("403 Forbidden", "", b"forbidden".to_vec()),
        ]);

        let catalog = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap();

        assert_eq!(catalog["controller"]["capabilities"][1]["status"], "unavailable");
        assert!(catalog["controller"]["capabilities"][1]["details"]
            .as_str()
            .unwrap()
            .contains("403"));
        assert_eq!(catalog["controller"]["capabilities"][2]["status"], "unavailable");
    }

    #[test]
    fn rejects_unauthorized_core_endpoint() {
        let (controller, requests) = mock_sequence(vec![response(
            "401 Unauthorized",
            "",
            b"denied".to_vec(),
        )]);

        let error = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap_err();

        assert_eq!(
            error,
            DiscoveryError::AuthenticationFailed {
                endpoint: CORE_ENDPOINT
            }
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn rejects_malformed_core_json() {
        let (controller, _) = mock_sequence(vec![response(
            "200 OK",
            "",
            b"{not json".to_vec(),
        )]);

        let error = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap_err();

        assert_eq!(
            error,
            DiscoveryError::MalformedJson {
                endpoint: CORE_ENDPOINT
            }
        );
    }

    #[test]
    fn records_malformed_optional_plugin_json_as_unknown() {
        let (controller, _) = mock_sequence(vec![
            core_response(),
            response("200 OK", "", b"{not json".to_vec()),
            response("404 Not Found", "", Vec::new()),
        ]);

        let catalog = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap();

        assert_eq!(catalog["controller"]["capabilities"][1]["status"], "unknown");
        assert_eq!(catalog["coverage"][1]["status"], "not-collected");
    }

    #[test]
    fn reports_core_request_timeout() {
        let mut delayed = core_response();
        delayed.delay = Duration::from_millis(100);
        let (controller, _) = mock_sequence(vec![delayed]);

        let error = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_millis(10)),
        )
        .unwrap_err();

        assert_eq!(
            error,
            DiscoveryError::RequestTimeout {
                endpoint: CORE_ENDPOINT
            }
        );
    }

    #[test]
    fn rejects_oversized_core_response_before_json_parsing() {
        let (controller, _) = mock_sequence(vec![response(
            "200 OK",
            "",
            vec![b'x'; MAX_CORE_RESPONSE_BYTES + 1],
        )]);

        let error = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(2)),
        )
        .unwrap_err();

        assert_eq!(
            error,
            DiscoveryError::ResponseTooLarge {
                endpoint: CORE_ENDPOINT,
                limit: MAX_CORE_RESPONSE_BYTES
            }
        );
    }

    #[test]
    fn strips_query_and_fragment_and_never_serializes_url_secrets() {
        let (base, _) = mock_sequence(vec![
            core_response(),
            plugin_response(),
            response("404 Not Found", "", Vec::new()),
        ]);
        let controller = format!("{base}?token=not-a-secret-to-persist#fragment");

        let catalog = discover_with_client(
            &controller,
            None,
            None,
            &test_client(Duration::from_secs(1)),
        )
        .unwrap();

        assert_eq!(catalog["controller"]["identity"]["baseUrl"], base);
        assert!(!catalog.to_string().contains("not-a-secret-to-persist"));
        assert!(!catalog.to_string().contains("authorization"));
    }

    #[test]
    fn requires_username_and_token_together() {
        let error = discover("https://ci.example.test", Some("reader"), None).unwrap_err();
        assert_eq!(error, DiscoveryError::CredentialsMustBePaired);
    }

    #[test]
    fn rejects_controller_urls_with_embedded_credentials() {
        assert_eq!(
            normalized_controller("https://reader:secret@ci.example.test/jenkins"),
            Err(DiscoveryError::InvalidControllerUrl)
        );
    }
}
