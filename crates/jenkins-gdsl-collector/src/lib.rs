use catalog_validator::validate_catalog;
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::{
    io::Read,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_GDSL_SYMBOLS: usize = 10_000;
const MAX_PARAMETERS_PER_SYMBOL: usize = 256;
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum CollectorError {
    InvalidControllerUrl,
    RequestFailed,
    RequestTimeout,
    ResponseTooLarge { limit: usize },
    InvalidEncoding,
    TooManySymbols { limit: usize },
    TooManyParameters { limit: usize },
    Unavailable(u16),
    AuthenticationFailed,
    EmptyResponse,
    MalformedGdsl(String),
    InvalidCatalog(String),
}

impl std::fmt::Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidControllerUrl => write!(f, "controller URL must be an HTTP(S) URL without embedded credentials"),
            Self::RequestFailed => write!(f, "request to Jenkins failed (request details are redacted)"),
            Self::RequestTimeout => write!(f, "request to Jenkins timed out"),
            Self::ResponseTooLarge { limit } => write!(f, "Jenkins GDSL response exceeded the {limit}-byte limit"),
            Self::InvalidEncoding => write!(f, "Jenkins GDSL response is not valid UTF-8"),
            Self::TooManySymbols { limit } => write!(f, "Jenkins GDSL exceeded the {limit}-symbol limit"),
            Self::TooManyParameters { limit } => write!(f, "Jenkins GDSL method exceeded the {limit}-parameter limit"),
            Self::Unavailable(code) => write!(f, "Jenkins GDSL endpoint returned HTTP {code}"),
            Self::AuthenticationFailed => write!(f, "Jenkins rejected the supplied credentials"),
            Self::EmptyResponse => write!(f, "Jenkins GDSL endpoint returned an empty response"),
            Self::MalformedGdsl(message) => write!(f, "could not parse Jenkins GDSL: {message}"),
            Self::InvalidCatalog(message) => write!(f, "generated catalog is invalid: {message}"),
        }
    }
}
impl std::error::Error for CollectorError {}

/// Fetches the controller's generated Pipeline DSL. Credentials are supplied separately
/// and are never included in URLs, errors, metadata, or the returned catalog.
pub fn collect(
    controller: &str,
    username: Option<&str>,
    api_token: Option<&str>,
) -> Result<Value, CollectorError> {
    let mut base = Url::parse(controller).map_err(|_| CollectorError::InvalidControllerUrl)?;
    if !matches!(base.scheme(), "http" | "https")
        || !base.username().is_empty()
        || base.password().is_some()
        || base.host_str().is_none()
    {
        return Err(CollectorError::InvalidControllerUrl);
    }
    base.set_query(None);
    base.set_fragment(None);
    let mut segments = base.path_segments_mut().map_err(|_| CollectorError::InvalidControllerUrl)?;
    segments.pop_if_empty();
    segments.push("pipeline-syntax").push("gdsl");
    drop(segments);

    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| CollectorError::RequestFailed)?;
    collect_with_client(controller, &base, username, api_token, &client)
}

fn collect_with_client(
    controller: &str,
    endpoint: &Url,
    username: Option<&str>,
    api_token: Option<&str>,
    client: &Client,
) -> Result<Value, CollectorError> {
    let mut request = client.get(endpoint.clone());
    if username.is_some() != api_token.is_some() {
        return Err(CollectorError::RequestFailed);
    }
    if let (Some(user), Some(token)) = (username, api_token) {
        request = request.basic_auth(user, Some(token));
    }
    let response = request.send().map_err(|error| {
        if error.is_timeout() {
            CollectorError::RequestTimeout
        } else {
            CollectorError::RequestFailed
        }
    })?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(CollectorError::AuthenticationFailed);
    }
    if !status.is_success() {
        return Err(CollectorError::Unavailable(status.as_u16()));
    }
    let headers = response.headers().clone();
    let mut body = Vec::with_capacity(64 * 1024);
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                CollectorError::RequestTimeout
            } else {
                CollectorError::RequestFailed
            }
        })?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(CollectorError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let body = String::from_utf8(body).map_err(|_| CollectorError::InvalidEncoding)?;
    if body.trim().is_empty() {
        return Err(CollectorError::EmptyResponse);
    }
    catalog_from_gdsl(controller, &headers, &body)
}

/// Convert GDSL method declarations into catalog step symbols.
/// Only explicit declarations are emitted; unknown parameter types/defaults remain absent.
pub fn catalog_from_gdsl(
    controller: &str,
    headers: &HeaderMap,
    gdsl: &str,
) -> Result<Value, CollectorError> {
    if gdsl.len() > MAX_RESPONSE_BYTES {
        return Err(CollectorError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let methods = parse_methods(gdsl)?;
    if methods.is_empty() {
        return Err(CollectorError::MalformedGdsl(
            "no explicit method(name: ...) declarations found".into(),
        ));
    }
    let controller_url = normalized_controller(controller)?;
    let version = headers
        .get("X-Jenkins")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let retrieved_at = timestamp();
    let source_id = format!("jenkins-controller-{}", stable_id(&controller_url));
    let endpoint = endpoint_for(controller)?;
    let symbols = methods.into_iter().map(|method| {
        let id = format!("{}:step:{}:{}", source_id, method.name, method.line);
        let mut symbol = json!({
            "id": id,
            "name": method.name,
            "kind": "step",
            "sourceId": source_id,
            "signature": method.signature,
            "confidence": "verified",
            "provenance": [format!("GET {endpoint}; GDSL method declaration at line {}", method.line)]
        });
        if let Some(description) = method.description {
            symbol["description"] = json!(description);
        }
        if let Some(url) = method.documentation_url {
            symbol["documentationUrl"] = json!(url);
        }
        if !method.parameters.is_empty() {
            symbol["parameters"] = json!(method.parameters);
        }
        symbol
    }).collect::<Vec<_>>();
    let catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": format!("jenkins-controller-{}", stable_id(&controller_url)),
        "generatedAt": retrieved_at,
        "sources": [{
            "id": source_id,
            "kind": "jenkins-controller",
            "name": controller_url.trim_end_matches('/'),
            "version": version,
            "revision": format!("retrieved:{retrieved_at}"),
            "url": controller_url.trim_end_matches('/'),
            "metadata": {
                "endpoint": endpoint,
                "retrievedAt": retrieved_at,
                "jenkinsVersion": version
            }
        }],
        "symbols": symbols
    });
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
        .map_err(|error| CollectorError::InvalidCatalog(error.to_string()))?;
    validate_catalog(&catalog, &schema)
        .map_err(|error| CollectorError::InvalidCatalog(error.to_string()))?;
    Ok(catalog)
}

struct Method {
    name: String,
    signature: String,
    description: Option<String>,
    documentation_url: Option<String>,
    parameters: Vec<Value>,
    line: usize,
}

fn parse_methods(input: &str) -> Result<Vec<Method>, CollectorError> {
    // Jenkins GDSL emits method(name: 'step', type: '...', params: [...], doc: '...').
    // Capture one declaration at a time, respecting nested brackets and quoted strings.
    let marker = Regex::new(r"\bmethod\s*\(").expect("constant regex");
    let name_re = Regex::new(r#"\bname\s*:\s*(?:'([^'\r\n]+)'|"([^"\r\n]+)")"#).expect("constant regex");
    let type_re = Regex::new(r#"\btype\s*:\s*(?:'([^'\r\n]+)'|"([^"\r\n]+)")"#).expect("constant regex");
    let doc_re = Regex::new(r#"\b(?:doc|description)\s*:\s*(?:'((?:\\.|[^'])*)'|"((?:\\.|[^"])*)")"#).expect("constant regex");
    let params_re = Regex::new(r"\bparams\s*:\s*\[([^\]]*)\]").expect("constant regex");
    let param_re = Regex::new(r#"([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(?:'((?:\\.|[^'])*)'|"((?:\\.|[^"])*)")"#).expect("constant regex");
    let mut methods = Vec::new();
    for found in marker.find_iter(input) {
        if methods.len() >= MAX_GDSL_SYMBOLS {
            return Err(CollectorError::TooManySymbols {
                limit: MAX_GDSL_SYMBOLS,
            });
        }
        let start = found.end();
        let end = matching_paren(input, start - 1).ok_or_else(|| {
            CollectorError::MalformedGdsl(format!("unterminated method declaration at line {}", line_number(input, found.start())))
        })?;
        let declaration = &input[start..end];
        let Some(captures) = name_re.captures(declaration) else { continue };
        let name = captures.get(1).or_else(|| captures.get(2)).unwrap().as_str().to_owned();
        let return_type = type_re.captures(declaration).map(|c| c.get(1).or_else(|| c.get(2)).unwrap().as_str().to_owned()).unwrap_or_else(|| "Object".into());
        let description = doc_re.captures(declaration).map(|c| unescape(c.get(1).or_else(|| c.get(2)).unwrap().as_str()));
        let documentation_url = description.as_deref().and_then(extract_url);
        let parameters: Vec<Value> = params_re.captures(declaration).map(|captures| {
            param_re.captures_iter(&captures[1])
                .take(MAX_PARAMETERS_PER_SYMBOL + 1)
                .map(|c| json!({"name": &c[1], "description": unescape(c.get(2).or_else(|| c.get(3)).unwrap().as_str())}))
                .collect()
        }).unwrap_or_default();
        if parameters.len() > MAX_PARAMETERS_PER_SYMBOL {
            return Err(CollectorError::TooManyParameters {
                limit: MAX_PARAMETERS_PER_SYMBOL,
            });
        }
        let parameter_names = parameters.iter().filter_map(|p| p["name"].as_str()).collect::<Vec<_>>();
        let signature = format!("{}({}) -> {}", name, parameter_names.join(", "), return_type);
        methods.push(Method { name, signature, description, documentation_url, parameters, line: line_number(input, found.start()) });
    }
    Ok(methods)
}

fn matching_paren(input: &str, open: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        if let Some(q) = quote {
            if escaped { escaped = false; }
            else if *byte == b'\\' { escaped = true; }
            else if *byte == q { quote = None; }
            continue;
        }
        if *byte == b'\'' || *byte == b'"' { quote = Some(*byte); }
        else if *byte == b'(' { depth += 1; }
        else if *byte == b')' {
            depth -= 1;
            if depth == 0 { return Some(index); }
        }
    }
    None
}

fn unescape(value: &str) -> String {
    value.replace("\\'", "'").replace("\\\"", "\"").replace("\\\\", "\\")
}
fn extract_url(value: &str) -> Option<String> {
    value.split_whitespace().find(|part| part.starts_with("https://") || part.starts_with("http://"))
        .map(|part| part.trim_end_matches([')', ',', ';']).to_owned())
}
fn line_number(input: &str, offset: usize) -> usize { input[..offset].bytes().filter(|b| *b == b'\n').count() + 1 }
fn stable_id(value: &str) -> String {
    // Deterministic, non-cryptographic identifier; no credentials are accepted in the URL.
    let hash = value.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
    format!("{hash:08x}")
}
fn normalized_controller(controller: &str) -> Result<String, CollectorError> {
    let mut url = Url::parse(controller).map_err(|_| CollectorError::InvalidControllerUrl)?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() || url.host_str().is_none() {
        return Err(CollectorError::InvalidControllerUrl);
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}
fn endpoint_for(controller: &str) -> Result<String, CollectorError> {
    let mut url = Url::parse(&normalized_controller(controller)?).map_err(|_| CollectorError::InvalidControllerUrl)?;
    { let mut path = url.path_segments_mut().map_err(|_| CollectorError::InvalidControllerUrl)?;
      path.pop_if_empty(); path.push("pipeline-syntax").push("gdsl"); }
    Ok(url.to_string())
}
fn timestamp() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (seconds / 86400) as i64;
    let rem = seconds % 86400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2)/153;
    let d = doy - (153*mp+2)/5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use std::{io::Write, net::TcpListener, thread};

    const FIXTURE: &str = include_str!("../tests/fixtures/pipeline.gdsl");

    fn mock_jenkins(status: &str, body: Vec<u8>, delay: Duration) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            if !delay.is_zero() {
                thread::sleep(delay);
            }
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        });
        Url::parse(&format!("http://{address}/pipeline-syntax/gdsl")).unwrap()
    }

    fn test_client(timeout: Duration) -> Client {
        Client::builder().connect_timeout(Duration::from_secs(1)).timeout(timeout).build().unwrap()
    }

    #[test]
    fn extracts_steps_parameters_docs_and_provenance() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Jenkins", HeaderValue::from_static("2.479.1"));
        let catalog = catalog_from_gdsl("https://ci.example.test/jenkins", &headers, FIXTURE).unwrap();
        assert_eq!(catalog["sources"][0]["kind"], "jenkins-controller");
        assert_eq!(catalog["sources"][0]["version"], "2.479.1");
        assert_eq!(catalog["symbols"].as_array().unwrap().len(), 2);
        assert_eq!(catalog["symbols"][0]["name"], "sh");
        assert_eq!(catalog["symbols"][0]["parameters"][0]["name"], "script");
        assert_eq!(catalog["symbols"][0]["documentationUrl"], "https://www.jenkins.io/doc/pipeline/steps/workflow-durable-task-step/#sh-shell-script");
        assert_eq!(catalog["symbols"][0]["confidence"], "verified");
        assert!(catalog["symbols"][0]["provenance"][0].as_str().unwrap().contains("line 2"));
    }

    #[test]
    fn rejects_gdsl_without_explicit_methods() {
        let error = catalog_from_gdsl("https://ci.example.test", &HeaderMap::new(), "not gdsl").unwrap_err();
        assert!(matches!(error, CollectorError::MalformedGdsl(_)));
    }

    #[test]
    fn rejects_unterminated_method() {
        let error = catalog_from_gdsl("https://ci.example.test", &HeaderMap::new(), "method(name: 'sh'").unwrap_err();
        assert!(matches!(error, CollectorError::MalformedGdsl(_)));
    }

    #[test]
    fn controller_url_cannot_embed_credentials() {
        assert!(matches!(endpoint_for("https://user:secret@ci.example.test"), Err(CollectorError::InvalidControllerUrl)));
    }

    #[test]
    fn controller_query_and_fragment_are_not_recorded() {
        let catalog = catalog_from_gdsl("https://ci.example.test/jenkins?token=secret#fragment", &HeaderMap::new(), FIXTURE).unwrap();
        assert_eq!(catalog["sources"][0]["url"], "https://ci.example.test/jenkins");
        assert!(!catalog.to_string().contains("secret"));
    }



    #[test]
    fn collects_successful_http_response() {
        let endpoint = mock_jenkins("200 OK", FIXTURE.as_bytes().to_vec(), Duration::ZERO);
        let controller = format!("http://{}", endpoint.host_str().unwrap());
        let result = collect_with_client(&controller, &endpoint, None, None, &test_client(Duration::from_secs(1))).unwrap();
        assert_eq!(result["symbols"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn reports_authentication_http_status() {
        let endpoint = mock_jenkins("401 Unauthorized", b"denied".to_vec(), Duration::ZERO);
        let error = collect_with_client("http://localhost", &endpoint, None, None, &test_client(Duration::from_secs(1))).unwrap_err();
        assert_eq!(error, CollectorError::AuthenticationFailed);
    }

    #[test]
    fn reports_non_success_http_status() {
        let endpoint = mock_jenkins("503 Service Unavailable", b"unavailable".to_vec(), Duration::ZERO);
        let error = collect_with_client("http://localhost", &endpoint, None, None, &test_client(Duration::from_secs(1))).unwrap_err();
        assert_eq!(error, CollectorError::Unavailable(503));
    }

    #[test]
    fn reports_empty_http_response() {
        let endpoint = mock_jenkins("200 OK", Vec::new(), Duration::ZERO);
        let error = collect_with_client("http://localhost", &endpoint, None, None, &test_client(Duration::from_secs(1))).unwrap_err();
        assert_eq!(error, CollectorError::EmptyResponse);
    }

    #[test]
    fn rejects_non_utf8_http_response() {
        let endpoint = mock_jenkins("200 OK", vec![0xff, 0xfe], Duration::ZERO);
        let error = collect_with_client("http://localhost", &endpoint, None, None, &test_client(Duration::from_secs(1))).unwrap_err();
        assert_eq!(error, CollectorError::InvalidEncoding);
    }

    #[test]
    fn rejects_oversized_http_response_while_reading() {
        let endpoint = mock_jenkins("200 OK", vec![b'x'; MAX_RESPONSE_BYTES + 1], Duration::ZERO);
        let error = collect_with_client("http://localhost", &endpoint, None, None, &test_client(Duration::from_secs(2))).unwrap_err();
        assert_eq!(error, CollectorError::ResponseTooLarge { limit: MAX_RESPONSE_BYTES });
    }

    #[test]
    fn reports_http_request_timeout() {
        let endpoint = mock_jenkins("200 OK", FIXTURE.as_bytes().to_vec(), Duration::from_millis(100));
        let error = collect_with_client("http://localhost", &endpoint, None, None, &test_client(Duration::from_millis(10))).unwrap_err();
        assert_eq!(error, CollectorError::RequestTimeout);
    }

    #[test]
    fn rejects_oversized_gdsl_before_parsing() {
        let input = " ".repeat(MAX_RESPONSE_BYTES + 1);
        let error = catalog_from_gdsl("https://ci.example.test", &HeaderMap::new(), &input).unwrap_err();
        assert_eq!(error, CollectorError::ResponseTooLarge { limit: MAX_RESPONSE_BYTES });
    }

    #[test]
    fn rejects_excessive_symbol_count() {
        let input = "method(name: 'x')\n".repeat(MAX_GDSL_SYMBOLS + 1);
        let error = parse_methods(&input).unwrap_err();
        assert_eq!(error, CollectorError::TooManySymbols { limit: MAX_GDSL_SYMBOLS });
    }

    #[test]
    fn rejects_excessive_parameter_count() {
        let params = (0..=MAX_PARAMETERS_PER_SYMBOL)
            .map(|index| format!("p{index}: 'description'"))
            .collect::<Vec<_>>()
            .join(", ");
        let input = format!("method(name: 'many', params: [{params}])");
        let error = parse_methods(&input).unwrap_err();
        assert_eq!(error, CollectorError::TooManyParameters { limit: MAX_PARAMETERS_PER_SYMBOL });
    }

    #[test]
    fn generated_catalog_has_no_secret_material() {
        let catalog = catalog_from_gdsl("https://ci.example.test", &HeaderMap::new(), FIXTURE).unwrap();
        let serialized = catalog.to_string();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("token"));
    }
}
