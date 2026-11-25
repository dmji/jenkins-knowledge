use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}, process::Command, sync::OnceLock};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig { pub name: String, pub repository: String, pub reference: String }
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanConfig { pub schema_version: String, pub catalog_id: String, pub libraries: Vec<LibraryConfig> }
#[derive(Debug, serde::Serialize)]
pub struct Diagnostic { pub source: String, pub path: String, pub line: usize, pub message: String }
pub struct ScanOutput { pub catalog: Value, pub diagnostics: Vec<Diagnostic> }
#[derive(Debug)]
pub enum ScanError { Io(String), Git(String), Config(String) }
impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Io(s) | Self::Git(s) | Self::Config(s) => f.write_str(s) }
    }
}
impl std::error::Error for ScanError {}

fn run_git(args: &[&str], cwd: Option<&Path>) -> Result<String, ScanError> {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(path) = cwd { command.current_dir(path); }
    let output = command.output().map_err(|_| ScanError::Git("could not execute git; ensure Git is installed".into()))?;
    if !output.status.success() {
        return Err(ScanError::Git(format!("git operation failed ({})", output.status)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn stable_id(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.bytes() { hash ^= u64::from(byte); hash = hash.wrapping_mul(0x100000001b3); }
    format!("{hash:016x}")
}
fn validate_remote(remote: &str) -> Result<(), ScanError> {
    if remote.trim().is_empty() { return Err(ScanError::Config("repository must not be empty".into())); }
    if let Some((scheme, rest)) = remote.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh") { return Err(ScanError::Config("repository URL scheme must be https, http, or ssh".into())); }
        if rest.split('/').next().unwrap_or_default().contains('@') {
            return Err(ScanError::Config("repository URLs must not embed credentials; use Git credential helpers or SSH agents".into()));
        }
    }
    Ok(())
}
fn git_revision(repo: &Path, reference: &str) -> Result<String, ScanError> {
    let rev = format!("{reference}^{{commit}}");
    let remote_branch = format!("refs/remotes/origin/{reference}^{{commit}}");
    let sha = run_git(&["rev-parse", "--verify", &rev], Some(repo))
        .or_else(|_| run_git(&["rev-parse", "--verify", &remote_branch], Some(repo)))
        .or_else(|_| {
            run_git(&["fetch", "--quiet", "origin", reference], Some(repo))?;
            run_git(&["rev-parse", "--verify", "FETCH_HEAD^{commit}"], Some(repo))
        })?;
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ScanError::Git(format!("reference '{reference}' did not resolve to a commit SHA")));
    }
    Ok(sha)
}
pub fn scan(config: ScanConfig) -> Result<ScanOutput, ScanError> {
    if config.schema_version != "1.0.0" { return Err(ScanError::Config("schema_version must be 1.0.0".into())); }
    if config.catalog_id.trim().is_empty() { return Err(ScanError::Config("catalog_id must not be empty".into())); }
    let root = std::env::temp_dir().join(format!("jenkins-library-scan-{}-{}", std::process::id(), stable_id(&format!("{:?}", std::time::SystemTime::now()))));
    fs::create_dir_all(&root).map_err(|e| ScanError::Io(format!("cannot create temporary scan directory: {e}")))?;
    let result = scan_in(&config, &root);
    let _ = fs::remove_dir_all(&root);
    result
}
fn scan_in(config: &ScanConfig, root: &Path) -> Result<ScanOutput, ScanError> {
    let mut sources = Vec::new(); let mut symbols = Vec::new(); let mut diagnostics = Vec::new();
    for (index, library) in config.libraries.iter().enumerate() {
        validate_remote(&library.repository)?;
        let checkout = root.join(format!("library-{index}"));
        run_git(&["clone", "--quiet", "--no-checkout", "--filter=blob:none", "--", &library.repository, checkout.to_str().ok_or_else(|| ScanError::Io("invalid temporary path".into()))?], None)?;
        let sha = git_revision(&checkout, &library.reference)?;
        run_git(&["checkout", "--quiet", "--detach", &sha], Some(&checkout))?;
        let source_id = format!("shared-library-{}", stable_id(&format!("{}\0{}\0{}", library.name, library.repository, library.reference)));
        sources.push(json!({"id":source_id,"kind":"shared-library","name":library.name,"version":library.reference,"revision":sha,"url":library.repository,"metadata":{"repository":library.repository,"configuredRef":library.reference,"resolvedCommitSha":sha}}));
        scan_tree(&checkout, &source_id, &library.name, &mut symbols, &mut diagnostics)?;
    }
    let stamp = Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]).output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| s.len() == 20).unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
    let catalog = json!({"schemaVersion":config.schema_version,"catalogId":config.catalog_id,"generatedAt":stamp,"sources":sources,"symbols":symbols});
    Ok(ScanOutput { catalog, diagnostics })
}
fn declaration_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:(?:public|protected|private|static|final|synchronized)\s+)*(?:def|[A-Za-z_$][\w.$<>?\[\]]*)\s+([A-Za-z_$][\w$]*)\s*\(([^()]*)\)\s*(?:\{|$)").unwrap())
}
fn class_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*(?:public\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)").unwrap())
}
fn clean_code_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") || trimmed.starts_with("'''") || trimmed.starts_with("\"\"\"") { String::new() } else { line.to_owned() }
}
fn parameters(raw: &str) -> Vec<Value> {
    raw.split(',').filter_map(|part| {
        let part = part.trim(); if part.is_empty() { return None; }
        let part = part.split('=').next().unwrap_or(part).trim();
        let mut words = part.split_whitespace(); let first = words.next()?;
        let (ty, name) = match words.next() { Some(n) => (Some(first), n), None => (None, first) };
        if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$') { return None; }
        Some(match ty { Some(t) => json!({"name":name,"type":t}), None => json!({"name":name}) })
    }).collect()
}
fn scan_tree(root: &Path, source_id: &str, library: &str, symbols: &mut Vec<Value>, diagnostics: &mut Vec<Diagnostic>) -> Result<(), ScanError> {
    for base in ["vars", "src"] {
        let dir = root.join(base); if !dir.exists() { continue; }
        let mut files = Vec::new(); collect_groovy(&dir, &mut files)?; files.sort();
        for path in files {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let content = fs::read_to_string(&path).map_err(|e| ScanError::Io(format!("cannot read {relative}: {e}")))?;
            let is_global = base == "vars";
            let global_name = Path::new(&relative).file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
            if is_global {
                let description = fs::read_to_string(path.with_extension("txt")).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
                let mut symbol = json!({"id":format!("{source_id}:{relative}:global"),"name":global_name,"kind":"global-variable","sourceId":source_id,"namespace":library,"confidence":"inferred","provenance":[format!("{relative}:1")]});
                if let Some(description) = description { symbol["description"] = json!(description); }
                symbols.push(symbol);
            }
            let mut class_name: Option<String> = None;
            for (line_no, raw) in content.lines().enumerate() {
                let line = clean_code_line(raw); if line.is_empty() { continue; }
                if let Some(c) = class_regex().captures(&line) {
                    class_name = c.get(1).map(|m| m.as_str().to_owned());
                    if let Some(name) = &class_name { symbols.push(json!({"id":format!("{source_id}:{relative}:class:{name}"),"name":name,"kind":"class","sourceId":source_id,"namespace":library,"confidence":"inferred","provenance":[format!("{relative}:{}",line_no+1)]})); }
                    continue;
                }
                if let Some(c) = declaration_regex().captures(&line) {
                    let name = c.get(1).map(|m| m.as_str()).unwrap_or_default();
                    if ["methodMissing", "propertyMissing", "invokeMethod"].contains(&name) {
                        diagnostics.push(Diagnostic { source: source_id.into(), path: relative.clone(), line: line_no + 1, message: "dynamic Groovy dispatch is not statically resolved".into() });
                        continue;
                    }
                    if ["if","for","while","switch","catch","return","assert","new"].contains(&name) { continue; }
                    let raw_params = c.get(2).map(|m| m.as_str()).unwrap_or_default();
                    let scope = class_name.as_deref().unwrap_or(global_name);
                    symbols.push(json!({"id":format!("{source_id}:{relative}:{name}:{line_no}"),"name":name,"kind":"method","sourceId":source_id,"namespace":format!("{library}.{scope}"),"signature":format!("{}({})",name,raw_params.trim()),"parameters":parameters(raw_params),"confidence":"inferred","provenance":[format!("{relative}:{}",line_no+1)]}));
                } else if ["evaluate(","methodMissing(","propertyMissing(","invokeMethod("].iter().any(|s| line.contains(s)) {
                    diagnostics.push(Diagnostic { source:source_id.into(),path:relative.clone(),line:line_no+1,message:"dynamic Groovy dispatch is not statically resolved".into() });
                }
            }
        }
    }
    Ok(())
}
fn collect_groovy(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), ScanError> {
    for entry in fs::read_dir(dir).map_err(|e| ScanError::Io(format!("cannot read source directory: {e}")))? {
        let path = entry.map_err(|e| ScanError::Io(format!("cannot read directory entry: {e}")))?.path();
        if path.is_dir() { collect_groovy(&path, files)?; }
        else if path.extension().is_some_and(|ext| ext == "groovy") { files.push(path); }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parses_typed_dynamic_parameters_and_overloads() {
        assert_eq!(parameters("String name, Map options = [:], count").len(), 3);
        let source = "def deploy(String name) {\n}\ndef deploy(Map config) {\n}\n";
        assert_eq!(source.lines().filter(|l| declaration_regex().is_match(l)).count(), 2);
    }
    #[test] fn ignores_comment_lines() {
        assert!(clean_code_line(" // def fake() {}").is_empty());
        assert!(declaration_regex().captures(&clean_code_line(" // def fake() {}")).is_none());
    }
    #[test] fn rejects_embedded_credentials() {
        assert!(validate_remote("https://user:token@example.com/repo.git").is_err());
        assert!(validate_remote("https://example.com/repo.git").is_ok());
    }
    #[test] fn refs_get_distinct_source_ids() {
        assert_ne!(stable_id("lib\0main\0v1"), stable_id("lib\0main\0v2"));
    }
}
