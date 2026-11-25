use serde::Serialize;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub source_id: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ScanResult {
    pub catalog: Value,
    pub diagnostics: Vec<Diagnostic>,
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

    let source_id = format!("shared-library:{}:{}@{}", slug(&config.name), slug(&config.reference), &resolved_commit[..12]);
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
            push_diagnostic(
                &mut diagnostics,
                &source_id,
                &path,
                "dynamic-groovy-constructs",
                "Dynamic Groovy constructs detected; affected declarations are heuristic.",
            );
        }
        let confidence = if dynamic { "inferred" } else { "verified" };
        if path.starts_with("vars/") {
            scan_global(
                &file,
                &path,
                &content,
                &source_id,
                confidence,
                &mut symbols,
                &mut diagnostics,
            )?;
        }
        scan_classes_and_methods(&path, &content, &source_id, confidence, &mut symbols, &mut diagnostics);
    }

    let generated_at = timestamp();
    let catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": format!("shared-library-{}-{}-{}", slug(&config.name), slug(&config.reference), &resolved_commit[..12]),
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
    if config.repository.starts_with("http://") || config.repository.starts_with("https://") {
        let authority = config.repository.split_once("://").map(|(_, rest)| rest.split('/').next().unwrap_or("")).unwrap_or("");
        if authority.contains('@') || config.repository.chars().any(|c| c == '?' || c == '#') {
            return Err(ScanError::InvalidConfig("HTTP repository URLs must not contain credentials, query parameters, or fragments".into()));
        }
    }
    Ok(())
}

fn run_git(directory: &Path, args: &[&str]) -> Result<(), ScanError> {
    let output = Command::new("git").args(args).current_dir(directory).output()
        .map_err(|e| ScanError::Git(e.to_string()))?;
    if output.status.success() { Ok(()) } else {
        Err(ScanError::Git("git command failed; remote details are redacted".into()))
    }
}
fn git_output(directory: &Path, args: &[&str]) -> Result<String, ScanError> {
    let output = Command::new("git").args(args).current_dir(directory).output()
        .map_err(|e| ScanError::Git(e.to_string()))?;
    if output.status.success() { Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned()) }
    else { Err(ScanError::Git("git command failed; remote details are redacted".into())) }
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

fn scan_global(
    file: &Path,
    path: &str,
    source: &str,
    source_id: &str,
    confidence: &str,
    symbols: &mut Vec<Value>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), ScanError> {
    let name = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if name.is_empty() {
        return Ok(());
    }
    let declaration_line = source.lines().position(|line| {
        let line = line.trim();
        line.starts_with("def ") || line.starts_with("public ") || line.starts_with("protected ")
            || line.starts_with("private ") || line.starts_with("static ")
    }).unwrap_or(0);
    let (groovydoc_description, examples) = documentation(source, declaration_line);
    let text_description = read_adjacent_documentation(file)?;
    let description = groovydoc_description.or(text_description);
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
    if !examples.is_empty() {
        symbol["examples"] = json!(examples);
    }
    symbols.push(symbol);

    let methods = parse_methods(source);
    for method in methods {
        symbols.push(method_symbol(path, source_id, confidence, method, "method", Some(name)));
    }
    if source.contains("metaClass") || source.contains("methodMissing") || source.contains("invokeMethod") {
        push_diagnostic(
            diagnostics,
            source_id,
            path,
            "dynamic-global-methods",
            "Dynamic dispatch may expose additional global methods that cannot be enumerated.",
        );
    }
    Ok(())
}

fn read_adjacent_documentation(groovy_file: &Path) -> Result<Option<String>, ScanError> {
    let documentation_file = groovy_file.with_extension("txt");
    match fs::read_to_string(&documentation_file) {
        Ok(content) => {
            let content = content.trim();
            if content.is_empty() {
                Ok(None)
            } else {
                Ok(Some(content.to_owned()))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ScanError::Io(format!("{}: {error}", documentation_file.display()))),
    }
}

fn push_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    source_id: &str,
    path: &str,
    code: &str,
    message: &str,
) {
    diagnostics.push(Diagnostic {
        code: code.to_owned(),
        severity: "warning".to_owned(),
        source_id: source_id.to_owned(),
        path: path.to_owned(),
        message: message.to_owned(),
    });
}

fn scan_classes_and_methods(path: &str, source: &str, source_id: &str, confidence: &str,
    symbols: &mut Vec<Value>, diagnostics: &mut Vec<Diagnostic>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut depth = 0i32;
    let mut owner: Option<String> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some((name, signature)) = parse_class_declaration(trimmed) {
            let line_no = index + 1;
            let mut symbol = json!({
                "id": format!("{source_id}:{path}:class:{name}:{line_no}"),
                "name": name, "kind": "class", "sourceId": source_id,
                "confidence": confidence, "provenance": [format!("{path}:{line_no}")]
            });
            if !signature.is_empty() { symbol["signature"] = json!(signature); }
            symbols.push(symbol);
            owner = Some(name);
            depth = brace_delta(line);
        } else if owner.is_some() {
            depth += brace_delta(line);
            if depth <= 0 { owner = None; depth = 0; }
            else if path.starts_with("src/") && !trimmed.is_empty() && !trimmed.starts_with('@') {
                if let Some((name, ty)) = parse_property_declaration(trimmed) {
                    let class_name = owner.as_deref().unwrap_or_default();
                    symbols.push(json!({
                        "id": format!("{source_id}:{path}:property:{class_name}:{name}:{}", index + 1),
                        "name": name, "kind": "property", "sourceId": source_id,
                        "namespace": class_name, "signature": ty, "confidence": confidence,
                        "provenance": [format!("{path}:{}", index + 1)]
                    }));
                }
            }
        }
    }
    if path.starts_with("src/") {
        for method in parse_methods(source) {
            let owner = method.owner.clone();
            symbols.push(method_symbol(path, source_id, confidence, method, "method", owner.as_deref()));
        }
    }
    diagnose_unsupported_syntax(path, source, source_id, diagnostics);
    if source.contains("methodMissing") || source.contains("invokeMethod") || source.contains("metaClass") {
        push_diagnostic(diagnostics, source_id, path, "dynamic-method-dispatch",
            "Dynamic method dispatch cannot be fully enumerated.");
    }
}

fn parse_class_declaration(line: &str) -> Option<(String, String)> {
    let clean = line.trim_start_matches("public ").trim_start_matches("abstract ")
        .trim_start_matches("final ").trim_start_matches("static ");
    let rest = clean.strip_prefix("class ")?;
    let name = rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next()?;
    if !valid_identifier(name) { return None; }
    Some((name.to_owned(), rest.trim().trim_end_matches('{').trim().to_owned()))
}

fn parse_property_declaration(line: &str) -> Option<(String, String)> {
    if line.contains('(') || line.starts_with("//") || line.starts_with("/*") || line.starts_with('*')
        || line.starts_with("return ") || line.starts_with("throw ") || line.contains("->") { return None; }
    let declaration = line.trim_end_matches(';').split('=').next()?.trim();
    let mut words: Vec<&str> = declaration.split_whitespace().collect();
    while words.first().is_some_and(|w| matches!(*w,
        "public" | "protected" | "private" | "static" | "final" | "transient" | "volatile")) {
        words.remove(0);
    }
    if words.len() < 2 { return None; }
    let name = words[words.len()-1].trim_end_matches('?');
    if !valid_identifier(name) { return None; }
    let ty = words[..words.len()-1].join(" ");
    if ty.is_empty() || ["return","new","if","else","while","for","switch"].contains(&ty.as_str()) { return None; }
    Some((name.to_owned(), ty))
}

fn brace_delta(line: &str) -> i32 {
    line.chars().fold(0, |n, c| match c { '{' => n+1, '}' => n-1, _ => n })
}

fn diagnose_unsupported_syntax(path: &str, source: &str, source_id: &str, diagnostics: &mut Vec<Diagnostic>) {
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if ["trait ", "enum ", "record ", "@interface", "sealed class ", "def (", "def ["]
            .iter().any(|needle| trimmed.starts_with(needle)) {
            push_diagnostic(diagnostics, source_id, path, "unsupported-groovy-syntax",
                &format!("Unsupported declaration form at line {}; no symbol was inferred from it.", index + 1));
        }
    }
}

#[derive(Debug)]
struct Method {
    name: String, parameters: Vec<Value>, signature: String, line: usize,
    description: Option<String>, examples: Vec<String>, owner: Option<String>,
}

fn parse_methods(source: &str) -> Vec<Method> {
    let lines: Vec<&str> = source.lines().collect();
    let mut result = Vec::new();
    let mut owner: Option<String> = None;
    let mut depth = 0i32;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if let Some((name, _)) = parse_class_declaration(trimmed) {
            owner = Some(name);
            depth = brace_delta(line);
        } else if owner.is_some() {
            depth += brace_delta(line);
            if depth <= 0 { owner = None; depth = 0; }
        }
        if is_method_declaration_start(trimmed) {
            let start = index;
            let mut declaration = trimmed.to_owned();
            while !has_balanced_parentheses(&declaration) && index + 1 < lines.len() {
                index += 1;
                declaration.push(' ');
                declaration.push_str(lines[index].trim());
            }
            if let Some(method) = parse_method_declaration(&declaration, start + 1, owner.clone()) {
                result.push(method);
            }
        }
        index += 1;
    }
    result
}

fn is_method_declaration_start(line: &str) -> bool {
    if line.starts_with('@') || line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") { return false; }
    let Some(open) = line.find('(') else { return false };
    let prefix = line[..open].trim();
    let name = prefix.split_whitespace().last().unwrap_or("");
    valid_identifier(name) && !["if","for","while","switch","catch","synchronized","assert"].contains(&name)
        && (prefix.starts_with("def ") || prefix.split_whitespace().count() >= 2)
}

fn has_balanced_parentheses(text: &str) -> bool {
    let Some(open) = text.find('(') else { return true };
    matching_delimiter(text, open, '(', ')').is_some()
}

fn parse_method_declaration(text: &str, line: usize, owner: Option<String>) -> Option<Method> {
    let open = text.find('(')?;
    let close = matching_delimiter(text, open, '(', ')')?;
    let prefix = text[..open].trim();
    let name = prefix.split_whitespace().last()?;
    if !valid_identifier(name) || ["if","for","while","switch","catch"].contains(&name) { return None; }
    let parameters = parse_parameters(&text[open+1..close]);
    let signature = format!("{}({})", name, parameters.iter()
        .map(|p| p["name"].as_str().unwrap_or("?")).collect::<Vec<_>>().join(", "));
    Some(Method { name: name.to_owned(), parameters, signature, line, description: None,
        examples: Vec::new(), owner })
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
            b'\'' | b'"' => quote = Some(byte),
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
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
    if declaration_line == 0 || declaration_line >= lines.len() { return (None, Vec::new()); }
    let previous = lines[declaration_line - 1].trim();
    if previous.starts_with("/**") && previous.ends_with("*/") {
        let text = previous.trim_start_matches("/**").trim_end_matches("*/").trim();
        return documentation_fields(vec![text.to_owned()]);
    }
    let mut start = declaration_line;
    while start > 0 && (lines[start - 1].trim().starts_with('*') || lines[start - 1].trim().starts_with("*/")) {
        start -= 1;
    }
    if start == declaration_line || !lines[start - 1].trim().starts_with("/**") {
        return (None, Vec::new());
    }
    let block = lines[start - 1..declaration_line].join("\n");
    let cleaned = block.lines().map(|line| {
        line.trim().trim_start_matches("/**").trim_start_matches("*/").trim_start_matches('*').trim().to_owned()
    }).collect();
    documentation_fields(cleaned)
}

fn documentation_fields(lines: Vec<String>) -> (Option<String>, Vec<String>) {
    let description = lines.iter().filter(|line| !line.starts_with('@') && !line.is_empty())
        .cloned().collect::<Vec<_>>().join(" ");
    let examples = lines.iter().filter_map(|line| line.strip_prefix("@example ").map(str::to_owned)).collect();
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
fn method_symbol(path: &str, source_id: &str, confidence: &str, method: Method, kind: &str, owner: Option<&str>) -> Value {
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
    if let Some(owner) = owner { symbol["namespace"] = json!(owner); }
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
        assert!(symbols.iter().any(|s| s["kind"] == "global-variable" && s["description"] == "Deploy an application to an environment."));
        assert!(symbols.iter().any(|s| s["kind"] == "global-variable" && s["examples"][0] == "deployApp([environment: 'staging'])"));
        assert!(symbols.iter().any(|s| s["kind"] == "class" && s["name"] == "Deployment"));
        assert!(symbols.iter().any(|s| s["kind"] == "method" && s["name"] == "deploy" && s["description"] == "Deploy using the supplied name."));
        assert!(symbols.iter().any(|s| s["name"] == "call" && s["kind"] == "method"));
        assert!(symbols.iter().any(|s| s["name"] == "call" && s["parameters"].as_array().unwrap().len() == 2));
        assert!(result.catalog["sources"][0]["revision"] == "0123456789abcdef0123456789abcdef01234567");
    }

    #[test]
    fn groovydoc_takes_precedence_over_adjacent_text_documentation() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp.path().join("vars");
        fs::create_dir_all(&vars).unwrap();
        fs::write(
            vars.join("deploy.groovy"),
            "/**\n * Groovydoc description.\n */\ndef call() { }\n",
        ).unwrap();
        fs::write(vars.join("deploy.txt"), "Fallback text-file description.").unwrap();

        let result = scan_checkout(
            &config("main"),
            temp.path(),
            "0123456789abcdef0123456789abcdef01234567",
        ).unwrap();
        let global = result.catalog["symbols"].as_array().unwrap().iter()
            .find(|symbol| symbol["kind"] == "global-variable")
            .unwrap();

        assert_eq!(global["description"], "Groovydoc description.");
    }

    #[test]
    fn adjacent_text_documentation_is_used_when_groovydoc_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp.path().join("vars");
        fs::create_dir_all(&vars).unwrap();
        fs::write(vars.join("releaseNotes.groovy"), "def call() { }\n").unwrap();
        fs::write(
            vars.join("releaseNotes.txt"),
            "  Release notes helper.\n\nIncludes rollback guidance. \n",
        ).unwrap();

        let result = scan_checkout(
            &config("main"),
            temp.path(),
            "0123456789abcdef0123456789abcdef01234567",
        ).unwrap();
        let global = result.catalog["symbols"].as_array().unwrap().iter()
            .find(|symbol| symbol["kind"] == "global-variable")
            .unwrap();

        assert_eq!(
            global["description"],
            "Release notes helper.\n\nIncludes rollback guidance.",
        );
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
    fn same_commit_with_different_refs_remains_distinct() {
        let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let main = scan_checkout(&config("main"), &fixture(), sha).unwrap();
        let release = scan_checkout(&config("release/1"), &fixture(), sha).unwrap();
        assert_ne!(main.catalog["sources"][0]["id"], release.catalog["sources"][0]["id"]);
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
    fn rejects_repository_urls_that_could_leak_credentials() {
        let config = LibraryConfig {
            name: "unsafe".into(),
            repository: "https://user:token@example.test/library.git".into(),
            reference: "main".into(),
        };
        assert!(matches!(scan_checkout(&config, &fixture(), "0123456789abcdef0123456789abcdef01234567"), Err(ScanError::InvalidConfig(_))));
    }

    #[test]
    fn parses_multiline_annotated_generic_methods_and_class_members() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src/example");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("Service.groovy"), r#"
package example
class Service<T> extends BaseService implements Runnable, Closeable {
    @Deprecated
    private Map<String, List<Integer>> cache
    @Override
    public List<Result> deploy(
        Map<String, List<Integer>> options,
        Closure<Result> callback
    ) { null }
    protected String overloaded(String name) { name }
    protected String overloaded(String name, int retries) { name }
}
"#).unwrap();
        let result = scan_checkout(&config("main"), temp.path(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        let symbols = result.catalog["symbols"].as_array().unwrap();
        let class = symbols.iter().find(|s| s["kind"] == "class").unwrap();
        assert!(class["signature"].as_str().unwrap().contains("extends BaseService"));
        assert!(class["signature"].as_str().unwrap().contains("implements Runnable, Closeable"));
        let property = symbols.iter().find(|s| s["kind"] == "property").unwrap();
        assert_eq!(property["name"], "cache");
        assert_eq!(property["namespace"], "Service");
        assert!(property["signature"].as_str().unwrap().contains("Map<String, List<Integer>>"));
        let deploy = symbols.iter().find(|s| s["name"] == "deploy").unwrap();
        assert_eq!(deploy["namespace"], "Service");
        assert_eq!(deploy["provenance"][0], "src/example/Service.groovy:7");
        assert_eq!(deploy["parameters"].as_array().unwrap().len(), 2);
        assert!(deploy["parameters"][0]["type"].as_str().unwrap().contains("Map<String, List<Integer>>"));
        assert_eq!(symbols.iter().filter(|s| s["name"] == "overloaded").count(), 2);
    }

    #[test]
    fn reports_unsupported_declarations_without_fabricating_symbols() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp.path().join("vars");
        fs::create_dir_all(&vars).unwrap();
        fs::write(vars.join("example.groovy"), "trait DynamicThing { }\nenum State { READY }\n").unwrap();
        let result = scan_checkout(&config("main"), temp.path(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        assert!(result.diagnostics.iter().filter(|d| d.code == "unsupported-groovy-syntax").count() >= 2);
        assert!(!result.catalog["symbols"].as_array().unwrap().iter().any(|s| s["kind"] == "class"));
    }

    #[test]
    fn records_global_method_ownership() {
        let result = scan_checkout(&config("main"), &fixture(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        let helper = result.catalog["symbols"].as_array().unwrap().iter().find(|s| s["name"] == "helper").unwrap();
        assert_eq!(helper["namespace"], "deployApp");
    }

    #[test]
    fn rejects_short_commit_sha() {
        assert!(matches!(scan_checkout(&config("main"), &fixture(), "abc"), Err(ScanError::InvalidConfig(_))));
    }
}
