use catalog_validator::{redact_sensitive_fields, stamp_catalog_identity};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
pub mod source_security;
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
    scan_library_at(config, &config.reference)
}

/// Fetch a specific branch, tag, or commit while retaining the configured ref in metadata.
pub fn scan_library_at(config: &LibraryConfig, fetch_ref: &str) -> Result<ScanResult, ScanError> {
    validate_config(config)?;
    validate_git_reference(fetch_ref).map_err(ScanError::InvalidConfig)?;
    let checkout_budget = git_checkout_budget_from_environment()?;
    let (checkout, commit) = fetch_checkout(&config.repository, fetch_ref, checkout_budget)?;
    scan_checkout(config, checkout.path(), &commit)
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
    let limits = ScanLimits::from_environment()?;
    scan_checkout_with_limits(config, checkout, resolved_commit, limits)
}

fn scan_checkout_with_limits(
    config: &LibraryConfig,
    checkout: &Path,
    resolved_commit: &str,
    limits: ScanLimits,
) -> Result<ScanResult, ScanError> {
    let source_identity = format!("{}\n{}\n{}", config.name, config.repository, config.reference);
    let identity_hash = catalog_validator::sha256_hex(source_identity.as_bytes());
    let source_id = format!("shared-library:{}:{}", slug(&config.name), &identity_hash[7..23]);
    let mut symbols = Vec::new();
    let mut diagnostics = Vec::new();
    let canonical_root = checkout.canonicalize().map_err(|e| ScanError::Io(format!("cannot resolve checkout root: {e}")))?;
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    let mut entries = 0usize;
    for directory in ["vars", "src"] {
        let root = canonical_root.join(directory);
        if root.exists() {
            collect_groovy_files(
                &root,
                &canonical_root,
                0,
                &mut files,
                &mut total_bytes,
                &mut entries,
                &limits,
            )?;
        }
    }
    files.sort();

    for file in files {
        let relative = file.strip_prefix(&canonical_root).map_err(|e| ScanError::Io(e.to_string()))?;
        let path = relative.to_string_lossy().replace('\\', "/");
        let content = fs::read_to_string(&file)
            .map_err(|_| ScanError::Io(format!("cannot read source file {path}")))?;
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
        if symbols.len() > limits.symbols {
            return Err(ScanError::Io(format!(
                "generated symbol count exceeds configured limit ({})",
                limits.symbols
            )));
        }
    }

    let generated_at = timestamp();
    let mut catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": format!("shared-library-{}-{}", slug(&config.name), &identity_hash[7..23]),
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
    redact_sensitive_fields(&mut catalog);
    stamp_catalog_identity(&mut catalog).map_err(ScanError::InvalidCatalog)?;
    let catalog_size = serde_json::to_vec_pretty(&catalog)
        .map_err(|_| ScanError::InvalidCatalog("cannot serialize generated catalog".into()))?
        .len();
    if catalog_size > limits.catalog_bytes {
        return Err(ScanError::InvalidCatalog(format!(
            "generated catalog exceeds configured size limit ({} bytes)",
            limits.catalog_bytes
        )));
    }
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
        .map_err(|e| ScanError::InvalidCatalog(e.to_string()))?;
    catalog_validator::validate_catalog(&catalog, &schema)
        .map_err(|e| ScanError::InvalidCatalog(e.to_string()))?;
    Ok(ScanResult { catalog, diagnostics })
}

pub fn validate_git_url(value: &str) -> Result<(), String> { crate::source_security::validate_git_url(value) }
pub fn validate_git_reference(value: &str) -> Result<(), String> { crate::source_security::validate_git_reference(value) }

fn validate_config(config: &LibraryConfig) -> Result<(), ScanError> {
    if config.name.trim().is_empty() || config.repository.trim().is_empty() || config.reference.trim().is_empty() {
        return Err(ScanError::InvalidConfig("name, repository, and reference must be non-empty".into()));
    }
    validate_git_url(&config.repository).map_err(ScanError::InvalidConfig)?;
    validate_git_reference(&config.reference).map_err(ScanError::InvalidConfig)?;
    Ok(())
}

const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_GIT_CHECKOUT_BYTES: u64 = 256 * 1024 * 1024;

fn git_checkout_budget_from_environment() -> Result<u64, ScanError> {
    const NAME: &str = "JENKINS_KNOWLEDGE_GIT_MAX_CHECKOUT_BYTES";
    match std::env::var(NAME) {
        Ok(value) => value.parse::<u64>().ok().filter(|limit| *limit > 0)
            .ok_or_else(|| ScanError::InvalidConfig(format!("{NAME} must be a positive integer"))),
        Err(_) => Ok(DEFAULT_GIT_CHECKOUT_BYTES),
    }
}

/// A unique, unprivileged temporary checkout. Git is monitored while every
/// command runs; Unix children additionally inherit an RLIMIT_FSIZE ceiling.
struct GitCheckout {
    temp: tempfile::TempDir,
    limit: u64,
}

impl GitCheckout {
    fn new(limit: u64) -> Result<Self, ScanError> {
        if limit == 0 {
            return Err(ScanError::InvalidConfig(
                "Git checkout storage budget must be a positive integer".into(),
            ));
        }
        let temp = tempfile::Builder::new()
            .prefix("jenkins-knowledge-git-")
            .tempdir()
            .map_err(|_| ScanError::Io("cannot create temporary Git checkout".into()))?;
        Ok(Self { temp, limit })
    }

    fn path(&self) -> &Path {
        self.temp.path()
    }
}

fn fetch_checkout(
    repository: &str,
    fetch_ref: &str,
    limit: u64,
) -> Result<(GitCheckout, String), ScanError> {
    let checkout = GitCheckout::new(limit)?;
    run_git(checkout.path(), &["init", "--quiet"], checkout.limit)?;
    run_git(
        checkout.path(),
        &["remote", "add", "origin", repository],
        checkout.limit,
    )?;
    run_git(
        checkout.path(),
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-recurse-submodules",
            "--depth",
            "1",
            "--filter=blob:limit=2m",
            "origin",
            fetch_ref,
        ],
        checkout.limit,
    )?;
    let commit = git_output(checkout.path(), &["rev-parse", "FETCH_HEAD"], checkout.limit)?;
    run_git(
        checkout.path(),
        &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
        checkout.limit,
    )?;
    Ok((checkout, commit))
}

const MAX_CHECKOUT_ENTRIES: usize = 100_000;

/// Measure regular-file bytes without following symlinks. Stop as soon as the
/// budget is exceeded so an attacker cannot force an unbounded accounting pass.
fn checkout_size(root: &Path, limit: u64) -> std::io::Result<u64> {
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0u64;
    let mut entries = 0usize;
    while let Some(directory) = pending.pop() {
        let directory_entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in directory_entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            entries += 1;
            if entries > MAX_CHECKOUT_ENTRIES {
                return Ok(limit.saturating_add(1));
            }
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                // Git creates and atomically renames pack/index files while the
                // watchdog walks the directory. A vanished entry is not a quota
                // accounting failure; the next poll observes the stable state.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
                if total > limit {
                    return Ok(total);
                }
            }
        }
    }
    Ok(total)
}

fn run_git(directory: &Path, args: &[&str], limit: u64) -> Result<(), ScanError> {
    run_git_inner(directory, args, limit).map(|_| ())
}
fn git_output(directory: &Path, args: &[&str], limit: u64) -> Result<String, ScanError> {
    run_git_inner(directory, args, limit)
}

fn terminate_git(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let _ = Command::new("/bin/kill").args(["-KILL", &group]).status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn run_git_inner(directory: &Path, args: &[&str], limit: u64) -> Result<String, ScanError> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        unsafe {
            command.pre_exec(move || {
                let file_limit = libc::rlimit {
                    rlim_cur: limit as libc::rlim_t,
                    rlim_max: limit as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &file_limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| ScanError::Git("cannot start git".into()))?;
    let started = std::time::Instant::now();
    loop {
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(_) => {
                terminate_git(&mut child);
                return Err(ScanError::Git("cannot wait for git".into()));
            }
        };
        match status {
            Some(status) => {
                let size = checkout_size(directory, limit)
                    .map_err(|_| ScanError::Git("cannot account for temporary checkout storage".into()))?;
                if size > limit {
                    terminate_git(&mut child);
                    return Err(ScanError::Git("Git checkout exceeded its configured storage budget".into()));
                }
                if !status.success() {
                    terminate_git(&mut child);
                    return Err(ScanError::Git("git command failed; remote details are redacted".into()));
                }
                let mut output = String::new();
                if let Some(stdout) = child.stdout.take() {
                    use std::io::Read;
                    stdout
                        .take(4096)
                        .read_to_string(&mut output)
                        .map_err(|_| ScanError::Git("cannot read git output".into()))?;
                }
                return Ok(output.trim().to_owned());
            }
            None if started.elapsed() >= GIT_TIMEOUT => {
                terminate_git(&mut child);
                return Err(ScanError::Git(
                    "git command timed out after 120 seconds; child process tree terminated".into(),
                ));
            }
            None => {
                match checkout_size(directory, limit) {
                    Ok(size) if size <= limit => {}
                    Ok(_) => {
                        terminate_git(&mut child);
                        return Err(ScanError::Git(
                            "Git checkout exceeded its configured storage budget; process tree terminated".into(),
                        ));
                    }
                    Err(_) => {
                        terminate_git(&mut child);
                        return Err(ScanError::Git(
                            "cannot account for temporary checkout storage; Git process tree terminated".into(),
                        ));
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ScanLimits {
    depth: usize,
    files: usize,
    entries: usize,
    file_bytes: u64,
    total_bytes: u64,
    symbols: usize,
    catalog_bytes: usize,
}
impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            depth: 32,
            files: 10_000,
            entries: 100_000,
            file_bytes: 1_048_576,
            total_bytes: 33_554_432,
            symbols: 50_000,
            catalog_bytes: 67_108_864,
        }
    }
}

impl ScanLimits {
    fn from_environment() -> Result<Self, ScanError> {
        fn limit(name: &str, default: u64) -> Result<u64, ScanError> {
            match std::env::var(name) {
                Ok(value) => value.parse().ok().filter(|v| *v > 0)
                    .ok_or_else(|| ScanError::InvalidConfig(format!("{name} must be a positive integer"))),
                Err(_) => Ok(default),
            }
        }
        Ok(Self {
            depth: limit("JENKINS_KNOWLEDGE_SCAN_MAX_DEPTH", 32)? as usize,
            files: limit("JENKINS_KNOWLEDGE_SCAN_MAX_FILES", 10000)? as usize,
            entries: limit("JENKINS_KNOWLEDGE_SCAN_MAX_ENTRIES", 100000)? as usize,
            file_bytes: limit("JENKINS_KNOWLEDGE_SCAN_MAX_FILE_BYTES", 1048576)?,
            total_bytes: limit("JENKINS_KNOWLEDGE_SCAN_MAX_TOTAL_BYTES", 33554432)?,
            symbols: limit("JENKINS_KNOWLEDGE_SCAN_MAX_SYMBOLS", 50000)? as usize,
            catalog_bytes: limit("JENKINS_KNOWLEDGE_SCAN_MAX_CATALOG_BYTES", 67108864)? as usize,
        })
    }
}

fn collect_groovy_files(
    root: &Path,
    checkout: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
    total_bytes: &mut u64,
    entry_count: &mut usize,
    limits: &ScanLimits,
) -> Result<(), ScanError> {
    if depth > limits.depth {
        return Err(ScanError::Io(format!("scan depth exceeds limit ({})", limits.depth)));
    }
    let metadata = fs::symlink_metadata(root)
        .map_err(|e| ScanError::Io(format!("cannot inspect directory: {e}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| ScanError::Io(format!("cannot resolve directory: {e}")))?;
    if !canonical_root.starts_with(checkout) {
        return Err(ScanError::Io("directory escaped checkout root".into()));
    }
    for entry in fs::read_dir(&canonical_root)
        .map_err(|e| ScanError::Io(format!("cannot read directory: {e}")))?
    {
        *entry_count = entry_count.saturating_add(1);
        if *entry_count > limits.entries {
            return Err(ScanError::Io(format!(
                "filesystem entry count exceeds limit ({})",
                limits.entries
            )));
        }
        let entry = entry.map_err(|e| ScanError::Io(format!("cannot read directory entry: {e}")))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| ScanError::Io(format!("cannot inspect path: {e}")))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let canonical = path
                .canonicalize()
                .map_err(|e| ScanError::Io(format!("cannot resolve directory: {e}")))?;
            if !canonical.starts_with(checkout) {
                return Err(ScanError::Io("directory escaped checkout root".into()));
            }
            collect_groovy_files(
                &canonical,
                checkout,
                depth + 1,
                output,
                total_bytes,
                entry_count,
                limits,
            )?;
        } else if metadata.is_file()
            && path.extension().is_some_and(|ext| ext == "groovy" || ext == "txt")
        {
            let canonical = path
                .canonicalize()
                .map_err(|e| ScanError::Io(format!("cannot resolve source file: {e}")))?;
            if !canonical.starts_with(checkout) {
                return Err(ScanError::Io("source file escaped checkout root".into()));
            }
            if metadata.len() > limits.file_bytes {
                return Err(ScanError::Io(format!(
                    "source file exceeds per-file limit of {} bytes",
                    limits.file_bytes
                )));
            }
            *total_bytes = total_bytes.saturating_add(metadata.len());
            if *total_bytes > limits.total_bytes {
                return Err(ScanError::Io(format!(
                    "source files exceed aggregate limit of {} bytes",
                    limits.total_bytes
                )));
            }
            if path.extension().is_some_and(|ext| ext == "groovy") {
                output.push(canonical);
                if output.len() > limits.files {
                    return Err(ScanError::Io(format!(
                        "source file count exceeds limit ({})",
                        limits.files
                    )));
                }
            }
        }
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
    let text_description = read_adjacent_documentation(file, path)?;
    let description = groovydoc_description.or(text_description);
    let mut symbol = json!({
        "id": format!("{source_id}:{path}:global"),
        "name": name,
        "kind": "global-variable",
        "sourceId": source_id,
        "description": description.unwrap_or_else(|| format!("Shared Library global defined by {path}")),
        "contexts": ["pipeline"],
        "confidence": confidence,
        "provenance": [format!("{path}:{} (global script)", declaration_line + 1)]
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

fn read_adjacent_documentation(groovy_file: &Path, relative_path: &str) -> Result<Option<String>, ScanError> {
    let documentation_file = groovy_file.with_extension("txt");
    match fs::symlink_metadata(&documentation_file) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {},
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ScanError::Io(format!("cannot inspect adjacent documentation for {relative_path}"))),
    };
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
        Err(_) => Err(ScanError::Io(format!("cannot read adjacent documentation for {relative_path}"))),
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
    let mut signature_depth = 0i32;
    let mut owner: Option<String> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some((name, signature)) = parse_class_declaration(trimmed) {
            let line_no = index + 1;
            let mut symbol = json!({
                "id": format!("{source_id}:{path}:class:{name}"),
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
            else if path.starts_with("src/") && depth == 1 && signature_depth == 0
                && !trimmed.is_empty() && !trimmed.starts_with('@') {
                if let Some((name, ty)) = parse_property_declaration(trimmed) {
                    let class_name = owner.as_deref().unwrap_or_default();
                    symbols.push(json!({
                        "id": format!("{source_id}:{path}:property:{class_name}:{name}:{}", &catalog_validator::sha256_hex(ty.as_bytes())[7..23]),
                        "name": name, "kind": "property", "sourceId": source_id,
                        "namespace": class_name, "signature": ty, "confidence": confidence,
                        "provenance": [format!("{path}:{}", index + 1)]
                    }));
                }
            }
        }
        if owner.is_some() {
            signature_depth = (signature_depth + paren_delta(line)).max(0);
        } else {
            signature_depth = 0;
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

fn paren_delta(line: &str) -> i32 {
    line.chars().fold(0, |n, c| match c { '(' => n+1, ')' => n-1, _ => n })
}

fn diagnose_unsupported_syntax(path: &str, source: &str, source_id: &str, diagnostics: &mut Vec<Diagnostic>) {
    let lines: Vec<&str> = source.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if ["trait ", "enum ", "record ", "@interface", "sealed class ", "def (", "def ["]
            .iter().any(|needle| trimmed.starts_with(needle)) {
            push_diagnostic(diagnostics, source_id, path, "unsupported-groovy-syntax",
                &format!("Unsupported declaration form at line {}; no symbol was inferred from it.", index + 1));
        }
        if is_method_declaration_start(trimmed) {
            let mut declaration = trimmed.to_owned();
            let mut next = index + 1;
            while !has_balanced_parentheses(&declaration) && next < lines.len() {
                declaration.push(' ');
                declaration.push_str(lines[next].trim());
                next += 1;
            }
            if !has_balanced_parentheses(&declaration) {
                push_diagnostic(diagnostics, source_id, path, "unsupported-groovy-syntax",
                    &format!("Incomplete method signature at line {}; no symbol was inferred from it.", index + 1));
            }
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
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if is_method_declaration_start(trimmed) {
            let start = index;
            let mut declaration = trimmed.to_owned();
            while !has_balanced_parentheses(&declaration) && index + 1 < lines.len() {
                index += 1;
                declaration.push(' ');
                declaration.push_str(lines[index].trim());
            }
            let owner = lines[..=start].iter().rev()
                .find_map(|line| parse_class_declaration(line.trim()).map(|(name, _)| name));
            if let Some(mut method) = parse_method_declaration(&declaration, start + 1, owner) {
                let (description, examples) = documentation(source, start);
                method.description = description;
                method.examples = examples;
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
    let first = prefix.split_whitespace().next().unwrap_or("");
    !["return", "throw", "new", "assert", "yield"].contains(&first)
        && !prefix.contains('=')
        && valid_identifier(name)
        && !["if","for","while","switch","catch","synchronized","assert"].contains(&name)
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
    let signature_hash = catalog_validator::sha256_hex(method.signature.as_bytes());
    let owner_id = owner.unwrap_or("global");
    let mut symbol = json!({
        "id": format!("{source_id}:{path}:{kind}:{owner_id}:{}:{}", method.name, &signature_hash[7..23]),
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

    static GIT_CHECKOUT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        assert!(symbols.iter().any(|s| s["name"] == "deployApp" && s["kind"] == "global-variable"
            && s["provenance"][0] == "vars/deployApp.groovy:5 (global script)"));
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
        assert_ne!(a.catalog["symbols"][0]["id"], b.catalog["symbols"][0]["id"]);
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
    fn rejects_local_repository_paths() {
        let config = LibraryConfig { name: "local".into(), repository: "/tmp/library".into(), reference: "main".into() };
        assert!(matches!(scan_library(&config), Err(ScanError::InvalidConfig(_))));
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
        fs::write(vars.join("example.groovy"), "trait DynamicThing { }\nenum State { READY }\ndef broken(\n").unwrap();
        let result = scan_checkout(&config("main"), temp.path(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        assert!(result.diagnostics.iter().filter(|d| d.code == "unsupported-groovy-syntax").count() >= 3);
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
    #[test]
    fn counts_adjacent_text_documentation_toward_total_source_budget() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp.path().join("vars");
        fs::create_dir_all(&vars).unwrap();
        fs::write(vars.join("helper.groovy"), "12345").unwrap();
        fs::write(vars.join("helper.txt"), "67890").unwrap();
        let checkout = temp.path().canonicalize().unwrap();
        let limits = ScanLimits {
            total_bytes: 9,
            ..ScanLimits::default()
        };
        let mut files = Vec::new();
        let mut total = 0;
        let mut entries = 0;
        assert!(collect_groovy_files(
            &vars,
            &checkout,
            0,
            &mut files,
            &mut total,
            &mut entries,
            &limits,
        )
        .is_err());
    }

    #[test]
    fn enforces_filesystem_entry_count_before_scanning() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp.path().join("vars");
        fs::create_dir_all(&vars).unwrap();
        fs::write(vars.join("one.groovy"), "def call() {}").unwrap();
        fs::write(vars.join("two.txt"), "documentation").unwrap();
        fs::write(vars.join("ignored.bin"), "other").unwrap();
        let checkout = temp.path().canonicalize().unwrap();
        let limits = ScanLimits {
            entries: 2,
            ..ScanLimits::default()
        };
        let mut files = Vec::new();
        let mut total = 0;
        let mut entries = 0;
        assert!(collect_groovy_files(
            &vars,
            &checkout,
            0,
            &mut files,
            &mut total,
            &mut entries,
            &limits,
        )
        .is_err());
    }

    #[test]
    fn rejects_symbol_and_catalog_output_over_limits() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp.path().join("vars");
        fs::create_dir_all(&vars).unwrap();
        fs::write(
            vars.join("deploy.groovy"),
            "def call() {}\ndef helper() {}\ndef another() {}\n",
        )
        .unwrap();
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let symbol_limited = ScanLimits {
            symbols: 1,
            ..ScanLimits::default()
        };
        let error = scan_checkout_with_limits(
            &config("main"),
            temp.path(),
            commit,
            symbol_limited,
        )
        .unwrap_err();
        assert!(error.to_string().contains("generated symbol count"));

        let catalog_limited = ScanLimits {
            catalog_bytes: 1,
            ..ScanLimits::default()
        };
        let error = scan_checkout_with_limits(
            &config("main"),
            temp.path(),
            commit,
            catalog_limited,
        )
        .unwrap_err();
        assert!(error.to_string().contains("generated catalog exceeds"));
    }

    #[cfg(unix)]
    #[test]
    fn ignores_symlinked_files_and_directories() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("vars")).unwrap();
        fs::create_dir_all(outside.path().join("vars")).unwrap();
        fs::write(outside.path().join("vars/escaped.groovy"), "def escaped() {}").unwrap();
        symlink(outside.path().join("vars/escaped.groovy"), temp.path().join("vars/linked.groovy")).unwrap();
        symlink(outside.path().join("vars"), temp.path().join("vars/linked-dir")).unwrap();
        symlink(temp.path().join("vars"), temp.path().join("vars/loop")).unwrap();
        let result = scan_checkout(&config("main"), temp.path(), "0123456789abcdef0123456789abcdef01234567").unwrap();
        assert!(!result.catalog["symbols"].as_array().unwrap().iter().any(|s| s["name"] == "escaped"));
    }

    #[test]
    fn enforces_file_size_count_depth_and_aggregate_limits() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vars");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("a.groovy"), "12345").unwrap();
        fs::write(root.join("nested/b.groovy"), "12345").unwrap();
        let checkout = temp.path().canonicalize().unwrap();
        let limits = ScanLimits {
            depth: 1,
            files: 2,
            entries: 10,
            file_bytes: 5,
            total_bytes: 10,
            symbols: 50,
            catalog_bytes: 1024 * 1024,
        };
        let mut files = Vec::new();
        let mut total = 0;
        let mut entries = 0;
        collect_groovy_files(&root, &checkout, 0, &mut files, &mut total, &mut entries, &limits).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(total, 10);

        let too_small = ScanLimits { file_bytes: 4, ..limits };
        assert!(collect_groovy_files(&root, &checkout, 0, &mut Vec::new(), &mut 0, &mut 0, &too_small).is_err());
        let too_few = ScanLimits { files: 1, ..limits };
        assert!(collect_groovy_files(&root, &checkout, 0, &mut Vec::new(), &mut 0, &mut 0, &too_few).is_err());
        let too_deep = ScanLimits { depth: 0, ..limits };
        assert!(collect_groovy_files(&root, &checkout, 0, &mut Vec::new(), &mut 0, &mut 0, &too_deep).is_err());
        let too_much = ScanLimits { total_bytes: 9, ..limits };
        assert!(collect_groovy_files(&root, &checkout, 0, &mut Vec::new(), &mut 0, &mut 0, &too_much).is_err());
    }

    #[test]
    fn temporary_checkout_is_unprivileged_unique_and_cleans_up() {
        let _guard = GIT_CHECKOUT_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let first = GitCheckout::new(1024 * 1024).unwrap();
        let second = GitCheckout::new(1024 * 1024).unwrap();
        assert_ne!(first.path(), second.path());
        let first_path = first.path().to_path_buf();
        fs::write(first_path.join("checkout-file"), "data").unwrap();
        assert!(first_path.exists());
        drop(first);
        assert!(!first_path.exists());
        assert!(second.path().exists());
    }

    fn git_for_test(directory: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(directory)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git is installed for scanner integration tests");
        assert!(status.success(), "git test command failed: {args:?}");
    }

    fn local_bare_remote(root: &Path, payload_bytes: usize) -> PathBuf {
        let source = root.join("source");
        let remote = root.join("remote.git");
        fs::create_dir_all(source.join("vars")).unwrap();
        git_for_test(&source, &["init", "--quiet"]);
        git_for_test(&source, &["config", "user.name", "Scanner Test"]);
        git_for_test(&source, &["config", "user.email", "scanner@example.test"]);
        let mut state = 0x1234_5678_9abc_def0u64;
        let payload: Vec<u8> = (0..payload_bytes)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                (state >> 32) as u8
            })
            .collect();
        fs::write(source.join("vars/payload.groovy"), payload).unwrap();
        git_for_test(&source, &["add", "--all"]);
        git_for_test(&source, &["commit", "--quiet", "-m", "fixture"]);
        git_for_test(root, &["init", "--bare", "--quiet", remote.to_str().unwrap()]);
        git_for_test(&source, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git_for_test(&source, &["push", "--quiet", "origin", "HEAD:refs/heads/main"]);
        git_for_test(&remote, &["config", "uploadpack.allowFilter", "false"]);
        remote
    }

    #[test]
    fn fetches_local_bare_repository_without_mount_privileges() {
        let _guard = GIT_CHECKOUT_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = tempfile::tempdir().unwrap();
        let remote = local_bare_remote(root.path(), 1024);
        let (checkout, commit) =
            fetch_checkout(remote.to_str().unwrap(), "main", 16 * 1024 * 1024).unwrap();
        assert_eq!(commit.len(), 40);
        assert!(checkout.path().join("vars/payload.groovy").is_file());
    }

    #[test]
    fn aborts_when_server_ignores_blob_filter_and_checkout_budget_is_exceeded() {
        let _guard = GIT_CHECKOUT_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = tempfile::tempdir().unwrap();
        let remote = local_bare_remote(root.path(), 8 * 1024 * 1024);
        let temp_root = std::env::temp_dir();
        let before = fs::read_dir(&temp_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("jenkins-knowledge-git-"))
            .count();

        let result = fetch_checkout(remote.to_str().unwrap(), "main", 512 * 1024);
        assert!(result.is_err(), "oversized ignored-filter fetch unexpectedly succeeded");

        let after = fs::read_dir(&temp_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("jenkins-knowledge-git-"))
            .count();
        assert_eq!(after, before, "failed fetch left a temporary checkout behind");
    }

    #[test]
    fn checkout_size_counts_regular_files_and_ignores_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a"), vec![0u8; 16]).unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(temp.path().join("nested/b"), vec![0u8; 32]).unwrap();
        assert_eq!(checkout_size(temp.path(), 64).unwrap(), 48);
        assert!(checkout_size(temp.path(), 32).unwrap() > 32);
    }

    #[test]
    fn rejects_zero_checkout_budget() {
        assert!(matches!(GitCheckout::new(0), Err(ScanError::InvalidConfig(_))));
    }

    #[test]
    fn scanner_uses_shared_recursive_secret_redaction_policy() {
        let mut catalog = serde_json::json!({
            "sources": [{
                "metadata": {
                    "safe": "preserve",
                    "privateKey": "scanner-secret-key",
                    "nested": [{"sessionId": "scanner-secret-session"}],
                    "repositoryUrl": "https://scanner:scanner-secret-user@github.example.test/team/repo.git?token=scanner-secret-token"
                }
            }]
        });
        redact_sensitive_fields(&mut catalog);
        let serialized = catalog.to_string();
        assert!(!serialized.contains("scanner-secret"));
        assert_eq!(catalog["sources"][0]["metadata"]["safe"], "preserve");
        assert!(catalog["sources"][0]["metadata"].get("privateKey").is_none());
        assert!(catalog["sources"][0]["metadata"]["nested"][0].get("sessionId").is_none());
        assert_eq!(catalog["sources"][0]["metadata"]["repositoryUrl"], "https://github.example.test/team/repo.git");
    }

    #[test]
    fn source_identity_is_stable_when_the_same_reference_moves_to_a_new_commit() {
        let first = scan_checkout(
            &config("main"),
            &fixture(),
            "0123456789abcdef0123456789abcdef01234567",
        ).unwrap();
        let second = scan_checkout(
            &config("main"),
            &fixture(),
            "89abcdef0123456789abcdef0123456789abcdef",
        ).unwrap();

        assert_eq!(first.catalog["sources"][0]["id"], second.catalog["sources"][0]["id"]);
        assert_ne!(first.catalog["sources"][0]["revision"], second.catalog["sources"][0]["revision"]);
        assert_ne!(first.catalog["snapshotId"], second.catalog["snapshotId"]);
    }

}
