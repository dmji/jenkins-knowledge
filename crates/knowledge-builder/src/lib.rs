use catalog_validator::{completeness_reason_codes, redact_sensitive_fields, source_collection_summary, stamp_catalog_identity, validate_catalog, validate_package_metadata};
use jenkins_gdsl_collector::{ApiTypeLimits, CollectorError, DiscoveryError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet, HashSet}, env, fs, path::{Path, PathBuf}};

mod source_security;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub profile: String,
    #[serde(rename = "collectionProfile", default = "default_collection_profile")]
    pub collection_profile: String,
    #[serde(default)]
    pub jenkins: JenkinsConfig,
    #[serde(default)]
    pub libraries: Vec<LibraryConfig>,
    #[serde(default)]
    pub fixtures: Vec<String>,
    #[serde(default)]
    pub package: PackageConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JenkinsConfig {
    #[serde(default)]
    pub controllers: Vec<ControllerConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    pub id: String,
    pub url: String,
    #[serde(default, rename = "apiRoots")]
    pub api_roots: Vec<String>,
    #[serde(default, rename = "apiLimits")]
    pub api_limits: ApiTypeLimits,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig {
    pub name: String,
    pub url: String,
    pub revision: String,
    pub commit: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    pub id: Option<String>,
    pub include: Option<Vec<PackageSection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageSection {
    JenkinsCore,
    Controller,
    Plugins,
    SharedLibraries,
}

impl PackageSection {
    fn as_str(self) -> &'static str {
        match self {
            Self::JenkinsCore => "jenkins-core",
            Self::Controller => "controller",
            Self::Plugins => "plugins",
            Self::SharedLibraries => "shared-libraries",
        }
    }

    fn for_source_kind(kind: &str) -> Option<Self> {
        match kind {
            "jenkins-core" => Some(Self::JenkinsCore),
            "jenkins-controller" => Some(Self::Controller),
            "plugin" => Some(Self::Plugins),
            "shared-library" => Some(Self::SharedLibraries),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct PackageSelection {
    included: BTreeSet<PackageSection>,
    explicit: bool,
}

impl PackageSelection {
    fn includes(&self, section: PackageSection) -> bool {
        self.included.contains(&section)
    }
}

fn default_collection_profile() -> String {
    "quick".into()
}

fn valid_api_class_name(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 || value.trim() != value
        || value.starts_with("groovy.") || value.starts_with("org.codehaus.groovy.")
    {
        return false;
    }
    value.split('.').all(|part| {
        let mut chars = part.chars();
        matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_' || ch == '$')
            && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    })
}

fn validate_profile(profile: &Profile) -> Result<PackageSelection, BuildError> {
    if !matches!(profile.collection_profile.as_str(), "quick" | "standard" | "deep") {
        return Err(BuildError::Profile(
            "collectionProfile: expected quick, standard, or deep".into(),
        ));
    }
    if profile.profile.trim().is_empty()
        || profile.profile.trim() != profile.profile
        || profile.profile.chars().any(char::is_control)
    {
        return Err(BuildError::Profile(
            "profile: identity must be non-empty and have no surrounding whitespace".into(),
        ));
    }
    if profile.package.id.as_ref().is_some_and(|id| {
        id.trim().is_empty() || id.trim() != id || id.chars().any(char::is_control)
    }) {
        return Err(BuildError::Profile(
            "package.id: must be non-empty and have no surrounding whitespace when provided".into(),
        ));
    }

    let fixtures_mode = !profile.fixtures.is_empty();
    if fixtures_mode && (!profile.jenkins.controllers.is_empty() || !profile.libraries.is_empty()) {
        return Err(BuildError::Profile(
            "fixtures: offline fixture catalogs cannot be combined with live Jenkins controllers or Shared Libraries".into(),
        ));
    }
    if !fixtures_mode && profile.jenkins.controllers.is_empty() && profile.libraries.is_empty() {
        return Err(BuildError::Profile(
            "configure at least one Jenkins controller, Shared Library, or fixture catalog".into(),
        ));
    }
    let mut controller_ids = HashSet::new();
    for (index, controller) in profile.jenkins.controllers.iter().enumerate() {
        if controller.id.trim().is_empty()
            || controller.id.trim() != controller.id
            || controller.id.chars().any(char::is_control)
        {
            return Err(BuildError::Profile(format!(
                "jenkins.controllers[{index}].id: must be non-empty and have no surrounding whitespace"
            )));
        }
        if !controller_ids.insert(&controller.id) {
            return Err(BuildError::Profile(format!(
                "jenkins.controllers[{index}].id: duplicate controller id"
            )));
        }
        source_security::validate_controller_url(&controller.url).map_err(|error| {
            BuildError::Profile(format!("jenkins.controllers[{index}].url: {error}"))
        })?;
        if controller.api_roots.len() > 20 {
            return Err(BuildError::Profile(format!(
                "jenkins.controllers[{index}].apiRoots: at most 20 explicit class roots are allowed"
            )));
        }
        if !controller.api_limits.is_within_hard_limits() {
            return Err(BuildError::Profile(format!(
                "jenkins.controllers[{index}].apiLimits exceeds the hard safety maxima"
            )));
        }
        let mut api_roots = HashSet::new();
        for (root_index, root) in controller.api_roots.iter().enumerate() {
            if !valid_api_class_name(root) {
                return Err(BuildError::Profile(format!(
                    "jenkins.controllers[{index}].apiRoots[{root_index}]: expected a fully-qualified Java class name; Groovy runtime roots are excluded"
                )));
            }
            if !api_roots.insert(root) {
                return Err(BuildError::Profile(format!(
                    "jenkins.controllers[{index}].apiRoots[{root_index}]: duplicate API root"
                )));
            }
        }
    }

    if profile.jenkins.controllers.len() > 1 {
        return Err(BuildError::Profile(
            "jenkins.controllers: one controller per package is supported by the current catalog snapshot schema; build separate profiles for separate controllers".into(),
        ));
    }

    let mut library_names = HashSet::new();
    for (index, library) in profile.libraries.iter().enumerate() {
        if library.name.trim().is_empty()
            || library.name.trim() != library.name
            || library.name.chars().any(char::is_control)
        {
            return Err(BuildError::Profile(format!(
                "libraries[{index}].name: must be non-empty and have no surrounding whitespace"
            )));
        }
        if !library_names.insert(&library.name) {
            return Err(BuildError::Profile(format!(
                "libraries[{index}].name: duplicate Shared Library name"
            )));
        }
        source_security::validate_git_url(&library.url).map_err(|error| {
            BuildError::Profile(format!("libraries[{index}].url: {error}"))
        })?;
        source_security::validate_git_reference(&library.revision).map_err(|error| {
            BuildError::Profile(format!("libraries[{index}].revision: {error}"))
        })?;
        if let Some(commit) = &library.commit {
            if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(BuildError::Profile(format!(
                    "libraries[{index}].commit: must be a full 40-character SHA"
                )));
            }
            if library.revision.len() == 40
                && library.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !library.revision.eq_ignore_ascii_case(commit)
            {
                return Err(BuildError::Profile(format!(
                    "libraries[{index}].commit: conflicts with the full-SHA revision"
                )));
            }
        }
    }

    let mut fixtures = HashSet::new();
    for (index, fixture) in profile.fixtures.iter().enumerate() {
        let path = Path::new(fixture);
        let has_windows_prefix = fixture.as_bytes().get(1) == Some(&b':');
        if fixture.trim().is_empty()
            || fixture.trim() != fixture
            || path.is_absolute()
            || fixture.starts_with("\\\\")
            || has_windows_prefix
            || fixture.chars().any(char::is_control)
        {
            return Err(BuildError::Profile(format!(
                "fixtures[{index}]: must be a non-empty relative path without control characters"
            )));
        }
        if !fixtures.insert(fixture) {
            return Err(BuildError::Profile(format!(
                "fixtures[{index}]: duplicate fixture path"
            )));
        }
    }

    let all_sections = [
        PackageSection::JenkinsCore,
        PackageSection::Controller,
        PackageSection::Plugins,
        PackageSection::SharedLibraries,
    ];
    let (included, explicit) = match &profile.package.include {
        None => (all_sections.into_iter().collect(), false),
        Some(values) if values.is_empty() => {
            return Err(BuildError::Profile(
                "package.include: omit the field to include all sections; an explicit empty list is invalid".into(),
            ));
        }
        Some(values) => {
            let included: BTreeSet<_> = values.iter().copied().collect();
            if included.len() != values.len() {
                return Err(BuildError::Profile(
                    "package.include: duplicate section selector".into(),
                ));
            }
            if !fixtures_mode {
                for section in &included {
                    match section {
                        PackageSection::JenkinsCore | PackageSection::Controller | PackageSection::Plugins
                            if profile.jenkins.controllers.is_empty() =>
                        {
                            return Err(BuildError::Profile(format!(
                                "package.include: '{}' requires at least one Jenkins controller",
                                section.as_str()
                            )));
                        }
                        PackageSection::SharedLibraries if profile.libraries.is_empty() => {
                            return Err(BuildError::Profile(
                                "package.include: 'shared-libraries' requires at least one configured Shared Library".into(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            (included, true)
        }
    };

    if !fixtures_mode && matches!(profile.collection_profile.as_str(), "quick" | "deep") && profile.jenkins.controllers.is_empty() {
        return Err(BuildError::Profile(format!(
            "collectionProfile: {} requires a Jenkins controller",
            profile.collection_profile
        )));
    }

    Ok(PackageSelection { included, explicit })
}

#[derive(Debug)]
pub enum BuildError { Io(String), Profile(String), Source(String), InvalidCatalog(String), OutputExists(String) }
impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "{s}"),
            Self::Profile(s) => write!(f, "profile: {s}"),
            Self::Source(s) => write!(f, "source: {s}"),
            Self::InvalidCatalog(s) => write!(f, "catalog validation failed: {s}"),
            Self::OutputExists(s) => write!(f, "output already exists: {s}"),
        }
    }
}
impl std::error::Error for BuildError {}

fn runtime_api_type_symbols(runtime: &Value, source_id: &str) -> Vec<Value> {
    let mut symbols = Vec::new();
    let mut member_count = 0usize;
    for api_type in runtime["apiTypes"].as_array().into_iter().flatten().take(40) {
        let (Some(class_name), Some(name)) = (api_type["className"].as_str(), api_type["name"].as_str()) else { continue; };
        let class_confidence = if api_type["status"] == "available" && api_type["synthetic"] != true && api_type["dynamic"] != true { "verified" } else { "unknown" };
        let mut class_symbol = json!({
            "id": format!("{source_id}:api-class:{}", hex(&Sha256::digest(class_name.as_bytes()))),
            "name": name, "kind": "class", "sourceId": source_id, "signature": class_name,
            "confidence": class_confidence, "provenance": ["jenkins-script-console", "api-types"],
            "fieldEvidence": {"name": class_confidence, "signature": "verified"},
            "evidence": [{
                "sourceId": source_id, "kind": "controller-observed", "confidence": class_confidence,
                "provenance": "Bounded reflection of registered or explicitly configured API roots",
                "retrievedAt": runtime["collectedAt"],
                "location": format!("script-console/api-types/{class_name}"),
                "sourceKind": "jenkins-controller"
            }]
        });
        if let Some(plugin) = api_type["plugin"].as_str() { class_symbol["namespace"] = json!(plugin); }
        if let Some(paths) = api_type["documentationPaths"].as_array() {
            class_symbol["documentationPaths"] = json!(paths);
        }
        if let Some(status) = api_type["status"].as_str() {
            class_symbol["typeStatus"] = json!(status);
            class_symbol["fieldEvidence"]["typeStatus"] = json!("verified");
        }
        if let Some(status) = api_type["membersStatus"].as_str() {
            class_symbol["membersStatus"] = json!(status);
            class_symbol["fieldEvidence"]["membersStatus"] = json!("verified");
        }
        for field in ["depth", "rootCategories", "methodsTruncated", "propertiesTruncated", "classLoader"] {
            if let Some(value) = api_type.get(field) { class_symbol[field] = value.clone(); }
        }
        for field in ["superClass", "origin"] {
            if let Some(value) = api_type[field].as_str() {
                class_symbol[field] = json!(value);
                class_symbol["fieldEvidence"][field] = json!("verified");
            }
        }
        if let Some(interfaces) = api_type["interfaces"].as_array() {
            class_symbol["interfaces"] = json!(interfaces);
            class_symbol["fieldEvidence"]["interfaces"] = json!("verified");
        }
        for field in ["synthetic", "dynamic"] {
            if let Some(value) = api_type[field].as_bool() {
                class_symbol[field] = json!(value);
                class_symbol["fieldEvidence"][field] = json!("verified");
            }
        }
        if let Some(properties) = api_type["properties"].as_array() {
            class_symbol["properties"] = json!(properties);
            if !properties.is_empty() { class_symbol["fieldEvidence"]["properties"] = json!("inferred"); }
        }
        symbols.push(class_symbol);
        for method in api_type["methods"].as_array().into_iter().flatten() {
            if member_count >= 300 { break; }
            let (Some(method_name), Some(signature), Some(return_type), Some(declaring_class)) = (
                method["name"].as_str(), method["signature"].as_str(),
                method["returnType"].as_str(), method["declaringClass"].as_str()
            ) else { continue; };
            let confidence = if class_confidence == "verified" && method["synthetic"] != true && method["bridge"] != true && method["dynamic"] != true { "verified" } else { "unknown" };
            let method_identity = format!("{class_name}\0{signature}");
            let mut method_symbol = json!({
                "id": format!("{source_id}:api-method:{}", hex(&Sha256::digest(method_identity.as_bytes()))),
                "name": method_name, "kind": "method", "sourceId": source_id,
                "signature": signature, "returnType": return_type, "declaringClass": declaring_class,
                "confidence": confidence, "provenance": ["jenkins-script-console", "api-types", class_name],
                "fieldEvidence": {"name": confidence, "signature": "verified", "returnType": "verified", "declaringClass": "verified"},
                "evidence": [{
                    "sourceId": source_id, "kind": "controller-observed", "confidence": confidence,
                    "provenance": "Java reflection metadata; method body was not executed",
                    "retrievedAt": runtime["collectedAt"],
                    "location": format!("script-console/api-types/{class_name}"),
                    "sourceKind": "jenkins-controller"
                }]
            });
            if let Some(plugin) = api_type["plugin"].as_str() { method_symbol["namespace"] = json!(plugin); }
            if let Some(parameters) = method["parameters"].as_array() {
                method_symbol["parameters"] = json!(parameters);
                method_symbol["fieldEvidence"]["parameters"] = json!("verified");
            }
            if let Some(truncated) = method["parametersTruncated"].as_bool() {
                method_symbol["parametersTruncated"] = json!(truncated);
                method_symbol["fieldEvidence"]["parametersTruncated"] = json!("verified");
            }
            for field in ["genericReturnType", "modifiers", "genericReturnTypeStatus"] {
                if let Some(value) = method[field].as_str() {
                    method_symbol[field] = json!(value);
                    if field != "genericReturnTypeStatus" { method_symbol["fieldEvidence"][field] = json!("verified"); }
                }
            }
            for field in ["synthetic", "dynamic", "bridge", "varargs"] {
                if let Some(value) = method[field].as_bool() {
                    method_symbol[field] = json!(value);
                    method_symbol["fieldEvidence"][field] = json!("verified");
                }
            }
            symbols.push(method_symbol);
            member_count += 1;
        }
    }
    symbols
}

fn runtime_symbols(runtime: &Value, source_id: &str) -> Vec<Value> {
    let mut symbols = Vec::new();
    for registry in runtime["extensionRegistries"].as_array().into_iter().flatten() {
        let registry_name = registry["name"].as_str().unwrap_or_default();
        let kind = match registry_name {
            "pipeline-steps" => "step",
            "pipeline-globals" => "global-variable",
            _ => continue,
        };
        for (index, item) in registry["items"].as_array().into_iter().flatten().enumerate() {
            let (Some(name), Some(class_name)) = (
                item["name"].as_str(),
                item["className"].as_str(),
            ) else {
                continue;
            };
            let id = format!("{}:runtime:{}:{}:{}:{}", source_id, registry_name, name, class_name, index);
            let mut symbol = json!({
                "id": id,
                "name": name,
                "kind": kind,
                "sourceId": source_id,
                "signature": class_name,
                "confidence": "verified",
                "provenance": ["jenkins-script-console", registry_name],
                "fieldEvidence": {
                    "name": "verified",
                    "signature": "verified",
                    "parameters": "unknown"
                },
                "evidence": [{
                    "sourceId": source_id,
                    "kind": "controller-observed",
                    "confidence": "verified",
                    "provenance": "Registered Jenkins extension descriptor",
                    "retrievedAt": runtime["collectedAt"],
                    "location": format!("script-console/{}", registry_name),
                    "sourceKind": "jenkins-controller"
                }]
            });
            if let Some(plugin) = item["plugin"].as_str() {
                symbol["namespace"] = json!(plugin);
            }
            if let Some(display_name) = item["displayName"].as_str() {
                symbol["displayName"] = json!(display_name);
            }
            if let Some(parameters) = item["parameters"].as_array() {
                symbol["parameters"] = json!(parameters);
                symbol["fieldEvidence"]["parameters"] = json!("verified");
            }
            symbols.push(symbol);
        }
    }
    symbols
}

#[derive(Debug, Serialize, Deserialize)]
struct ControllerCacheSnapshot {
    fingerprint: String,
    catalogs: Vec<(String, Value)>,
    outcomes: Vec<Value>,
}

fn cache_root(profile_dir: &Path) -> PathBuf {
    if let Some(path) = env::var_os("JENKINS_KNOWLEDGE_CACHE_DIR") {
        return PathBuf::from(path).join("controller-snapshots");
    }
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| profile_dir.join(".cache"))
        .join("jenkins-knowledge")
        .join("controller-snapshots")
}

fn controller_cache_path(base: &Path, controller: &ControllerConfig, profile: &Profile) -> Result<PathBuf, BuildError> {
    let identity = json!({
        "id": controller.id,
        "url": source_security::redact_url(&controller.url),
        "collectionProfile": profile.collection_profile,
        "apiRoots": controller.api_roots,
        "apiLimits": {
            "maxDepth": controller.api_limits.max_depth,
            "maxTypes": controller.api_limits.max_types,
            "maxMembersPerType": controller.api_limits.max_members_per_type,
            "maxTotalMembers": controller.api_limits.max_total_members,
        },
    });
    let bytes = serde_json::to_vec(&identity).map_err(|error| BuildError::Io(error.to_string()))?;
    let key = hex(&Sha256::digest(bytes));
    Ok(cache_root(base).join(format!("{key}.json")))
}

fn update_collection_source_revision(outcomes: &mut [Value], source_id: &str, revision: &Value) {
    for outcome in outcomes {
        if outcome["sourceId"].as_str() == Some(source_id)
            && outcome["revision"].is_string()
        {
            outcome["revision"] = revision.clone();
        }
    }
}

fn controller_fingerprint(catalog: &Value) -> Result<String, BuildError> {
    let controller = catalog.get("controller").ok_or_else(|| BuildError::Source(
        "controller discovery returned no metadata for fingerprinting".into()
    ))?;
    let identity = json!({
        "coreVersion": controller.get("coreVersion"),
        "plugins": controller.get("plugins"),
        "capabilities": controller.get("capabilities"),
        "identity": controller.get("identity"),
    });
    let bytes = serde_json::to_vec(&identity).map_err(|error| BuildError::Io(error.to_string()))?;
    Ok(hex(&Sha256::digest(bytes)))
}

fn read_controller_cache(path: &Path, fingerprint: &str) -> Option<ControllerCacheSnapshot> {
    let contents = fs::read(path).ok()?;
    let cached: ControllerCacheSnapshot = serde_json::from_slice(&contents).ok()?;
    (cached.fingerprint == fingerprint).then_some(cached)
}

fn write_controller_cache(path: &Path, snapshot: &ControllerCacheSnapshot) -> Result<(), BuildError> {
    let parent = path.parent().ok_or_else(|| BuildError::Io("cache path has no parent".into()))?;
    fs::create_dir_all(parent).map_err(|error| BuildError::Io(format!("cannot create controller cache: {error}")))?;
    let staging = tempfile::Builder::new().prefix(".controller-cache-").tempdir_in(parent)
        .map_err(|error| BuildError::Io(format!("cannot stage controller cache: {error}")))?;
    let bytes = serde_json::to_vec_pretty(snapshot).map_err(|error| BuildError::Io(error.to_string()))?;
    fs::write(staging.path().join("snapshot.json"), bytes)
        .map_err(|error| BuildError::Io(format!("cannot write controller cache: {error}")))?;
    fs::rename(staging.path().join("snapshot.json"), path)
        .map_err(|error| BuildError::Io(format!("cannot publish controller cache: {error}")))?;
    Ok(())
}

pub fn invalidate_cache(profile_path: &Path) -> Result<usize, BuildError> {
    let input = fs::read_to_string(profile_path)
        .map_err(|error| BuildError::Io(format!("cannot read {}: {error}", profile_path.display())))?;
    let profile: Profile = serde_yaml::from_str(&input).map_err(|error| BuildError::Profile(error.to_string()))?;
    validate_profile(&profile)?;
    let base = profile_path.parent().unwrap_or(Path::new("."));
    let mut removed = 0;
    for controller in &profile.jenkins.controllers {
        let path = controller_cache_path(base, controller, &profile)?;
        match fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(BuildError::Io(format!("cannot invalidate controller cache: {error}"))),
        }
    }
    Ok(removed)
}

/// Build a network-free collection plan for a validated profile.
pub fn collection_plan(profile_path: &Path) -> Result<Value, BuildError> {
    let input = fs::read_to_string(profile_path)
        .map_err(|error| BuildError::Io(format!("cannot read {}: {error}", profile_path.display())))?;
    let profile: Profile = serde_yaml::from_str(&input)
        .map_err(|error| BuildError::Profile(error.to_string()))?;
    let selection = validate_profile(&profile)?;
    let mut requests = Vec::new();

    for controller in &profile.jenkins.controllers {
        let base = source_security::redact_url(&controller.url).trim_end_matches('/').to_owned();
        requests.extend([
            json!({
                "controller": controller.id,
                "method": "GET",
                "url": format!("{base}/api/json?tree=_class,nodeName,jenkinsVersion"),
                "purpose": "required controller identity and Jenkins Core version",
                "responseCapBytes": 1_048_576
            }),
            json!({
                "controller": controller.id,
                "method": "GET",
                "url": format!("{base}/pluginManager/api/json?tree=plugins[shortName,version,active,enabled,dependencies[shortName,version,optional]]"),
                "purpose": "optional installed plugin inventory",
                "responseCapBytes": 4_194_304
            }),
            json!({
                "controller": controller.id,
                "method": "GET",
                "url": format!("{base}/pipeline-syntax/gdsl"),
                "headers": {"Range":"bytes=0-0"},
                "purpose": "one-byte GDSL availability probe",
                "responseReadCapBytes": 1
            })
        ]);
        if matches!(profile.collection_profile.as_str(), "standard" | "deep") {
            requests.push(json!({
                "controller": controller.id,
                "method": "GET",
                "url": format!("{base}/pipeline-syntax/gdsl"),
                "purpose": "bounded full Pipeline GDSL extraction",
                "responseCapBytes": 4_194_304,
                "maxSymbols": 10_000,
                "maxParametersPerSymbol": 256
            }));
        }
        if profile.collection_profile == "deep" {
            requests.push(json!({
                "controller": controller.id,
                "method": "POST",
                "url": format!("{base}/scriptText"),
                "formFields": ["script"],
                "probeTemplate": "src/probes/script_console_v1.groovy",
                "templateVersion": "3",
                "purpose": "fixed read-only runtime/extension/API metadata probe",
                "explicitOptIn": "collectionProfile: deep",
                "callerScriptAccepted": false,
                "responseCapBytes": 524_288,
                "apiRoots": controller.api_roots,
                "apiLimits": {
                    "maxDepth": controller.api_limits.max_depth,
                    "maxTypes": controller.api_limits.max_types,
                    "maxMembersPerType": controller.api_limits.max_members_per_type,
                    "maxTotalMembers": controller.api_limits.max_total_members
                },
                "confirmationRequired": true
            }));
        }
    }

    let libraries = if profile.collection_profile != "quick" && selection.includes(PackageSection::SharedLibraries) {
        profile.libraries.iter().map(|library| json!({
            "name": library.name,
            "url": source_security::redact_url(&library.url),
            "revision": library.revision,
            "commit": library.commit,
            "transport": "administrator-configured Git remote; bounded shallow/blob-filtered checkout"
        })).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut script_console = (profile.collection_profile == "deep"
        && !profile.jenkins.controllers.is_empty())
        .then(|| jenkins_gdsl_collector::script_console_plan()["scriptConsole"].clone());
    if let Some(plan) = script_console.as_mut() {
        plan["confirmationMethod"] = json!("explicit collectionProfile: deep selection");
        plan["callerScriptAccepted"] = json!(false);
    }

    Ok(json!({
        "schemaVersion": "1.0.0",
        "profile": profile.profile,
        "collectionProfile": profile.collection_profile,
        "packageId": profile.package.id.clone().unwrap_or_else(|| profile.profile.clone()),
        "packageSections": selection.included.iter().map(|section| section.as_str()).collect::<Vec<_>>(),
        "packageSectionsExplicit": selection.explicit,
        "offlineFixtures": profile.fixtures,
        "networkRequests": requests,
        "requestCount": requests.len(),
        "sharedLibraries": libraries,
        "scriptConsole": script_console,
        "timeouts": {"connectSeconds": 5, "requestSeconds": 15, "requestsSequential": true, "automaticRetries": false},
        "limits": {
            "controllerMetadataBytes": 1_048_576,
            "pluginInventoryBytes": 4_194_304,
            "gdslBytes": 4_194_304,
            "gdslSymbols": 10_000,
            "gdslParametersPerSymbol": 256,
            "scriptConsoleBytes": 524_288,
            "scriptConsoleApiRoots": controller_api_root_limit(&profile),
            "scriptConsoleApiTypes": 40,
            "scriptConsoleApiDepth": 2,
            "scriptConsoleMethodsPerType": 12,
            "scriptConsoleTotalMethods": 300
        },
        "credentials": {"source": "JENKINS_USERNAME and JENKINS_API_TOKEN environment variables", "valuesIncluded": false},
        "callsMade": false
    }))
}

fn controller_api_root_limit(profile: &Profile) -> usize {
    profile.jenkins.controllers.iter().map(|controller| controller.api_roots.len()).sum()
}

pub fn build_package(profile_path: &Path, output: &Path) -> Result<(), BuildError> {
    build_package_with_options(profile_path, output, false)
}

pub fn build_package_with_options(profile_path: &Path, output: &Path, refresh: bool) -> Result<(), BuildError> {
    if output.exists() { return Err(BuildError::OutputExists(output.display().to_string())); }
    let input = fs::read_to_string(profile_path).map_err(|e| BuildError::Io(format!("cannot read {}: {e}", profile_path.display())))?;
    let profile: Profile = serde_yaml::from_str(&input).map_err(|e| BuildError::Profile(e.to_string()))?;
    let selection = validate_profile(&profile)?;
    let base = profile_path.parent().unwrap_or(Path::new("."));
    let collection_started_at = build_timestamp();
    let mut collection_outcomes = Vec::new();
    let mut catalogs = Vec::new();

    if !profile.fixtures.is_empty() {
        for fixture in &profile.fixtures {
            let path = base.join(fixture);
            let text = fs::read_to_string(&path).map_err(|e| BuildError::Source(format!("fixture {}: {e}", path.display())))?;
            let catalog: Value = serde_json::from_str(&text).map_err(|e| BuildError::Source(format!("fixture {} is invalid JSON: {e}", path.display())))?;
            // A sibling manifest preserves explicit partial outcomes in sanitized fixture packages.
            let fixture_manifest_path = path
                .parent()
                .unwrap_or(base)
                .join("manifest.json");
            let fixture_outcomes = if fixture_manifest_path.is_file() {
                let manifest_text = fs::read_to_string(&fixture_manifest_path).map_err(|error| {
                    BuildError::Source(format!(
                        "fixture manifest {}: {error}",
                        fixture_manifest_path.display()
                    ))
                })?;
                let manifest: Value = serde_json::from_str(&manifest_text).map_err(|error| {
                    BuildError::Source(format!(
                        "fixture manifest {} is invalid JSON: {error}",
                        fixture_manifest_path.display()
                    ))
                })?;
                manifest
                    .pointer("/collection/sources")
                    .and_then(Value::as_array)
                    .cloned()
            } else {
                None
            };
            if let Some(outcomes) = fixture_outcomes {
                collection_outcomes.extend(outcomes);
            } else {
                for source in catalog["sources"].as_array().into_iter().flatten() {
                    collection_outcomes.push(json!({
                        "id": format!("fixture:{fixture}:{}", source["id"].as_str().unwrap_or_default()),
                        "kind": "fixture",
                        "name": fixture,
                        "surface": "catalog",
                        "sourceId": source["id"],
                        "revision": source["revision"],
                        "status": "succeeded"
                    }));
                }
            }
            catalogs.push((fixture.clone(), catalog));
        }
    } else {
        let username = env::var("JENKINS_USERNAME").ok();
        let api_token = env::var("JENKINS_API_TOKEN").ok();
        for controller in &profile.jenkins.controllers {
            let controller_cache = controller_cache_path(base, controller, &profile)?;
            let cached_catalog_start = catalogs.len();
            let cached_outcome_start = collection_outcomes.len();
            // Core controller metadata is mandatory. Optional inventories and GDSL
            // are recorded independently so their failure cannot discard valid data.
            let gdsl_only_profile = selection.explicit
                && selection.included.len() == 1
                && selection.includes(PackageSection::Controller)
                && profile.collection_profile == "standard";
            let discovery = jenkins_gdsl_collector::discover(
                &controller.url,
                username.as_deref(),
                api_token.as_deref(),
            );
            let (mut catalog, controller_metadata_available, discovery_failure) = match discovery {
                Ok(snapshot) => (snapshot, true, None),
                Err(error) if gdsl_only_profile => {
                    // Preserve legacy GDSL-only profiles: when the caller explicitly
                    // selects only the controller/GDSL section, Core metadata is not
                    // a prerequisite for the requested catalog.
                    let gdsl = jenkins_gdsl_collector::collect(
                        &controller.url,
                        username.as_deref(),
                        api_token.as_deref(),
                    )
                    .map_err(|gdsl_error| BuildError::Source(format!(
                        "Jenkins controller '{}' GDSL collection failed: {gdsl_error}",
                        controller.id
                    )))?;
                    (gdsl, false, Some(error))
                }
                Err(error) => {
                    return Err(BuildError::Source(format!(
                        "Jenkins controller '{}' mandatory metadata collection failed: {error}",
                        controller.id
                    )));
                }
            };
            let fingerprint = if controller_metadata_available {
                Some(controller_fingerprint(&catalog)?)
            } else {
                None
            };
            if !refresh {
                if let Some(fingerprint) = fingerprint.as_deref() {
                    if let Some(cached) = read_controller_cache(&controller_cache, fingerprint) {
                        eprintln!("controller snapshot cache hit: {}", controller.id);
                        catalogs.extend(cached.catalogs);
                        collection_outcomes.extend(cached.outcomes);
                        continue;
                    }
                }
            }
            let controller_source_id = catalog["sources"][0]["id"]
                .as_str()
                .ok_or_else(|| BuildError::Source("controller collection returned no source identity".into()))?
                .to_owned();
            let core_version = catalog["controller"]["coreVersion"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned();

            if controller_metadata_available {
                collection_outcomes.push(json!({
                    "id": format!("controller:{}", controller.id),
                    "kind": "controller",
                    "name": controller.id,
                    "surface": "controller-metadata",
                    "sourceId": controller_source_id,
                    "revision": catalog["sources"][0]["revision"],
                    "status": "succeeded"
                }));
            } else {
                let (status, code) = discovery_outcome(discovery_failure.as_ref().expect("fallback has discovery error"));
                collection_outcomes.push(json!({
                    "id": format!("controller:{}", controller.id),
                    "kind": "controller",
                    "name": controller.id,
                    "surface": "controller-metadata",
                    "sourceId": controller_source_id,
                    "status": status,
                    "errorCode": code
                }));
            }

            if controller_metadata_available {
                let plugin_coverage = catalog["coverage"].as_array()
                    .and_then(|entries| entries.iter().find(|entry| entry["surface"] == "plugin-inventory"));
                let (plugin_status, plugin_error) = match plugin_coverage {
                    Some(entry) if entry["status"] == "inspected" => ("succeeded", None),
                    Some(entry) if entry["status"] == "unavailable" => {
                        let details = entry["details"].as_str().unwrap_or_default();
                        if details.contains("HTTP 401") || details.contains("HTTP 403") {
                            ("permission-denied", Some("permission-denied"))
                        } else {
                            ("unavailable", Some("http-unavailable"))
                        }
                    }
                    Some(entry) if entry["details"].as_str().unwrap_or_default().contains("exceeded the") =>
                        ("failed", Some("limit-exceeded")),
                    Some(_) => ("failed", Some("invalid-response")),
                    None => ("failed", Some("missing-outcome")),
                };
                let mut plugin_outcome = json!({
                    "id": format!("controller:{}:plugin-inventory", controller.id),
                    "kind": "plugin-inventory",
                    "name": controller.id,
                    "surface": "plugin-inventory",
                    "sourceId": controller_source_id,
                    "status": plugin_status
                });
                if let Some(code) = plugin_error {
                    plugin_outcome["errorCode"] = json!(code);
                }
                collection_outcomes.push(plugin_outcome);
                if plugin_status == "succeeded" {
                    for source in catalog["sources"].as_array().into_iter().flatten()
                        .filter(|source| source["kind"] == "plugin")
                    {
                        collection_outcomes.push(json!({
                            "id": format!("plugin:{}", source["id"].as_str().unwrap_or_default()),
                            "kind": "plugin",
                            "name": source["name"],
                            "surface": "plugin-metadata",
                            "sourceId": source["id"],
                            "revision": source["revision"],
                            "status": "succeeded"
                        }));
                    }
                }
            } else {
                collection_outcomes.push(json!({
                    "id": format!("controller:{}:plugin-inventory", controller.id),
                    "kind": "plugin-inventory",
                    "name": controller.id,
                    "surface": "plugin-inventory",
                    "sourceId": controller_source_id,
                    "status": "skipped-by-profile"
                }));
            }

            // Full GDSL extraction is part of standard/deep; quick only performs
            // the bounded one-byte availability probe in discover().
            if !controller_metadata_available {
                collection_outcomes.push(json!({
                    "id": format!("controller:{}:pipeline-gdsl", controller.id),
                    "kind": "pipeline-gdsl",
                    "name": controller.id,
                    "surface": "pipeline-gdsl",
                    "sourceId": controller_source_id,
                    "revision": catalog["sources"][0]["revision"],
                    "status": "succeeded"
                }));
            } else if profile.collection_profile == "quick" {
                collection_outcomes.push(json!({
                    "id": format!("controller:{}:pipeline-gdsl", controller.id),
                    "kind": "pipeline-gdsl",
                    "name": controller.id,
                    "surface": "pipeline-gdsl",
                    "sourceId": controller_source_id,
                    "status": "skipped-by-profile"
                }));
            } else {
                match jenkins_gdsl_collector::collect(
                    &controller.url,
                    username.as_deref(),
                    api_token.as_deref(),
                ) {
                    Ok(gdsl_catalog) => {
                        let gdsl_source_id = gdsl_catalog["sources"][0]["id"].as_str().unwrap_or_default();
                        if gdsl_source_id != controller_source_id {
                            return Err(BuildError::Source(format!(
                                "Jenkins controller '{}': GDSL source identity differs from controller snapshot",
                                controller.id
                            )));
                        }
                        if let Some(revision) = gdsl_catalog["sources"][0].get("revision") {
                            catalog["sources"][0]["revision"] = revision.clone();
                            update_collection_source_revision(
                                &mut collection_outcomes,
                                &controller_source_id,
                                revision,
                            );
                        }
                        if let (Some(target), Some(additional)) = (
                            catalog["symbols"].as_array_mut(),
                            gdsl_catalog["symbols"].as_array(),
                        ) {
                            target.extend(additional.iter().cloned());
                        }
                        if let Some(coverage) = catalog["coverage"].as_array_mut() {
                            if let Some(entry) = coverage.iter_mut().find(|entry| entry["surface"] == "pipeline-gdsl") {
                                entry["status"] = json!("inspected");
                                entry["details"] = json!("full GDSL document was downloaded and parsed");
                            }
                        }
                        collection_outcomes.push(json!({
                            "id": format!("controller:{}:pipeline-gdsl", controller.id),
                            "kind": "pipeline-gdsl",
                            "name": controller.id,
                            "surface": "pipeline-gdsl",
                            "sourceId": controller_source_id,
                            "revision": gdsl_catalog["sources"][0]["revision"],
                            "status": "succeeded"
                        }));
                    }
                    Err(error) => {
                        let (status, code) = collector_outcome(&error);
                        collection_outcomes.push(json!({
                            "id": format!("controller:{}:pipeline-gdsl", controller.id),
                            "kind": "pipeline-gdsl",
                            "name": controller.id,
                            "surface": "pipeline-gdsl",
                            "sourceId": controller_source_id,
                            "status": status,
                            "errorCode": code
                        }));
                    }
                }
            }
            if profile.collection_profile == "deep" && controller_metadata_available {
                let mut runtime_record = jenkins_gdsl_collector::collect_script_console_with_options(
                    &controller.url,
                    username.as_deref(),
                    api_token.as_deref(),
                    &controller.api_roots,
                    &controller.api_limits,
                );
                redact_sensitive_fields(&mut runtime_record);
                let runtime_status = runtime_record["status"].as_str().unwrap_or("unavailable");
                let runtime_error = runtime_record["errors"].as_array()
                    .and_then(|errors| errors.first())
                    .and_then(|error| error["code"].as_str())
                    .map(str::to_owned);
                let (status, error_code) = match runtime_status {
                    "collected" => ("succeeded", None),
                    "partial" => ("partial", runtime_error.clone()),
                    _ if runtime_error.as_deref() == Some("permission-denied") => ("permission-denied", runtime_error.clone()),
                    _ => ("unavailable", runtime_error.clone().or_else(|| Some("probe-failed".into()))),
                };
                let observed_symbols = runtime_symbols(&runtime_record, &controller_source_id);
                if let Some(symbols) = catalog["symbols"].as_array_mut() {
                    symbols.extend(observed_symbols);
                }
                catalog["controller"]["runtime"] = runtime_record;
                let mut outcome = json!({
                    "id": format!("controller:{}:script-console", controller.id),
                    "kind": "script-console",
                    "name": controller.id,
                    "surface": "script-console",
                    "sourceId": controller_source_id,
                    "status": status
                });
                if let Some(code) = error_code {
                    outcome["errorCode"] = json!(code);
                }
                collection_outcomes.push(outcome);
            } else {
                collection_outcomes.push(json!({
                    "id": format!("controller:{}:script-console", controller.id),
                    "kind": "script-console",
                    "name": controller.id,
                    "surface": "script-console",
                    "sourceId": controller_source_id,
                    "status": "skipped-by-profile"
                }));
            }
            if controller_metadata_available {
                catalog["controller"]["collection"]["profile"] = json!(profile.collection_profile);
                catalog["controller"]["collection"]["requestCount"] = json!(match profile.collection_profile.as_str() {
                    "quick" => 3,
                    "standard" => 4,
                    "deep" => 5,
                    _ => 4
                });
                let java_version_status = java_metadata_status(
                    &profile.collection_profile,
                    &catalog["controller"]["runtime"],
                    "javaVersion",
                    "java-version",
                );
                let java_vendor_status = java_metadata_status(
                    &profile.collection_profile,
                    &catalog["controller"]["runtime"],
                    "javaVendor",
                    "java-vendor",
                );
                catalog["controller"]["collection"]["javaVersionStatus"] = json!(java_version_status);
                catalog["controller"]["collection"]["javaVendorStatus"] = json!(java_vendor_status);
            }
            let api_type_symbols = runtime_api_type_symbols(&catalog["controller"]["runtime"], &controller_source_id);
            if let Some(symbols) = catalog["symbols"].as_array_mut() { symbols.extend(api_type_symbols); }
            let declarative_runtime = catalog
                .get("controller")
                .and_then(|value| value.get("runtime"))
                .cloned();
            catalogs.push((format!("Jenkins controller '{}'", controller.id), catalog));

            if controller_metadata_available {
            let core_source_id = format!("jenkins-core-{}", controller.id);
            let core_catalog = json!({
                "schemaVersion": "1.0.0",
                "catalogId": core_source_id,
                "generatedAt": build_timestamp(),
                "sources": [{
                    "id": core_source_id,
                    "kind": "jenkins-core",
                    "name": format!("Jenkins Core ({})", controller.id),
                    "version": core_version,
                    "revision": core_version,
                    "metadata": {
                        "controllerSourceId": controller_source_id,
                        "evidence": "observed-controller-core-version"
                    }
                }],
                "symbols": []
            });
            collection_outcomes.push(json!({
                "id": format!("controller:{}:jenkins-core", controller.id),
                "kind": "jenkins-core",
                "name": controller.id,
                "surface": "core-version",
                "sourceId": core_source_id,
                "revision": core_version,
                "status": "succeeded"
            }));
            catalogs.push((format!("Jenkins Core for '{}'", controller.id), core_catalog));
            }

            let source_id = format!("declarative-model-{}", controller.id);
            let fragment = declarative_model::catalog_fragment(&source_id);
            let mut model_symbols = fragment["symbols"].as_array().cloned().unwrap_or_default();
            for symbol in &mut model_symbols {
                let local_id = symbol["id"].as_str().unwrap_or("unknown").to_owned();
                symbol["id"] = json!(format!("{source_id}:{local_id}"));
            }
            if let Some(runtime) = declarative_runtime.as_ref() {
                model_symbols.extend(declarative_model::runtime_symbols(runtime, &source_id));
            }
            let model_catalog = json!({
                "schemaVersion":"1.0.0","catalogId":source_id,"generatedAt":build_timestamp(),
                "sources":[{"id":source_id,"kind":"jenkins-controller","name":format!("Declarative Pipeline model ({})", controller.id),"revision":"model-v1"}],
                "symbols":model_symbols
            });
            collection_outcomes.push(json!({
                "id": format!("controller:{}:declarative-model", controller.id),
                "kind": "declarative-model",
                "name": controller.id,
                "surface": "declarative-model",
                "sourceId": source_id,
                "revision": "model-v1",
                "status": "succeeded"
            }));
            catalogs.push((format!("Declarative model for '{}'", controller.id), model_catalog));

            // Do not cache partial results: failed optional surfaces are retried next time.
            let cacheable = collection_outcomes[cached_outcome_start..].iter().all(|outcome|
                matches!(outcome["status"].as_str(), Some("succeeded" | "skipped-by-profile" | "not-configured"))
            );
            if cacheable {
                if let Some(fingerprint) = fingerprint {
                    let snapshot = ControllerCacheSnapshot {
                        fingerprint,
                        catalogs: catalogs[cached_catalog_start..].to_vec(),
                        outcomes: collection_outcomes[cached_outcome_start..].to_vec(),
                    };
                    write_controller_cache(&controller_cache, &snapshot)?;
                }
            }
        }
        for library in &profile.libraries {
            if profile.collection_profile == "quick" {
                collection_outcomes.push(json!({
                    "id": format!("shared-library:{}", library.name),
                    "kind": "shared-library",
                    "name": library.name,
                    "surface": "library-source",
                    "revision": library.revision,
                    "status": "skipped-by-profile"
                }));
                continue;
            }
            let config = shared_library_scanner::LibraryConfig {
                name: library.name.clone(),
                repository: library.url.clone(),
                reference: library.revision.clone(),
            };
            let result = if let Some(commit) = &library.commit {
                shared_library_scanner::scan_library_at(&config, commit)
            } else {
                shared_library_scanner::scan_library(&config)
            };
            match result {
                Ok(result) => {
                    let source = result.catalog["sources"].as_array()
                        .and_then(|sources| sources.first())
                        .ok_or_else(|| BuildError::Source(format!(
                            "Shared Library '{}' scanner returned no source",
                            library.name
                        )))?;
                    collection_outcomes.push(json!({
                        "id": format!("shared-library:{}", library.name),
                        "kind": "shared-library",
                        "name": library.name,
                        "surface": "library-source",
                        "sourceId": source["id"],
                        "revision": source["revision"],
                        "status": "succeeded"
                    }));
                    catalogs.push((format!("Shared Library '{}'", library.name), result.catalog));
                }
                Err(_) => {
                    collection_outcomes.push(json!({
                        "id": format!("shared-library:{}", library.name),
                        "kind": "shared-library",
                        "name": library.name,
                        "surface": "library-source",
                        "revision": library.revision,
                        "status": "failed",
                        "errorCode": "collection-failed"
                    }));
                }
            }
        }
        if profile.libraries.is_empty() {
            collection_outcomes.push(json!({
                "id": "shared-libraries",
                "kind": "shared-library",
                "name": "Shared Libraries",
                "surface": "library-source",
                "status": "not-configured"
            }));
        }
    }
    if catalogs.is_empty() && collection_outcomes.is_empty() {
        return Err(BuildError::Profile("configure at least one controller, library, or fixture catalog".into()));
    }

    let mut sources = Vec::new();
    let mut symbols = Vec::new();
    let mut coverage = Vec::new();
    let mut source_revisions = BTreeMap::new();
    let mut source_origins = BTreeMap::new();
    let mut provenance = BTreeMap::new();
    let mut controller_snapshot: Option<Value> = None;
    let mut controller_metadata: Option<Value> = None;
    let mut included_source_counts = BTreeMap::<PackageSection, usize>::new();
    for (label, mut catalog) in catalogs {
        // Remove secret-bearing metadata before validation, merging, or serialization.
        redact_sensitive_fields(&mut catalog);
        let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).expect("bundled catalog schema");
        validate_catalog(&catalog, &schema).map_err(|e| BuildError::Source(format!("{label}: {e}")))?;

        if selection.includes(PackageSection::Controller) {
            if let Some(controller) = catalog.get("controller") {
                let mut safe_controller = controller.clone();
                if !selection.includes(PackageSection::Plugins) {
                    safe_controller["plugins"] = json!([]);
                }
                if let Some(base_url) = safe_controller["identity"]["baseUrl"].as_str() {
                    safe_controller["identity"]["baseUrl"] = json!(source_security::redact_url(base_url));
                }
                if let Some(previous) = &controller_snapshot {
                    if previous != &safe_controller {
                        return Err(BuildError::Source(format!(
                            "{label}: controller snapshot differs from the other configured source snapshots; refusing to mix controller identities or revisions"
                        )));
                    }
                } else {
                    controller_snapshot = Some(safe_controller.clone());
                    controller_metadata = Some(safe_controller);
                }
            }
        }

        let mut included_source_ids = HashSet::new();
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let kind = source["kind"].as_str().unwrap_or_default();
            let section = PackageSection::for_source_kind(kind).ok_or_else(|| {
                BuildError::Source(format!("{label}: source has unsupported kind"))
            })?;
            if !selection.includes(section) {
                continue;
            }
            let mut safe_source = source.clone();
            if let Some(url) = source["url"].as_str() {
                safe_source["url"] = json!(source_security::redact_url(url));
            }
            let id = safe_source["id"].as_str().ok_or_else(|| BuildError::Source(format!("{label}: source has no id")))?;
            let revision = safe_source["revision"].as_str().unwrap_or("unknown");
            if let Some(previous) = source_origins.insert(id.to_owned(), (label.clone(), revision.to_owned())) {
                return Err(BuildError::Source(format!(
                    "source id '{id}' is declared by both '{}' (revision '{}') and '{}' (revision '{}'); source identities and revisions must not be overwritten during merge",
                    previous.0, previous.1, label, revision
                )));
            }
            included_source_ids.insert(id.to_owned());
            *included_source_counts.entry(section).or_default() += 1;
            source_revisions.insert(id.to_owned(), revision.to_owned());
            provenance.insert(id.to_owned(), json!({
                "name":safe_source["name"],
                "kind":safe_source["kind"],
                "revision":safe_source["revision"],
                "url":safe_source.get("url")
            }));
            sources.push(safe_source);
        }
        for symbol in catalog["symbols"].as_array().into_iter().flatten() {
            if !symbol["sourceId"]
                .as_str()
                .is_some_and(|source_id| included_source_ids.contains(source_id))
            {
                continue;
            }
            let mut safe_symbol = symbol.clone();
            if let Some(availability) = safe_symbol
                .get_mut("availability")
                .and_then(Value::as_object_mut)
            {
                let has_empty_plugin_refs = if let Some(plugin_source_ids) =
                    availability.get_mut("pluginSourceIds").and_then(Value::as_array_mut)
                {
                    plugin_source_ids.retain(|source_id| {
                        source_id
                            .as_str()
                            .is_some_and(|source_id| included_source_ids.contains(source_id))
                    });
                    plugin_source_ids.is_empty()
                } else {
                    false
                };
                if has_empty_plugin_refs {
                    availability.remove("pluginSourceIds");
                }
            }
            symbols.push(safe_symbol);
        }
        coverage.extend(
            catalog["coverage"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| {
                    entry["sourceId"]
                        .as_str()
                        .is_some_and(|source_id| included_source_ids.contains(source_id))
                })
                .cloned(),
        );
    }
    if selection.explicit {
        for section in &selection.included {
            if included_source_counts.get(section).copied().unwrap_or_default() == 0 {
                return Err(BuildError::Profile(format!(
                    "package.include: selector '{}' matched no sources",
                    section.as_str()
                )));
            }
        }
    }
    let included_source_ids = sources
        .iter()
        .filter_map(|source| source["id"].as_str())
        .collect::<HashSet<_>>();
    for outcome in &mut collection_outcomes {
        let kind = outcome["kind"].as_str().unwrap_or_default();
        let section = match kind {
            "controller" | "pipeline-gdsl" | "declarative-model" | "script-console" | "plugin-inventory" => Some(PackageSection::Controller),
            "jenkins-core" => Some(PackageSection::JenkinsCore),
            "shared-library" => Some(PackageSection::SharedLibraries),
            _ => None,
        };
        let source_excluded = outcome["sourceId"]
            .as_str()
            .is_some_and(|source_id| !included_source_ids.contains(source_id));
        let excluded = section.is_some_and(|section| !selection.includes(section))
            || (kind == "plugin-inventory" && !selection.includes(PackageSection::Plugins))
            || source_excluded;
        if excluded {
            outcome["status"] = json!("skipped-by-profile");
            if let Some(fields) = outcome.as_object_mut() {
                fields.remove("sourceId");
                fields.remove("revision");
                fields.remove("errorCode");
            }
        }
    }

    sources.sort_by_key(|v| v["id"].as_str().unwrap_or("").to_owned());
    symbols.sort_by_key(|v| v["id"].as_str().unwrap_or("").to_owned());
    coverage.sort_by_key(|v| {
        (
            v["sourceId"].as_str().unwrap_or("").to_owned(),
            v["surface"].as_str().unwrap_or("").to_owned(),
        )
    });
    let collection_completed_at = build_timestamp();
    for source in &mut sources {
        let source_id = source["id"].as_str().unwrap_or_default();
        source["collection"] = source_collection_summary(
            source_id,
            &collection_outcomes,
            &profile.collection_profile,
            &collection_started_at,
            &collection_completed_at,
        );
    }
    let all_sources_reported = sources.iter().all(|source| {
        source["collection"]["status"].as_str().is_some_and(|status| status != "unknown")
    });
    let reason_codes = completeness_reason_codes(&collection_outcomes);
    let completeness_status = if !all_sources_reported {
        "unknown"
    } else if !reason_codes.is_empty() {
        "partial"
    } else {
        "complete"
    };
    let mut catalog = json!({
        "schemaVersion":"1.0.0",
        "catalogId":profile.package.id.clone().unwrap_or_else(|| profile.profile.clone()),
        "generatedAt":build_timestamp(),
        "completeness":{"status":completeness_status,"reasonCodes":reason_codes},
        "sources":sources,
        "symbols":symbols,
        "coverage":coverage
    });
    if let Some(controller) = controller_metadata {
        catalog["controller"] = controller;
    }
    stamp_catalog_identity(&mut catalog).map_err(BuildError::InvalidCatalog)?;
    for source in catalog["sources"].as_array().into_iter().flatten() {
        if let (Some(id), Some(fingerprint)) =
            (source["id"].as_str(), source["fingerprint"].as_str())
        {
            if let Some(record) = provenance.get_mut(id) {
                record["fingerprint"] = json!(fingerprint);
            }
        }
    }
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).expect("bundled catalog schema");
    validate_catalog(&catalog, &schema).map_err(|e| BuildError::InvalidCatalog(e.to_string()))?;

    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| BuildError::Io(e.to_string()))?;
    let staging = tempfile::Builder::new()
        .prefix(".jenkins-knowledge-staging-")
        .tempdir_in(parent)
        .map_err(|e| BuildError::Io(format!("cannot create package staging directory: {e}")))?;
    let staging_path = staging.path();
    let catalog_bytes = format!("{}\n", serde_json::to_string_pretty(&catalog).map_err(|e| BuildError::Io(e.to_string()))?);
    fs::write(staging_path.join("catalog.json"), &catalog_bytes).map_err(|e| BuildError::Io(e.to_string()))?;
    let mut manifest = json!({
        "packageVersion":"1.0.0",
        "catalogSchemaVersion":"1.0.0",
        "profile":profile.profile,
        "createdAt":build_timestamp(),
        "catalog":"catalog.json",
        "sourceRevisions":source_revisions,
        "provenance":provenance,
        "checksums":{"catalog.json":hex(&Sha256::digest(catalog_bytes.as_bytes()))},
        "toolVersions":{"knowledge-builder":env!("CARGO_PKG_VERSION")},
        "collection": {
            "status": completeness_status,
            "profile": profile.collection_profile,
            "startedAt": collection_started_at,
            "completedAt": collection_completed_at,
            "sources": collection_outcomes
        }
    });
    redact_sensitive_fields(&mut manifest);
    validate_package_metadata(&catalog, &manifest)
        .map_err(|error| BuildError::InvalidCatalog(format!("manifest/catalog consistency failed: {error}")))?;
    let manifest_schema: Value = serde_json::from_str(include_str!("../../../schemas/manifest.schema.json"))
        .expect("bundled manifest schema");
    let manifest_validator = jsonschema::validator_for(&manifest_schema)
        .map_err(|e| BuildError::InvalidCatalog(format!("invalid manifest schema: {e}")))?;
    if let Some(error) = manifest_validator.iter_errors(&manifest).next() {
        return Err(BuildError::InvalidCatalog(format!("manifest: {error}")));
    }
    fs::write(staging_path.join("manifest.json"), format!("{}\n", serde_json::to_string_pretty(&manifest).map_err(|e| BuildError::Io(e.to_string()))?))
        .map_err(|e| BuildError::Io(e.to_string()))?;
    if output.exists() {
        return Err(BuildError::OutputExists(output.display().to_string()));
    }
    fs::rename(staging.path(), output)
        .map_err(|error| BuildError::Io(format!("cannot publish package {}: {error}", output.display())))?;
    Ok(())
}

fn discovery_outcome(error: &DiscoveryError) -> (&'static str, &'static str) {
    match error {
        DiscoveryError::AuthenticationFailed { .. } | DiscoveryError::Unavailable { status: 401 | 403, .. } =>
            ("permission-denied", "permission-denied"),
        DiscoveryError::Unavailable { .. } => ("unavailable", "http-unavailable"),
        DiscoveryError::RequestTimeout { .. } => ("failed", "request-timeout"),
        DiscoveryError::ResponseTooLarge { .. } => ("failed", "response-too-large"),
        DiscoveryError::MalformedJson { .. } | DiscoveryError::InvalidPluginInventory =>
            ("failed", "invalid-response"),
        DiscoveryError::TooManyPlugins { .. } | DiscoveryError::TooManyDependencies { .. } =>
            ("failed", "limit-exceeded"),
        DiscoveryError::InvalidCatalog(_) => ("failed", "invalid-catalog"),
        DiscoveryError::InvalidControllerUrl => ("failed", "invalid-controller-url"),
        DiscoveryError::CredentialsMustBePaired => ("failed", "credentials-must-be-paired"),
        DiscoveryError::RequestFailed { .. } => ("failed", "request-failed"),
    }
}

fn collector_outcome(error: &CollectorError) -> (&'static str, &'static str) {
    match error {
        CollectorError::AuthenticationFailed => ("permission-denied", "permission-denied"),
        CollectorError::Unavailable(_) => ("unavailable", "http-unavailable"),
        CollectorError::RequestTimeout => ("failed", "request-timeout"),
        CollectorError::ResponseTooLarge { .. } => ("failed", "response-too-large"),
        CollectorError::InvalidEncoding => ("failed", "invalid-encoding"),
        CollectorError::TooManySymbols { .. } | CollectorError::TooManyParameters { .. } => ("failed", "limit-exceeded"),
        CollectorError::EmptyResponse => ("failed", "empty-response"),
        CollectorError::MalformedGdsl(_) => ("failed", "malformed-response"),
        CollectorError::InvalidCatalog(_) => ("failed", "invalid-catalog"),
        CollectorError::InvalidControllerUrl => ("failed", "invalid-controller-url"),
        CollectorError::RequestFailed => ("failed", "request-failed"),
    }
}

fn java_metadata_status(
    profile: &str,
    runtime: &Value,
    field: &str,
    probe: &str,
) -> &'static str {
    if profile != "deep" {
        return "not-collected-by-profile";
    }
    if runtime
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return "available";
    }
    let probe_failed = runtime
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| {
            errors.iter().any(|error| {
                error["probe"] == probe && error["code"] == "probe-failed"
            })
        });
    if probe_failed {
        "probe-failed"
    } else {
        "unavailable"
    }
}

fn build_timestamp() -> String {
    if let Ok(epoch) = env::var("SOURCE_DATE_EPOCH") {
        if let Ok(seconds) = epoch.parse::<i64>() {
            if let Some(timestamp) = chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0) {
                return timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            }
        }
    }
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }


#[cfg(test)]
mod tests {
    use super::{build_package, collection_plan, java_metadata_status, runtime_api_type_symbols, runtime_symbols, valid_api_class_name, validate_profile, Profile};
    use serde_json::{json, Value};
    use std::{fs, path::Path};

    #[test]
    fn java_metadata_status_distinguishes_profile_and_probe_outcomes() {
        let available = json!({
            "javaVersion": "17.0.12",
            "javaVendor": "Eclipse Adoptium",
            "errors": []
        });
        assert_eq!(
            java_metadata_status("quick", &available, "javaVersion", "java-version"),
            "not-collected-by-profile"
        );
        assert_eq!(
            java_metadata_status("standard", &available, "javaVendor", "java-vendor"),
            "not-collected-by-profile"
        );
        assert_eq!(
            java_metadata_status("deep", &available, "javaVersion", "java-version"),
            "available"
        );
        assert_eq!(
            java_metadata_status("deep", &available, "javaVendor", "java-vendor"),
            "available"
        );

        let unavailable = json!({ "errors": [] });
        assert_eq!(
            java_metadata_status("deep", &unavailable, "javaVersion", "java-version"),
            "unavailable"
        );
        let failed = json!({
            "errors": [{ "probe": "java-version", "code": "probe-failed" }]
        });
        assert_eq!(
            java_metadata_status("deep", &failed, "javaVersion", "java-version"),
            "probe-failed"
        );
        let denied = json!({
            "errors": [{ "probe": "script-console", "code": "permission-denied" }]
        });
        assert_eq!(
            java_metadata_status("deep", &denied, "javaVendor", "java-vendor"),
            "unavailable"
        );
    }

    #[test]
    fn emits_distinct_controller_observed_symbols_for_duplicate_runtime_steps() {
        let runtime = json!({
            "collectedAt": "2026-11-26T00:00:00Z",
            "extensionRegistries": [{
                "name": "pipeline-steps",
                "items": [
                    {"name":"deploy","className":"plugin.OneStep","plugin":"one","parameters":[{"name":"target","type":"java.lang.String","required":true}]},
                    {"name":"deploy","className":"plugin.TwoStep","plugin":"two"}
                ]
            },{
                "name":"pipeline-globals",
                "items":[{"name":"env","className":"workflow.Env"}]
            }]
        });
        let symbols = runtime_symbols(&runtime, "controller:test");
        assert_eq!(symbols.len(), 3);
        assert_ne!(symbols[0]["id"], symbols[1]["id"]);
        assert_eq!(symbols[0]["kind"], "step");
        assert_eq!(symbols[0]["parameters"][0]["name"], "target");
        assert_eq!(symbols[1]["namespace"], "two");
        assert_eq!(symbols[2]["kind"], "global-variable");
        assert_eq!(symbols[0]["evidence"][0]["kind"], "controller-observed");
    }

    #[test]
    fn accepts_only_explicit_bounded_api_roots_in_profiles() {
        let profile: Profile = serde_yaml::from_str(
            "profile: api-roots-test\ncollectionProfile: deep\njenkins:\n  controllers:\n    - id: local\n      url: https://jenkins.example.test\n      apiRoots:\n        - hudson.model.Run\n        - org.example.Outer$Inner\n      apiLimits:\n        maxDepth: 1\n        maxTypes: 10\n        maxMembersPerType: 5\n        maxTotalMembers: 50\n"
        ).unwrap();
        validate_profile(&profile).unwrap();
        assert_eq!(profile.jenkins.controllers[0].api_limits.max_depth, 1);
        assert_eq!(profile.jenkins.controllers[0].api_limits.max_types, 10);
        let invalid: Profile = serde_yaml::from_str(
            "profile: api-roots-test\ncollectionProfile: deep\njenkins:\n  controllers:\n    - id: local\n      url: https://jenkins.example.test\n      apiRoots:\n        - java.lang.String;System.exit(0)\n"
        ).unwrap();
        assert!(validate_profile(&invalid).is_err());

        let invalid_limits: Profile = serde_yaml::from_str(
            "profile: api-roots-test\ncollectionProfile: deep\njenkins:\n  controllers:\n    - id: local\n      url: https://jenkins.example.test\n      apiLimits:\n        maxTypes: 41\n"
        ).unwrap();
        assert!(validate_profile(&invalid_limits).is_err());
    }

    #[test]
    fn validates_explicit_api_roots_as_bounded_class_names() {
        assert!(valid_api_class_name("hudson.model.Run"));
        assert!(valid_api_class_name("org.example.Outer$Inner"));
        assert!(!valid_api_class_name("java.lang.String;System.exit(0)"));
        assert!(!valid_api_class_name("java.lang.String/../../etc/passwd"));
        assert!(!valid_api_class_name("groovy.lang.Closure"));
        assert!(!valid_api_class_name("java.lang.String "));
    }

    #[test]
    fn emits_bounded_class_method_and_property_symbols() {
        let runtime = json!({
            "collectedAt": "2026-11-26T00:00:00Z",
            "apiTypes": [{
                "name": "hudson.model.Run", "className": "hudson.model.Run", "depth": 0,
                "rootCategories": ["configured"], "status": "available", "synthetic": false,
                "dynamic": false, "superClass": "hudson.model.AbstractBuild",
                "interfaces": ["hudson.model.Queue$Executable"], "plugin": "jenkins-core",
                "origin": "jenkins.war", "documentationPaths": ["help/Run.html"], "methodsTruncated": false, "propertiesTruncated": false,
                "methods": [
                    {"name":"getLog","declaringClass":"hudson.model.Run","signature":"getLog(int) -> java.util.List","returnType":"java.util.List","genericReturnType":"java.util.List<java.lang.String>","modifiers":"public","synthetic":false,"bridge":false,"varargs":false,"parameters":[{"name":"maxLines","type":"int","genericType":"int","namePresent":true}],"parametersTruncated":false},
                    {"name":"getLog","declaringClass":"hudson.model.Run","signature":"getLog(java.lang.String) -> java.util.List","returnType":"java.util.List","genericReturnType":"java.util.List<java.lang.String>","modifiers":"public","synthetic":false,"bridge":false,"varargs":false,"parameters":[{"name":"path","type":"java.lang.String","genericType":"java.lang.String","namePresent":true}],"parametersTruncated":false}
                ],
                "properties": [{"name":"number","readType":"int","accessors":["getNumber"],"confidence":"inferred"}]
            },{
                "name":"hudson.model.AbstractBuild","className":"hudson.model.AbstractBuild","depth":2,
                "rootCategories":["supertype-of:hudson.model.Run"],"status":"partial","synthetic":false,
                "dynamic":false,"superClass":"hudson.model.Run","interfaces":[],"membersStatus":"inaccessible",
                "methods":[],"properties":[],"methodsTruncated":false,"propertiesTruncated":false
            }]
        });
        let symbols = runtime_api_type_symbols(&runtime, "controller:test");
        assert_eq!(symbols.len(), 4);
        assert_eq!(symbols[0]["kind"], "class");
        assert_eq!(symbols[0]["superClass"], "hudson.model.AbstractBuild");
        assert_eq!(symbols[0]["interfaces"][0], "hudson.model.Queue$Executable");
        assert_eq!(symbols[0]["properties"][0]["confidence"], "inferred");
        assert_eq!(symbols[0]["documentationPaths"][0], "help/Run.html");
        assert_eq!(symbols[1]["name"], "getLog");
        assert_eq!(symbols[2]["name"], "getLog");
        assert_ne!(symbols[1]["id"], symbols[2]["id"]);
        assert_eq!(symbols[2]["parameters"][0]["namePresent"], true);
        assert_eq!(symbols[3]["confidence"], "unknown");
        assert_eq!(symbols[3]["typeStatus"], "partial");
        assert_eq!(symbols[3]["membersStatus"], "inaccessible");
    }

    #[test]
    fn preserves_collection_coverage_in_merged_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        catalog["coverage"] = json!([{
            "sourceId":"controller",
            "surface":"pipeline-steps",
            "status":"inspected"
        }]);
        fs::write(
            directory.path().join("fixture.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            "profile: coverage-test\nfixtures:\n  - fixture.json\n",
        )
        .unwrap();

        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        let merged: Value =
            serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
        assert_eq!(merged["coverage"][0]["surface"], "pipeline-steps");
        assert_eq!(merged["coverage"][0]["status"], "inspected");
        assert_eq!(merged["controller"]["identity"]["id"], "jenkins-ci");
    }

    #[test]
    fn writes_reconciled_source_revision_and_provenance_metadata() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            "profile: manifest-test\nfixtures:\n  - fixture.json\n",
        ).unwrap();
        fs::write(
            directory.path().join("fixture.json"),
            include_str!("../../../tests/fixtures/catalog-valid.json"),
        ).unwrap();

        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        let catalog: Value = serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
        let manifest: Value = serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
        for source in catalog["sources"].as_array().unwrap() {
            let id = source["id"].as_str().unwrap();
            assert_eq!(manifest["sourceRevisions"][id], source["revision"]);
            assert_eq!(manifest["provenance"][id]["name"], source["name"]);
            assert_eq!(manifest["provenance"][id]["kind"], source["kind"]);
            assert_eq!(manifest["provenance"][id]["revision"], source["revision"]);
        }
    }

    #[test]
    fn rejects_conflicting_source_revisions_in_fixture_merge() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = include_str!("../../../tests/fixtures/catalog-valid.json");
        let first: Value = serde_json::from_str(fixture).unwrap();
        let mut second = first.clone();
        second["catalogId"] = json!("second-fixture");
        second["sources"][0]["revision"] = json!("2.480.0");

        fs::write(
            directory.path().join("first.json"),
            serde_json::to_vec(&first).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("second.json"),
            serde_json::to_vec(&second).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            "profile: conflict-test\nfixtures:\n  - first.json\n  - second.json\n",
        )
        .unwrap();

        let error = build_package(
            &directory.path().join("profile.yml"),
            &directory.path().join("package"),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("source id 'controller' is declared by both"),
            "{message}"
        );
        assert!(message.contains("2.479.1"), "{message}");
        assert!(message.contains("2.480.0"), "{message}");
        assert!(!directory.path().join("package").exists());
    }

    #[test]
    fn production_profile_template_matches_the_strict_profile_contract() {
        let profile: Profile = serde_yaml::from_str(include_str!(
            "../../../profiles/production-pipelines.yml"
        ))
        .unwrap();
        let selection = validate_profile(&profile).unwrap();
        assert_eq!(selection.included.len(), 4);
        assert_eq!(profile.jenkins.controllers.len(), 1);
        assert!(profile.libraries.iter().all(|library| library.commit.is_none()));
    }

    #[test]
    fn rejects_credential_bearing_controller_urls_before_collection() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: unsafe
jenkins:
  controllers:
    - id: ci
      url: https://user:secret@jenkins.example.test/
"#,
        )
        .unwrap();
        let error = build_package(
            &directory.path().join("profile.yml"),
            &directory.path().join("package"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("credentials must be supplied through the runtime environment"));
        assert!(!directory.path().join("package").exists());
    }

    #[test]
    fn strips_request_modifiers_from_catalog_and_manifest_urls() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        catalog["sources"][1]["url"] = json!("https://github.example.test/team/library.git?ref=release#readme");
        fs::write(
            directory.path().join("fixture.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: redaction-test
fixtures:
  - fixture.json
"#,
        )
        .unwrap();

        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        let catalog = fs::read_to_string(output.join("catalog.json")).unwrap();
        let manifest = fs::read_to_string(output.join("manifest.json")).unwrap();
        let package = format!("{catalog}{manifest}");
        assert!(!package.contains("?ref=release"));
        assert!(package.contains("https://github.example.test/team/library.git"));
    }

    #[test]
    fn redacts_nested_secret_metadata_before_publishing_package() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        catalog["sources"][0]["metadata"] = json!({
            "safeLabel": "keep-this",
            "apiKey": "builder-secret-api",
            "nested": [{"credentialId": "builder-secret-credential"}],
            "endpoint": "https://builder:builder-secret-user@controller.example.test/api?access_token=builder-secret-token"
        });
        fs::write(
            directory.path().join("fixture.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            "profile: secret-redaction\nfixtures:\n  - fixture.json\n",
        )
        .unwrap();

        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        let catalog_text = fs::read_to_string(output.join("catalog.json")).unwrap();
        let manifest_text = fs::read_to_string(output.join("manifest.json")).unwrap();
        let serialized = format!("{catalog_text}{manifest_text}");
        assert!(!serialized.contains("builder-secret"));
        let package_catalog: Value = serde_json::from_str(&catalog_text).unwrap();
        assert_eq!(package_catalog["sources"][0]["metadata"]["safeLabel"], "keep-this");
        assert!(package_catalog["symbols"][0]["signature"].as_str().is_some());
        assert_eq!(
            package_catalog["sources"][0]["metadata"]["endpoint"],
            "https://controller.example.test/api"
        );
        assert!(package_catalog["sources"][0]["metadata"].get("apiKey").is_none());
        assert!(package_catalog["sources"][0]["metadata"]["nested"][0]
            .get("credentialId")
            .is_none());
    }

    #[test]
    fn rejects_malformed_fixture_json_without_publishing_a_package() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("broken.json"), "{ not json").unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: malformed
fixtures:
  - broken.json
"#,
        )
        .unwrap();
        let output = directory.path().join("package");
        assert!(build_package(&directory.path().join("profile.yml"), &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn rejects_mixed_controller_snapshots_before_merging_sources() {
        let directory = tempfile::tempdir().unwrap();
        let first: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        let mut second = first.clone();
        second["catalogId"] = json!("second-snapshot");
        second["controller"]["identity"]["id"] = json!("different-controller");
        fs::write(directory.path().join("first.json"), serde_json::to_vec(&first).unwrap()).unwrap();
        fs::write(directory.path().join("second.json"), serde_json::to_vec(&second).unwrap()).unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: snapshot-conflict
fixtures:
  - first.json
  - second.json
"#,
        )
        .unwrap();

        let error = build_package(
            &directory.path().join("profile.yml"),
            &directory.path().join("package"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("controller snapshot differs"));
        assert!(!directory.path().join("package").exists());
    }


    #[test]
    fn preserves_legacy_gdsl_only_controller_profiles_without_core_api() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                if index == 0 {
                    write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    ).unwrap();
                } else {
                    let body = "method(name: 'sh', type: 'String')";
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    ).unwrap();
                }
            }
        });

        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            format!(
                "profile: gdsl-only\ncollectionProfile: standard\njenkins:\n  controllers:\n    - id: ci\n      url: http://{address}/\npackage:\n  include:\n    - controller\n"
            ),
        ).unwrap();
        let profile_path = directory.path().join("profile.yml");
        let output = directory.path().join("package");
        build_package(&profile_path, &output).unwrap();
        let profile: Profile = serde_yaml::from_str(&fs::read_to_string(&profile_path).unwrap()).unwrap();
        let cache_path = super::controller_cache_path(directory.path(), &profile.jenkins.controllers[0], &profile).unwrap();
        assert!(!cache_path.exists(), "partial controller snapshots must not be cached");
        server.join().unwrap();

        let catalog: Value = serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
        let manifest: Value = serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
        assert!(catalog["controller"].is_null());
        assert!(catalog["symbols"].as_array().unwrap().iter().any(|symbol| symbol["name"] == "sh"));
        assert_eq!(manifest["collection"]["status"], "partial");
        assert!(manifest["collection"]["sources"].as_array().unwrap().iter().any(|outcome|
            outcome["surface"] == "controller-metadata" && outcome["status"] == "unavailable"));
        assert!(manifest["collection"]["sources"].as_array().unwrap().iter().any(|outcome|
            outcome["surface"] == "pipeline-gdsl" && outcome["status"] == "succeeded"));
    }

    #[test]
    fn preserves_controller_snapshot_when_optional_plugin_and_gdsl_surfaces_fail() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                if request.starts_with("GET /api/json") {
                    let body = r#"{"_class":"hudson.model.Hudson","nodeName":"ci","jenkinsVersion":"2.479.1"}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nX-Jenkins: 2.479.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    ).unwrap();
                } else {
                    let status = if request.starts_with("GET /pluginManager/api/json") {
                        "403 Forbidden"
                    } else {
                        "404 Not Found"
                    };
                    write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    ).unwrap();
                }
            }
        });

        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            format!(
                "profile: partial-controller\ncollectionProfile: standard\njenkins:\n  controllers:\n    - id: ci\n      url: http://{address}/\n"
            ),
        ).unwrap();
        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        server.join().unwrap();

        let catalog: Value = serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
        let manifest: Value = serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["collection"]["status"], "partial");
        assert_eq!(manifest["collection"]["profile"], "standard");
        assert_eq!(
            catalog["controller"]["collection"]["javaVersionStatus"],
            "not-collected-by-profile"
        );
        assert_eq!(
            catalog["controller"]["collection"]["javaVendorStatus"],
            "not-collected-by-profile"
        );
        assert!(catalog["sources"].as_array().unwrap().iter().any(|source| source["kind"] == "jenkins-controller"));
        assert!(catalog["sources"].as_array().unwrap().iter().any(|source| source["kind"] == "jenkins-core"));
        assert!(catalog["symbols"].as_array().unwrap().iter().any(|symbol| symbol["kind"] == "declarative-directive"));
        assert!(manifest["collection"]["sources"].as_array().unwrap().iter().any(|outcome|
            outcome["surface"] == "plugin-inventory" && outcome["status"] == "permission-denied"));
        assert!(manifest["collection"]["sources"].as_array().unwrap().iter().any(|outcome|
            outcome["surface"] == "pipeline-gdsl" && outcome["status"] == "unavailable"));
        assert!(!manifest.to_string().contains("authorization"));
    }



    #[test]
    fn controller_cache_round_trips_and_rejects_changed_fingerprints() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshot.json");
        let snapshot = super::ControllerCacheSnapshot {
            fingerprint: "fingerprint-a".into(),
            catalogs: vec![("controller".into(), serde_json::json!({"catalogId":"snapshot-a"}))],
            outcomes: vec![serde_json::json!({"status":"succeeded"})],
        };
        super::write_controller_cache(&path, &snapshot).unwrap();
        let cached = super::read_controller_cache(&path, "fingerprint-a").unwrap();
        assert_eq!(cached.catalogs[0].1["catalogId"], "snapshot-a");
        assert!(super::read_controller_cache(&path, "fingerprint-b").is_none());
    }

    #[test]
    fn controller_fingerprint_changes_when_core_or_plugin_inventory_changes() {
        let baseline = serde_json::json!({"controller": {
            "coreVersion": "2.479.1",
            "plugins": [{"shortName":"workflow-cps","version":"4000.v1"}],
            "capabilities": [{"name":"plugin-inventory","status":"available"}],
            "identity": {"baseUrl":"https://jenkins.example.test/"}
        }});
        let baseline_fingerprint = super::controller_fingerprint(&baseline).unwrap();
        let mut plugin_changed = baseline.clone();
        plugin_changed["controller"]["plugins"][0]["version"] = serde_json::json!("4000.v2");
        assert_ne!(baseline_fingerprint, super::controller_fingerprint(&plugin_changed).unwrap());
        let mut core_changed = baseline;
        core_changed["controller"]["coreVersion"] = serde_json::json!("2.480.0");
        assert_ne!(baseline_fingerprint, super::controller_fingerprint(&core_changed).unwrap());
    }

    #[test]
    fn collection_plan_is_network_free_and_lists_profile_specific_requests() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.yml");
        for (profile, expected_requests) in [("quick", 3), ("standard", 4), ("deep", 5)] {
            fs::write(&path, format!(
                "profile: plan-test\ncollectionProfile: {profile}\njenkins:\n  controllers:\n    - id: ci\n      url: https://jenkins.example.test/\n      apiRoots:\n        - hudson.model.Run\nlibraries:\n  - name: platform\n    url: https://git.example.test/platform.git\n    revision: main\n"
            )).unwrap();
            let plan = collection_plan(&path).unwrap();
            assert_eq!(plan["collectionProfile"], profile);
            assert_eq!(plan["requestCount"], expected_requests);
            assert_eq!(plan["networkRequests"].as_array().unwrap().len(), expected_requests);
            assert_eq!(plan["callsMade"], false);
            assert_eq!(plan["timeouts"]["requestsSequential"], true);
            assert_eq!(plan["credentials"]["valuesIncluded"], false);
            assert_eq!(plan["sharedLibraries"].as_array().unwrap().len(), usize::from(profile != "quick"));
            assert_eq!(plan["scriptConsole"].is_object(), profile == "deep");
            if profile == "deep" {
                assert_eq!(plan["networkRequests"][4]["method"], "POST");
                assert_eq!(plan["networkRequests"][4]["explicitOptIn"], "collectionProfile: deep");
                assert_eq!(plan["networkRequests"][4]["callerScriptAccepted"], false);
                assert_eq!(plan["scriptConsole"]["confirmationMethod"], "explicit collectionProfile: deep selection");
                assert_eq!(plan["networkRequests"][4]["apiLimits"]["maxTypes"], 40);
            } else {
                assert!(!plan.to_string().contains("/scriptText"));
            }
        }
    }

    #[test]
    fn fixture_collection_plan_never_schedules_network_requests() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile.yml");
        let plan = collection_plan(&path).unwrap();
        assert_eq!(plan["requestCount"], 0);
        assert_eq!(plan["networkRequests"].as_array().unwrap().len(), 0);
        assert_eq!(plan["callsMade"], false);
        assert_eq!(plan["offlineFixtures"][0], "catalog.json");
    }

    #[test]
    fn deep_fixture_plan_does_not_claim_a_script_console_request() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.yml");
        fs::write(&path, "profile: offline-deep\ncollectionProfile: deep\nfixtures:\n  - catalog.json\n").unwrap();
        let plan = collection_plan(&path).unwrap();
        assert_eq!(plan["requestCount"], 0);
        assert!(plan["scriptConsole"].is_null());
        assert_eq!(plan["callsMade"], false);
    }

    #[test]
    fn collection_profile_defaults_to_quick() {
        let profile: Profile = serde_yaml::from_str("profile: default-mode\nfixtures:\n  - fixture.json\n").unwrap();
        assert_eq!(profile.collection_profile, "quick");
    }

    #[test]
    fn standard_controller_cache_is_reused_refreshed_and_invalidated() {
        use std::{io::{Read, Write}, net::TcpListener, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..11 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 { break; }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") { break; }
                }
                let request = String::from_utf8_lossy(&request);
                let (status, content_type, body) = if request.starts_with("GET /api/json") {
                    ("200 OK", "application/json", r#"{"_class":"hudson.model.Hudson","nodeName":"ci","jenkinsVersion":"2.479.1"}"#)
                } else if request.starts_with("GET /pluginManager/api/json") {
                    ("200 OK", "application/json", r#"{"plugins":[]}"#)
                } else if request.to_ascii_lowercase().contains("range: bytes=0-0") {
                    ("206 Partial Content", "text/plain", "m")
                } else {
                    ("200 OK", "text/plain", "method(name: 'sh', type: 'String')")
                };
                write!(stream, "HTTP/1.1 {status}\r\nX-Jenkins: 2.479.1\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let profile_path = directory.path().join("profile.yml");
        fs::write(&profile_path, format!("profile: cache-test\ncollectionProfile: standard\njenkins:\n  controllers:\n    - id: ci\n      url: http://{address}/\n")).unwrap();
        let profile: Profile = serde_yaml::from_str(&fs::read_to_string(&profile_path).unwrap()).unwrap();
        let cache_path = super::controller_cache_path(directory.path(), &profile.jenkins.controllers[0], &profile).unwrap();
        super::build_package(&profile_path, &directory.path().join("first-package")).unwrap();
        super::build_package(&profile_path, &directory.path().join("cached-package")).unwrap();
        super::build_package_with_options(&profile_path, &directory.path().join("refreshed-package"), true).unwrap();
        server.join().unwrap();
        assert!(cache_path.is_file());
        assert_eq!(super::invalidate_cache(&profile_path).unwrap(), 1);
        assert!(!cache_path.exists());
    }

}
