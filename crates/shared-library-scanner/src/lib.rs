use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct LibraryConfig {
    pub name: String,
    pub repository: String,
    pub reference: String,
}

#[derive(Debug)]
pub struct ScanResult {
    pub catalog: Value,
    pub diagnostics: Vec<String>,
}

#[derive(Debug)]
pub enum ScanError {
    InvalidConfig(String),
    Git(String),
    Io(String),
    InvalidCatalog(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "invalid library configuration: {message}"),
            Self::Git(message) => write!(f, "git operation failed: {message}"),
            Self::Io(message) => write!(f, "source scan failed: {message}"),
            Self::InvalidCatalog(message) => write!(f, "generated catalog is invalid: {message}"),
        }
    }
}
impl std::error::Error for ScanError {}

/// Fetch a configured branch, tag, or commit into an isolated temporary checkout.
/// Git credentials must be provided through the normal Git credential helper/environment;
/// they are never copied into catalog metadata or diagnostics.
pub fn scan_library(config: &LibraryConfig) -> Result<ScanResult, ScanError> {
    validate_config(config)?;
    let temp = tempfile::tempdir().map_err(|e| ScanError::Io(e.to_string()))?;
    run_git(temp.path(), &["init", "--quiet"])?;
    run_git(temp.path(), &["remote", "add", "origin", &config.repository])?;
    run_git(temp.path(), &["fetch", "--quiet", "--depth", "1", "origin", &config.reference])?;
    let commit = git_output(temp.path(), &["rev-parse", "FETCH_HEAD"])?;
    run_git(temp.path(), &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?;
    scan_checkout(config, temp.path(), commit.trim())
}

/// Scan an already-resolved checkout. The caller supplies the resolved commit SHA
/// obtained from Git; it is recorded verbatim for every source snapshot.
pub fn scan_checkout(
    config: &LibraryConfig,
    checkout: &Path,
    resolved_commit: &str,
) -> Result<ScanResult, ScanError> {
    validate_config(config)?;
    if resolved_commit.len() != 40 || !resolved_commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ScanError::InvalidConfig("resolved commit must be a full 40-character Git SHA".into()));
    }

    let source_id = format!("shared-library:{}@{}", slug(&config.name), &resolved_commit[..12]);
    let mut symbols = Vec::new();
    let mut diagnostics = Vec::new();
    let mut files = Vec::new();
    for directory in ["vars", "src"] {
        let root = checkout.join(directory);
        if root.exists() {
            collect_groovy_files(&root, &mut files)?;
        }
    }
    files.sort();

    for file in files {
        let relative = file.strip_prefix(checkout).map_err(|e| ScanError::Io(e.to_string()))?;
        let path = relative.to_string_lossy().replace('\\', "/");
        let content = fs::read_to_string(&file).map_err(|e| ScanError::Io(format!("{}: {e}", file.display())))?;
        let dynamic = has_dynamic_constructs(&content);
        if dynamic {
            diagnostics.push(format!("{path}: dynamic Groovy constructs detected; affected declarations are heuristic"));
        }
        let confidence = if dynamic { "inferred" } else { "verified" };
        if path.starts_with("vars/") {
            scan_global(&path, &content, &source_id, confidence, &mut symbols, &mut diagnostics);
        }
        scan_classes_and_methods(&path, &content, &source_id, confidence, &mut symbols, &mut diagnostics);
    }

    let generated_at = timestamp();
    let catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": format!("shared-library-{}-{}", slug(&config.name), &resolved_commit[..12]),
        "generatedAt": generated_at,
        "sources": [{
            "id": source_id,
            "kind": "shared-library",
            "name": config.name,
            "version": config.reference,
            "revision": resolved_commit,
            "url": config.repository,
            "metadata": { "configuredRef": config.reference, "resolvedCommit": resolved_commit }
        }],
        "symbols": symbols
    });
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
        .map_err(|e| ScanError::InvalidCatalog(e.to_string()))?;
    catalog_validator::validate_catalog(&catalog, &schema)
        .map_err(|e| ScanError::InvalidCatalog(e.to_string()))?;
    Ok(ScanResult { catalog, diagnostics })
}

fn validate_config(config: &LibraryConfig) -> Result<(), ScanError> {
    if config.name.trim().is_empty() || config.repository.trim().is_empty() || config.reference.trim().is_empty() {
        return Err(ScanError::InvalidConfig("name, repository, and reference must be non-empty".into()));
    }
    if config.repository.chars().any(|c| c.is_control()) || config.reference.chars().any(|c| c.is_control()) {
        return Err(ScanError::InvalidConfig("repository and reference must not contain control characters".into()));
    }
    Ok(())
}

fn run_git(directory: &Path, args: &[&str]) -> Result<(), ScanError> {
    let output = Command::new("git").args(args).current_dir(directory).output()
        .map_err(|e| ScanError::Git(e.to_string()))?;
    if output.status.success() { Ok(()) } else {
        Err(ScanError::Git(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
    }
}
fn git_output(directory: &Path, args: &[&str]) -> Result<String, ScanError> {
    let output = Command::new("git").args(args).current_dir(directory).output()
        .map_err(|e| ScanError::Git(e.to_string()))?;
    if output.status.success() { Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned()) }
    else { Err(ScanError::Git(String::from_utf8_lossy(&output.stderr).trim().to_owned())) }
}

fn collect_groovy_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), ScanError> {
    for entry in fs::read_dir(root).map_err(|e| ScanError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| ScanError::Io(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() { collect_groovy_files(&path, output)?; }
        else if path.extension().is_some_and(|ext| ext == "groovy") { output.push(path); }
    }
    Ok(())
}

fn scan_global(path: &str, source: &str, source_id: &str, confidence: &str,
    symbols: &mut Vec<Value>, diagnostics: &mut Vec<String>) {
    let name = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if name.is_empty() { return; }
    let (description, examples) = documentation(source, 0);
    let mut symbol = json!({
        "id": format!("{source_id}:{path}:global"),
        "name": name,
        "kind": "global-variable",
        "sourceId": source_id,
        "description": description.unwrap_or_else(|| format!("Shared Library global defined by {path}")),
        "contexts": ["pipeline"],
        "confidence": confidence,
        "provenance": [format!("{path}:1 (global script)")]
    });
    if !examples.is_empty() { symbol["examples"] = json!(examples); }
    symbols.push(symbol);

    let methods = parse_methods(source);
    for method in methods {
        symbols.push(method_symbol(path, source_id, confidence, method, "method"));
    }
    if source.contains("metaClass") || source.contains("methodMissing") || source.contains("invokeMethod") {
        diagnostics.push(format!("{path}: dynamic dispatch may expose additional global methods that cannot be enumerated statically"));
    }
}

fn scan_classes_and_methods(path: &str, source: &str, source_id: &str, confidence: &str,
    symbols: &mut Vec<Value>, diagnostics: &mut Vec<String>) {
    let mut offset = 0;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("class ").or_else(|| trimmed.strip_prefix("public class "))
            .or_else(|| trimmed.strip_prefix("abstract class ")) {
            let name = rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("");
            if !name.is_empty() {
                let line_no = line_number(source, offset);
                symbols.push(json!({
                    "id": format!("{source_id}:{path}:class:{name}:{line_no}"),
                    "name": name,
                    "kind": "class",
                    "sourceId": source_id,
                    "confidence": confidence,
                    "provenance": [format!("{path}:{line_no}")]
                }));
            }
        }
        offset += line.len() + 1;
    }
    if path.starts_with("src/") {
        for method in parse_methods(source) {
            symbols.push(method_symbol(path, source_id, confidence, method, "method"));
        }
    }
    if source.contains("methodMissing") || source.contains("invokeMethod") || source.contains("metaClass") {
        diagnostics.push(format!("{path}: dynamic method dispatch cannot be fully enumerated"));
    }
}

#[derive(Debug)]
struct Method {
    name: String,
    parameters: Vec<Value>,
    signature: String,
    line: usize,
    description: Option<String>,
    examples: Vec<String>,
}

fn parse_methods(source: &str) -> Vec<Method> {
    let lines: Vec<&str> = source.lines().collect();
    let mut methods = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") { continue; }
        let Some(open) = trimmed.find('(') else { continue };
        let prefix = trimmed[..open].trim();
        if !(prefix.starts_with("def ") || prefix.starts_with("public ") || prefix.starts_with("protected ")
            || prefix.starts_with("private ") || prefix.starts_with("static ")) { continue; }
        let Some(close) = matching_delimiter(trimmed, open, '(', ')') else { continue };
        let before = prefix.split_whitespace().last().unwrap_or("");
        if before.is_empty() || !valid_identifier(before) || ["if", "for", "while", "switch", "catch"].contains(&before) { continue; }
        let args = &trimmed[open + 1..close];
        let parameters = parse_parameters(args);
        let signature = format!("{}({})", before, parameters.iter().map(|p| p["name"].as_str().unwrap_or("?")).collect::<Vec<_>>().join(", "));
        let (description, examples) = documentation(source, index);
        methods.push(Method { name: before.to_owned(), parameters, signature, line: index + 1, description, examples });
    }
    methods
}

fn parse_parameters(args: &str) -> Vec<Value> {
    split_arguments(args).into_iter().filter_map(|arg| {
        let arg = arg.trim();
        if arg.is_empty() || arg == "..." { return None; }
        let clean = arg.split('=').next().unwrap_or("").trim();
        let words: Vec<&str> = clean.split_whitespace().collect();
        let name = words.last().copied().unwrap_or("");
        if !valid_identifier(name) { return None; }
        let mut value = json!({ "name": name });
        if words.len() > 1 { value["type"] = json!(words[..words.len()-1].join(" ")); }
        if arg.contains('=') { value["required"] = json!(false); }
        else { value["required"] = json!(true); }
        Some(value)
    }).collect()
}

fn split_arguments(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut quote = None;
    let mut escaped = false;
    for (i, byte) in input.bytes().enumerate() {
        if let Some(q) = quote {
            if escaped { escaped = false; } else if byte == b'\\' { escaped = true; } else if byte == q { quote = None; }
            continue;
        }
        match byte {
            b'\'' | b'"' -> quote = Some(byte),
            b'(' | b'[' | b'{' -> depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => { parts.push(&input[start..i]); start = i + 1; },
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

fn matching_delimiter(input: &str, open: usize, left: char, right: char) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    let mut escaped = false;
    for (i, ch) in input.char_indices().skip_while(|(i, _)| *i < open) {
        if let Some(q) = quote {
            if escaped { escaped = false; } else if ch == '\\' { escaped = true; } else if ch == q { quote = None; }
        } else if ch == '\'' || ch == '"' { quote = Some(ch); }
        else if ch == left { depth += 1; }
        else if ch == right { depth -= 1; if depth == 0 { return Some(i); } }
    }
    None
}

fn documentation(source: &str, declaration_line: usize) -> (Option<String>, Vec<String>) {
    let lines: Vec<&str> = source.lines().collect();
    if declaration_line >= lines.len() { return (None, Vec::new()); }
    let mut start = declaration_line;
    while start > 0 && (lines[start-1].trim().starts_with('*') || lines[start-1].trim().starts_with("*/")) { start -= 1; }
    if start == declaration_line || !lines[start-1].trim().starts_with("/**") { return (None, Vec::new()); }
    let block = lines[start-1..declaration_line].join("\n");
    let cleaned = block.lines().map(|line| line.trim().trim_start_matches("/**").trim_start_matches("*/").trim_start_matches('*').trim()).collect::<Vec<_>>();
    let description = cleaned.iter().filter(|line| !line.starts_with('@') && !line.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    let examples = cleaned.iter().filter_map(|line| line.strip_prefix("@example ").map(str::to_owned)).collect();
    (if description.is_empty() { None } else { Some(description) }, examples)
}

fn has_dynamic_constructs(source: &str) -> bool {
    ["methodMissing", "invokeMethod", "metaClass", "evaluate(", "GroovyShell", "Expando"]
        .iter().any(|needle| source.contains(needle))
}
fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn method_symbol(path: &str, source_id: &str, confidence: &str, method: Method, kind: &str) -> Value {
    let mut symbol = json!({
        "id": format!("{source_id}:{path}:{}:{}:{}", method.name, method.line, slug(&method.signature)),
        "name": method.name,
        "kind": kind,
        "sourceId": source_id,
        "signature": method.signature,
        "parameters": method.parameters,
        "contexts": ["pipeline"],
        "confidence": confidence,
        "provenance": [format!("{path}:{}", method.line)]
    });
    if let Some(description) = method.description { symbol["description"] = json!(description); }
    if !method.examples.is_empty() { symbol["examples"] = json!(method.examples); }
    symbol
}
fn slug(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() { output.push(ch.to_ascii_lowercase()); }
        else if !output.ends_with('-') { output.push('-'); }
    }
    output.trim_matches('-').to_owned()
}
fn line_number(source: &str, offset: usize) -> usize { source[..offset].bytes().filter(|b| *b == b'\n').count() + 1 }
fn timestamp() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (seconds / 86400) as i64;
    let rem = seconds % 86400;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2)/153;
    let day = doy - (153*mp+2)/5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem/3600, (rem%3600)/60, rem%60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library")
    }
    fn config(reference: &str) -> LibraryConfig {
        LibraryConfig { name: "sample-lib".into(), repository: "https://example.test/sample.git".into(), reference: reference.into() }
    }

    #[test]
    fn scans_vars_global_documentation_and_methods() {
        let result = scan_checkout(&config("main"), &fixture(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        let symbols = result.catalog["symbols"].as_array().unwrap();
        assert!(symbols.iter().any(|s| s["name"] == "deployApp" && s["kind"] == "global-variable"));
        assert!(symbols.iter().any(|s| s["name"] == "call" && s["kind"] == "method"));
        assert!(symbols.iter().any(|s| s["name"] == "call" && s["parameters"].as_array().unwrap().len() == 2));
        assert!(result.catalog["sources"][0]["revision"] == "0123456789abcdef0123456789abcdef01234567");
    }

    #[test]
    fn overloads_have_distinct_ids_and_signatures() {
        let result = scan_checkout(&config("v1"), &fixture(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        let methods: Vec<&Value> = result.catalog["symbols"].as_array().unwrap().iter().filter(|s| s["name"] == "overloaded").collect();
        assert_eq!(methods.len(), 2);
        assert_ne!(methods[0]["id"], methods[1]["id"]);
        assert_ne!(methods[0]["signature"], methods[1]["signature"]);
    }

    #[test]
    fn dynamic_constructs_are_reported_and_inferred() {
        let result = scan_checkout(&config("main"), &fixture(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        assert!(!result.diagnostics.is_empty());
        assert!(result.catalog["symbols"].as_array().unwrap().iter().any(|s| s["confidence"] == "inferred"));
    }

    #[test]
    fn multiple_refs_remain_separate_sources() {
        let a = scan_checkout(&config("release/1"), &fixture(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let b = scan_checkout(&config("release/2"), &fixture(), "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        assert_ne!(a.catalog["sources"][0]["id"], b.catalog["sources"][0]["id"]);
        assert_eq!(a.catalog["sources"][0]["version"], "release/1");
        assert_eq!(b.catalog["sources"][0]["version"], "release/2");
    }

    #[test]
    fn scan_library_records_resolved_git_commit() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        Command::new("git").args(["init", "--quiet"]).current_dir(&repo).status().unwrap();
        std::fs::create_dir(repo.join("vars")).unwrap();
        std::fs::write(repo.join("vars/deploy.groovy"), "def call() { }\n").unwrap();
        Command::new("git").args(["add", "."]).current_dir(&repo).status().unwrap();
        Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@example.test", "commit", "-m", "fixture"]).current_dir(&repo).status().unwrap();
        let result = scan_library(&LibraryConfig { name: "local".into(), repository: repo.to_string_lossy().into_owned(), reference: "HEAD".into() }).unwrap();
        let sha = result.catalog["sources"][0]["revision"].as_str().unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn rejects_short_commit_sha() {
        assert!(matches!(scan_checkout(&config("main"), &fixture(), "abc"), Err(ScanError::InvalidConfig(_))));
    }
}
