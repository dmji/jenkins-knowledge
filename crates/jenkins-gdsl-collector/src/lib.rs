pub mod discovery;
pub use discovery::{discover, DiscoveryError};
pub mod script_console;
pub use script_console::{collect_script_console, script_console_plan};

use catalog_validator::{redact_sensitive_fields, stamp_catalog_identity, validate_catalog, sha256_hex};
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
        let signature_hash = sha256_hex(method.signature.as_bytes());
        let id = format!("{}:step:{}:{}", source_id, method.name, &signature_hash[7..23]);
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
    let mut catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": format!("jenkins-controller-{}", stable_id(&controller_url)),
        "generatedAt": retrieved_at,
        "sources": [{
            "id": source_id,
            "kind": "jenkins-controller",
            "name": controller_url.trim_end_matches('/'),
            "version": version,
            "revision": sha256_hex(gdsl.as_bytes()),
            "url": controller_url.trim_end_matches('/'),
            "metadata": {
                "endpoint": endpoint,
                "retrievedAt": retrieved_at,
                "jenkinsVersion": version
            }
        }],
        "symbols": symbols
    });
    sanitize_collected_catalog(&mut catalog);
    stamp_catalog_identity(&mut catalog).map_err(CollectorError::InvalidCatalog)?;
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
        .map_err(|error| CollectorError::InvalidCatalog(error.to_string()))?;
    validate_catalog(&catalog, &schema)
        .map_err(|error| CollectorError::InvalidCatalog(error.to_string()))?;
    Ok(catalog)
}

#[derive(Debug)]
struct Method {
    name: String,
    signature: String,
    description: Option<String>,
    documentation_url: Option<String>,
    parameters: Vec<Value>,
    line: usize,
}

fn sanitize_collected_catalog(catalog: &mut Value) -> usize {
    redact_sensitive_fields(catalog)
}

fn parse_methods(input: &str) -> Result<Vec<Method>, CollectorError> {
    let mask = mask_non_code(input)?;
    let marker = Regex::new(r"\bmethod\s*\(").expect("constant regex");
    let mut methods = Vec::new();
    let mut search_from = 0;
    let mut declaration_count = 0;

    while let Some(found) = marker.find_at(&mask, search_from) {
        declaration_count += 1;
        if declaration_count > MAX_GDSL_SYMBOLS {
            return Err(CollectorError::TooManySymbols {
                limit: MAX_GDSL_SYMBOLS,
            });
        }

        let open = found.end() - 1;
        let end = matching_delimiter(&mask, open, b'(', b')').ok_or_else(|| {
            CollectorError::MalformedGdsl(format!(
                "unterminated method declaration at line {}",
                line_number(input, found.start())
            ))
        })?;
        search_from = end + 1;

        let declaration = &input[found.end()..end];
        let declaration_mask = &mask[found.end()..end];
        let arguments = parse_named_arguments(declaration, declaration_mask);
        let Some((_, name_value, name_mask)) =
            arguments.iter().find(|(key, _, _)| *key == "name")
        else {
            continue;
        };
        let Some(name) = parse_string_literal(name_value, name_mask) else {
            continue;
        };

        let return_type = arguments
            .iter()
            .find(|(key, _, _)| *key == "type")
            .and_then(|(_, value, value_mask)| parse_string_literal(value, value_mask))
            .unwrap_or_else(|| "Object".into());
        let description = arguments
            .iter()
            .find(|(key, _, _)| *key == "doc" || *key == "description")
            .and_then(|(_, value, value_mask)| parse_string_literal(value, value_mask));
        let documentation_url = description.as_deref().and_then(extract_url);
        let line = line_number(input, found.start());
        let parameters = if let Some((_, value, value_mask)) =
            arguments.iter().find(|(key, _, _)| *key == "params")
        {
            parse_parameters(value, value_mask, line)?
        } else {
            Vec::new()
        };
        let parameter_names = parameters
            .iter()
            .filter_map(|parameter| parameter["name"].as_str())
            .collect::<Vec<_>>();
        let signature = format!(
            "{}({}) -> {}",
            name,
            parameter_names.join(", "),
            return_type
        );

        methods.push(Method {
            name,
            signature,
            description,
            documentation_url,
            parameters,
            line,
        });
    }

    Ok(methods)
}

fn mask_non_code(input: &str) -> Result<String, CollectorError> {
    let bytes = input.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;
    let mut previous_word: Option<&str> = None;
    let mut previous_char: Option<u8> = None;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            let start = index;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            mask_range(&mut masked, start, index);
            continue;
        }

        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let start = index;
            let end = scan_block_comment(bytes, start).ok_or_else(|| {
                CollectorError::MalformedGdsl(format!(
                    "unterminated block comment at line {}",
                    line_number(input, start)
                ))
            })?;
            mask_range(&mut masked, start, end);
            index = end;
            continue;
        }

        if byte == b'$'
            && bytes.get(index + 1) == Some(&b'/')
            && slashy_can_start(previous_word, previous_char)
        {
            let start = index;
            let end = scan_dollar_slashy(bytes, start).ok_or_else(|| {
                CollectorError::MalformedGdsl(format!(
                    "unterminated dollar-slashy string at line {}",
                    line_number(input, start)
                ))
            })?;
            mask_range(&mut masked, start, end);
            index = end;
            previous_word = None;
            previous_char = Some(b'$');
            continue;
        }

        if byte == b'\'' || byte == b'"' {
            let start = index;
            let (end, _, _) = scan_quoted(bytes, start).ok_or_else(|| {
                CollectorError::MalformedGdsl(format!(
                    "unterminated quoted string at line {}",
                    line_number(input, start)
                ))
            })?;
            mask_range(&mut masked, start, end);
            index = end;
            previous_word = None;
            previous_char = Some(byte);
            continue;
        }

        if byte == b'/' && slashy_can_start(previous_word, previous_char) {
            let start = index;
            let end = scan_slashy(bytes, start).ok_or_else(|| {
                CollectorError::MalformedGdsl(format!(
                    "unterminated slashy string at line {}",
                    line_number(input, start)
                ))
            })?;
            mask_range(&mut masked, start, end);
            index = end;
            previous_word = None;
            previous_char = Some(b'/');
            continue;
        }

        if is_identifier_start(byte) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_identifier_continue(bytes[index]) {
                index += 1;
            }
            previous_word = Some(&input[start..index]);
            previous_char = Some(bytes[index - 1]);
            continue;
        }

        previous_word = None;
        previous_char = Some(byte);
        index += 1;
    }

    String::from_utf8(masked)
        .map_err(|_| CollectorError::MalformedGdsl("could not mask GDSL source".into()))
}

fn mask_range(masked: &mut [u8], start: usize, end: usize) {
    for byte in &mut masked[start..end] {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

fn scan_block_comment(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut index = start + 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'/' && bytes[index + 1] == b'*' {
            depth += 1;
            index += 2;
        } else if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Some(index);
            }
        } else {
            index += 1;
        }
    }
    None
}

fn scan_quoted(bytes: &[u8], start: usize) -> Option<(usize, usize, usize)> {
    let quote = *bytes.get(start)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let triple = start + 2 < bytes.len()
        && bytes[start + 1] == quote
        && bytes[start + 2] == quote;
    let delimiter = if triple { 3 } else { 1 };
    let content_start = start + delimiter;
    let mut index = content_start;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if triple {
            if index + 2 < bytes.len()
                && bytes[index] == quote
                && bytes[index + 1] == quote
                && bytes[index + 2] == quote
            {
                return Some((index + 3, content_start, index));
            }
        } else if bytes[index] == quote {
            return Some((index + 1, content_start, index));
        }
        index += 1;
    }

    None
}

fn scan_slashy(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
        } else if bytes[index] == b'/' {
            return Some(index + 1);
        } else {
            index += 1;
        }
    }
    None
}

fn scan_dollar_slashy(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'$'
            && (bytes[index + 1] == b'$' || bytes[index + 1] == b'/')
        {
            index += 2;
        } else if bytes[index] == b'/' && bytes[index + 1] == b'$' {
            return Some(index + 2);
        } else {
            index += 1;
        }
    }
    None
}

fn slashy_can_start(previous_word: Option<&str>, previous_char: Option<u8>) -> bool {
    if previous_char.is_none()
        || matches!(
            previous_char,
            Some(
                b'=' | b'('
                    | b'['
                    | b'{'
                    | b','
                    | b':'
                    | b';'
                    | b'!'
                    | b'?'
                    | b'&'
                    | b'|'
                    | b'~'
                    | b'>'
                    | b'<'
                    | b'+'
                    | b'-'
                    | b'*'
                    | b'%'
                    | b'^'
            )
        )
    {
        return true;
    }

    matches!(
        previous_word,
        Some(
            "return"
                | "throw"
                | "case"
                | "in"
                | "assert"
                | "yield"
                | "new"
                | "else"
                | "do"
                | "instanceof"
                | "as"
        )
    )
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn matching_delimiter(mask: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    if mask.as_bytes().get(open) != Some(&opening) {
        return None;
    }
    let mut depth = 0usize;
    for (index, byte) in mask.as_bytes().iter().enumerate().skip(open) {
        if *byte == opening {
            depth += 1;
        } else if *byte == closing {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn top_level_segments(mask: &str, delimiter: u8) -> Vec<(usize, usize)> {
    let bytes = mask.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;

    for (index, byte) in bytes.iter().copied().enumerate() {
        match byte {
            b'(' => parentheses += 1,
            b')' => parentheses = parentheses.saturating_sub(1),
            b'[' => brackets += 1,
            b']' => brackets = brackets.saturating_sub(1),
            b'{' => braces += 1,
            b'}' => braces = braces.saturating_sub(1),
            _ if byte == delimiter
                && parentheses == 0
                && brackets == 0
                && braces == 0 =>
            {
                segments.push((start, index));
                start = index + 1;
            }
            _ => {}
        }
    }

    segments.push((start, bytes.len()));
    segments
}

fn find_top_level_delimiter(mask: &str, delimiter: u8) -> Option<usize> {
    let bytes = mask.as_bytes();
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;

    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == delimiter && parentheses == 0 && brackets == 0 && braces == 0 {
            return Some(index);
        }
        match byte {
            b'(' => parentheses += 1,
            b')' => parentheses = parentheses.saturating_sub(1),
            b'[' => brackets += 1,
            b']' => brackets = brackets.saturating_sub(1),
            b'{' => braces += 1,
            b'}' => braces = braces.saturating_sub(1),
            _ => {}
        }
    }

    None
}

fn parse_named_arguments<'a>(
    input: &'a str,
    mask: &'a str,
) -> Vec<(&'a str, &'a str, &'a str)> {
    let mut arguments = Vec::new();
    for (start, end) in top_level_segments(mask, b',') {
        let segment_mask = &mask[start..end];
        let Some(colon) = find_top_level_delimiter(segment_mask, b':') else {
            continue;
        };
        let key = segment_mask[..colon].trim();
        if !is_identifier(key) {
            continue;
        }

        let (value_start, value_end) = trim_whitespace(input, start + colon + 1, end);
        arguments.push((
            key,
            &input[value_start..value_end],
            &mask[value_start..value_end],
        ));
    }
    arguments
}

fn trim_whitespace(input: &str, mut start: usize, mut end: usize) -> (usize, usize) {
    let bytes = input.as_bytes();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start, end)
}

fn skip_trivia(input: &str, mut index: usize) -> usize {
    let bytes = input.as_bytes();
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'/' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if index + 1 < bytes.len()
            && bytes[index] == b'/'
            && bytes[index + 1] == b'*'
        {
            index = scan_block_comment(bytes, index).unwrap_or(bytes.len());
        } else {
            return index;
        }
    }
}

fn parse_string_literal(value: &str, mask: &str) -> Option<String> {
    let start = skip_trivia(value, 0);
    let (end, content_start, content_end) = scan_quoted(value.as_bytes(), start)?;
    if !mask.get(end..)?.trim().is_empty() {
        return None;
    }
    Some(unescape(&value[content_start..content_end]))
}

fn parse_parameters(
    value: &str,
    mask: &str,
    line: usize,
) -> Result<Vec<Value>, CollectorError> {
    let start = skip_trivia(value, 0);
    if value.as_bytes().get(start) != Some(&b'[') {
        return Ok(Vec::new());
    }
    let end = matching_delimiter(mask, start, b'[', b']').ok_or_else(|| {
        CollectorError::MalformedGdsl(format!(
            "unterminated params list at line {line}"
        ))
    })?;
    let inner = &value[start + 1..end];
    let inner_mask = &mask[start + 1..end];
    let arguments = parse_named_arguments(inner, inner_mask);
    let mut parameters = Vec::with_capacity(arguments.len().min(MAX_PARAMETERS_PER_SYMBOL));

    for (name, description, description_mask) in arguments {
        if parameters.len() >= MAX_PARAMETERS_PER_SYMBOL {
            return Err(CollectorError::TooManyParameters {
                limit: MAX_PARAMETERS_PER_SYMBOL,
            });
        }
        let mut parameter = json!({ "name": name });
        if let Some(description) = parse_string_literal(description, description_mask) {
            parameter["description"] = json!(description);
        }
        parameters.push(parameter);
    }

    Ok(parameters)
}

fn is_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    is_identifier_start(first) && bytes.all(is_identifier_continue)
}

fn unescape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let Some(escaped) = characters.next() else {
            output.push('\\');
            break;
        };
        match escaped {
            'b' => output.push('\u{0008}'),
            'f' => output.push('\u{000c}'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            '\\' => output.push('\\'),
            '\'' => output.push('\''),
            '"' => output.push('"'),
            '$' => output.push('$'),
            '/' => output.push('/'),
            other => {
                output.push('\\');
                output.push(other);
            }
        }
    }
    output
}

fn extract_url(value: &str) -> Option<String> {
    value.split_whitespace().find(|part| part.starts_with("https://") || part.starts_with("http://"))
        .map(|part| part.trim_end_matches([')', ',', ';']).to_owned())
}
fn line_number(input: &str, offset: usize) -> usize { input[..offset].bytes().filter(|b| *b == b'\n').count() + 1 }
fn stable_id(value: &str) -> String {
    let digest = sha256_hex(value.as_bytes());
    digest[7..23].to_owned()
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
        let status = status.to_owned();
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
    #[test]
    fn collector_sanitizes_nested_secret_metadata_and_url_modifiers() {
        let mut catalog = json!({
            "sources": [{
                "metadata": {
                    "safeLabel": "preserve",
                    "apiKey": "collector-secret-api",
                    "nested": [{"Authorization": "collector-secret-auth"}],
                    "endpoint": "https://collector:collector-secret-user@jenkins.example.test/api?token=collector-secret-token"
                }
            }]
        });
        assert!(sanitize_collected_catalog(&mut catalog) >= 3);
        let serialized = catalog.to_string();
        assert!(!serialized.contains("collector-secret"));
        assert_eq!(catalog["sources"][0]["metadata"]["safeLabel"], "preserve");
        assert_eq!(
            catalog["sources"][0]["metadata"]["endpoint"],
            "https://jenkins.example.test/api"
        );
        assert!(catalog["sources"][0]["metadata"].get("apiKey").is_none());
        assert!(catalog["sources"][0]["metadata"]["nested"][0]
            .get("Authorization")
            .is_none());
    }

    #[test]
    fn ignores_method_like_text_in_comments_and_groovy_string_forms() {
        let input = r#"// method(name: 'line-comment')
/* method(name: 'block-comment') */
/* nested /* method(name: 'nested-comment') */ block */
def single = 'method(name: "single-quoted")'
def double = "method(name: 'double-quoted')"
def triple_single = '''method(name: 'triple-single')'''
def triple_double = """method(name: 'triple-double')"""
def slashy = /method(name: 'slashy')/
def dollar_slashy = $/method(name: 'dollar-slashy')/$
method(name: 'real')
"#;
        let methods = parse_methods(input).unwrap();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "real");
    }

    #[test]
    fn parses_escaped_quotes_and_nested_parameter_delimiters() {
        let input = r#"method(
    name: 'sh',
    type: 'Object',
    params: [
        script: 'Description with \'quoted\', ] and )',
        options: [mode: 'safe', nested: [label: 'a,b:c']],
        after: "tail ] )"
    ],
    doc: """Run with "quoted" values and https://docs.example.test/sh"""
)"#;
        let methods = parse_methods(input).unwrap();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "sh");
        assert_eq!(methods[0].parameters.len(), 3);
        assert_eq!(
            methods[0].parameters[0]["description"],
            "Description with 'quoted', ] and )"
        );
        assert_eq!(methods[0].parameters[1]["name"], "options");
        assert!(methods[0].parameters[1].get("description").is_none());
        assert_eq!(methods[0].parameters[2]["description"], "tail ] )");
        assert_eq!(
            methods[0].description.as_deref(),
            Some("Run with \"quoted\" values and https://docs.example.test/sh")
        );
        assert_eq!(
            methods[0].documentation_url.as_deref(),
            Some("https://docs.example.test/sh")
        );
    }

    #[test]
    fn rejects_unclosed_comments_strings_and_nested_parameter_lists() {
        for input in [
            "/* unclosed comment",
            "def value = 'unclosed string",
            "def value = \"\"\"unclosed triple string",
            "def value = /unclosed slashy",
            "def value = $/unclosed dollar-slashy",
            "method(name: 'sh', params: [script: 'description')",
        ] {
            assert!(
                matches!(parse_methods(input), Err(CollectorError::MalformedGdsl(_))),
                "input should fail safely: {input}"
            );
        }
    }

    #[test]
    fn gdsl_symbol_identity_is_independent_of_declaration_line_numbers() {
        let headers = HeaderMap::new();
        let first = catalog_from_gdsl(
            "https://ci.example.test/jenkins",
            &headers,
            "method(name: 'sh', params: [script: 'script'])",
        ).unwrap();
        let second = catalog_from_gdsl(
            "https://ci.example.test/jenkins",
            &headers,
            "// inserted comment\n\nmethod(name: 'sh', params: [script: 'script'])",
        ).unwrap();
        assert_eq!(first["symbols"][0]["id"], second["symbols"][0]["id"]);
        assert_ne!(first["symbols"][0]["provenance"], second["symbols"][0]["provenance"]);
        assert!(first["sources"][0]["revision"].as_str().unwrap().starts_with("sha256:"));
    }

}
