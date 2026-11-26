use catalog_validator::{validate_catalog, validate_package_metadata};
use serde::Deserialize;
use serde::de::{Error as DeError, MapAccess, Visitor};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BinaryHeap},
    fmt,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub mod groovy;

pub const DEFAULT_SEARCH_LIMIT: usize = 50;
pub const MAX_SEARCH_RESULTS: usize = 100;
pub const MAX_SEARCH_OFFSET: usize = 10_000;
const MAX_MANIFEST_BYTES: u64 = 1_048_576;
const MAX_CHECKSUM_ENTRIES: usize = 10_000;
const MAX_PACKAGE_FILE_BYTES: u64 = 67_108_864;
const MAX_PACKAGE_BYTES: u64 = 268_435_456;
const MAX_CATALOG_SOURCES: usize = 10_000;
const MAX_CATALOG_SYMBOLS: usize = 100_000;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub controller: Option<String>,
    pub plugin: Option<String>,
    pub owner: Option<String>,
    pub type_name: Option<String>,
    pub confidence: Option<String>,
    pub prefix: Option<String>,
    pub contexts: Option<Vec<String>>,
    pub kinds: Option<Vec<String>>,
    pub source_kind: Option<String>,
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query: String::new(),
            kind: None,
            source: None,
            controller: None,
            plugin: None,
            owner: None,
            type_name: None,
            confidence: None,
            prefix: None,
            contexts: None,
            kinds: None,
            source_kind: None,
            limit: DEFAULT_SEARCH_LIMIT,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchPage {
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
    pub results: Vec<Value>,
}

#[derive(Debug)]
pub struct RuntimeError(pub String);
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RuntimeError {}

#[derive(Debug)]
pub struct Runtime {
    manifest: Value,
    catalog: Value,
    source_index: BTreeMap<String, usize>,
    symbol_index: BTreeMap<String, Vec<usize>>,
    member_index: BTreeMap<String, Vec<usize>>,
}

#[derive(Debug)]
struct UniqueChecksums(BTreeMap<String, Value>);

impl<'de> Deserialize<'de> for UniqueChecksums {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ChecksumsVisitor;

        impl<'de> Visitor<'de> for ChecksumsVisitor {
            type Value = UniqueChecksums;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with unique checksum file names")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut checksums = BTreeMap::new();
                while let Some((name, digest)) = map.next_entry::<String, Value>()? {
                    if checksums.len() >= MAX_CHECKSUM_ENTRIES {
                        return Err(M::Error::custom(format!("checksum entry count exceeds limit ({MAX_CHECKSUM_ENTRIES})")));
                    }
                    if checksums.insert(name.clone(), digest).is_some() {
                        return Err(M::Error::custom(format!(
                            "duplicate checksum entry '{name}'"
                        )));
                    }
                }
                Ok(UniqueChecksums(checksums))
            }
        }

        deserializer.deserialize_map(ChecksumsVisitor)
    }
}

#[derive(Deserialize)]
struct ManifestDocument {
    #[serde(flatten)]
    fields: serde_json::Map<String, Value>,
    checksums: UniqueChecksums,
}

impl ManifestDocument {
    fn into_value(self) -> Value {
        let mut manifest = self.fields;
        manifest.insert(
            "checksums".into(),
            Value::Object(self.checksums.0.into_iter().collect()),
        );
        Value::Object(manifest)
    }
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn from_validated_parts(manifest: Value, catalog: Value) -> Self {
        let source_index = build_source_index(&catalog);
        let (symbol_index, member_index) = build_symbol_indexes(&catalog);
        Self { manifest, catalog, source_index, symbol_index, member_index }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let path = path.as_ref();
        let manifest_path = if path.is_dir() {
            path.join("manifest.json")
        } else {
            path.to_path_buf()
        };
        let package_dir = manifest_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
        let manifest_document: ManifestDocument = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| RuntimeError(format!("invalid JSON in {}: {e}", manifest_path.display())))?;
        let manifest = manifest_document.into_value();

        let manifest_schema: Value = serde_json::from_str(include_str!(
            "../../../schemas/manifest.schema.json"
        ))
        .expect("bundled manifest schema");
        validate_schema(&manifest, &manifest_schema, "manifest")?;

        let catalog_name = manifest
            .get("catalog")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError("manifest.catalog must be a string".into()))?;
        let checksums = manifest
            .get("checksums")
            .and_then(Value::as_object)
            .ok_or_else(|| RuntimeError("manifest.checksums must be an object".into()))?;
        if checksums.len() > MAX_CHECKSUM_ENTRIES {
            return Err(RuntimeError(format!("checksum entry count exceeds limit ({MAX_CHECKSUM_ENTRIES})")));
        }
        if !checksums.contains_key(catalog_name) {
            return Err(RuntimeError(format!(
                "manifest catalog '{catalog_name}' has no checksum entry"
            )));
        }

        let mut aggregate_bytes = 0u64;
        add_package_bytes(&mut aggregate_bytes, manifest_bytes.len() as u64)?;
        let mut catalog_bytes = None;
        for (name, expected) in checksums {
            let expected = expected.as_str().ok_or_else(|| {
                RuntimeError(format!("checksum for '{name}' must be a SHA-256 hex string"))
            })?;
            if expected.len() != 64
                || !expected
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(RuntimeError(format!(
                    "checksum for '{name}' must be 64 lowercase hexadecimal characters"
                )));
            }

            let file_path = resolve_package_file(package_dir, name)?;
            let contents = read_bounded(&file_path, MAX_PACKAGE_FILE_BYTES, name)?;
            add_package_bytes(&mut aggregate_bytes, contents.len() as u64)?;
            let actual = hex(&Sha256::digest(&contents));
            if actual != expected {
                return Err(RuntimeError(format!(
                    "checksum mismatch for package file '{name}'"
                )));
            }
            if name == catalog_name {
                catalog_bytes = Some(contents);
            }
        }

        let catalog_bytes = catalog_bytes.ok_or_else(|| {
            RuntimeError(format!("catalog '{catalog_name}' was not checksum-verified"))
        })?;
        let catalog: Value = serde_json::from_slice(&catalog_bytes)
            .map_err(|e| RuntimeError(format!("invalid JSON in catalog '{catalog_name}': {e}")))?;

        let source_count = catalog["sources"].as_array().map_or(0, Vec::len);
        if source_count > MAX_CATALOG_SOURCES {
            return Err(RuntimeError(format!("catalog source count exceeds limit ({MAX_CATALOG_SOURCES})")));
        }
        let symbol_count = catalog["symbols"].as_array().map_or(0, Vec::len);
        if symbol_count > MAX_CATALOG_SYMBOLS {
            return Err(RuntimeError(format!("catalog symbol count exceeds limit ({MAX_CATALOG_SYMBOLS})")));
        }

        let catalog_schema: Value =
            serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
                .expect("bundled catalog schema");
        validate_catalog(&catalog, &catalog_schema)
            .map_err(|e| RuntimeError(format!("invalid catalog '{catalog_name}': {e}")))?;
        validate_package_metadata(&catalog, &manifest)
            .map_err(|e| RuntimeError(format!("invalid package metadata: {e}")))?;
        let expected = manifest
            .get("catalogSchemaVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual = catalog
            .get("schemaVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if expected != actual {
            return Err(RuntimeError(format!(
                "package/catalog schema version mismatch: manifest declares '{expected}', catalog is '{actual}'"
            )));
        }
        let source_index = build_source_index(&catalog);
        let (symbol_index, member_index) = build_symbol_indexes(&catalog);
        Ok(Self { manifest, catalog, source_index, symbol_index, member_index })
    }

    pub fn profile(&self) -> &str {
        self.manifest["profile"].as_str().unwrap_or_default()
    }
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    pub fn catalog(&self) -> &Value {
        &self.catalog
    }
    pub fn symbols(&self) -> &[Value] {
        self.catalog["symbols"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn sources(&self) -> &[Value] {
        self.catalog["sources"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn source_detail(&self, id: &str) -> Option<Value> {
        let source = self.source_index.get(id).and_then(|index| self.sources().get(*index))?;
        let mut detail = source.clone();
        if let Some(object) = detail.as_object_mut() {
            object.insert("profile".into(), json!(self.profile()));
            object.insert("manifestProvenance".into(), self.manifest["provenance"].get(id).cloned()
                .unwrap_or_else(|| json!({"status":"unknown"})));
        }
        Some(detail)
    }

    pub fn source_details(&self) -> Vec<Value> {
        self.sources().iter().filter_map(|source| {
            source["id"].as_str().and_then(|id| self.source_detail(id))
        }).collect()
    }

    pub fn controller(&self) -> Option<&Value> {
        self.catalog.get("controller")
    }

    fn plugin_inventory(&self) -> &[Value] {
        self.catalog["controller"]["plugins"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn plugin_inventory_status(&self) -> &'static str {
        if let Some(capability) = self.controller().and_then(|controller| controller["capabilities"].as_array())
            .and_then(|capabilities| capabilities.iter().find(|item| {
                item["name"] == "plugin-manager-api" || item["name"] == "plugin-inventory"
            }))
        {
            return match capability["status"].as_str() {
                Some("available") => "available",
                Some("unavailable") => "unavailable",
                _ => "unknown",
            };
        }
        if self.controller().and_then(|controller| controller.get("plugins"))
            .and_then(Value::as_array).is_some_and(|plugins| !plugins.is_empty())
        {
            "available"
        } else {
            "unknown"
        }
    }

    pub fn plugins(&self) -> Vec<Value> {
        self.plugin_inventory().iter().map(|plugin| {
            let mut detail = plugin.clone();
            let source = plugin["sourceId"].as_str().and_then(|id| self.source_detail(id));
            if let Some(object) = detail.as_object_mut() {
                object.insert("source".into(), source.unwrap_or(Value::Null));
                object.insert("profile".into(), json!(self.profile()));
            }
            detail
        }).collect()
    }

    pub fn controller_details(&self) -> Option<Value> {
        let mut detail = self.controller()?.clone();
        let source = self.source_details().into_iter().find(|source| source["kind"].as_str() == Some("jenkins-controller"));
        if let Some(object) = detail.as_object_mut() {
            object.insert("source".into(), source.unwrap_or(Value::Null));
            object.insert("profile".into(), json!(self.profile()));
        }
        Some(detail)
    }

    /// Summarizes the environment recorded by this package without probing Jenkins.
    /// A caller may supply an independently observed target environment for an exact
    /// inventory comparison; a match is never treated as proof of runtime compatibility.
    pub fn compatibility_report(&self, target_environment: Option<&Value>) -> Value {
        let controller = self.controller();
        let core_version = controller
            .and_then(|value| value["coreVersion"].as_str())
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("unknown"));
        let runtime = controller.and_then(|value| value.get("runtime"));
        let controller_collection = controller.and_then(|value| value.get("collection"));
        let java_version = runtime
            .and_then(|value| value["javaVersion"].as_str())
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("unknown"));
        let java_vendor = runtime
            .and_then(|value| value["javaVendor"].as_str())
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("unknown"));
        let java_version_status = controller_collection
            .and_then(|value| value["javaVersionStatus"].as_str())
            .unwrap_or(if java_version.is_some() { "available" } else { "unknown" });
        let java_vendor_status = controller_collection
            .and_then(|value| value["javaVendorStatus"].as_str())
            .unwrap_or(if java_vendor.is_some() { "available" } else { "unknown" });
        let plugin_status = self.plugin_inventory_status();
        let package_plugins = plugin_version_map(self.plugin_inventory());
        let target = target_environment.filter(|value| value.is_object());
        let target_core = target.and_then(|value| value["coreVersion"].as_str());
        let target_java = target.and_then(|value| value["javaVersion"].as_str());
        let target_vendor = target.and_then(|value| value["javaVendor"].as_str());
        let target_plugins = target
            .and_then(|value| value["plugins"].as_array())
            .and_then(|plugins| plugin_version_map(plugins));
        let target_provided = target.is_some();

        let compare_value = |package: Option<&str>, target_value: Option<&str>| {
            let status = if !target_provided {
                "not-compared"
            } else {
                match (package, target_value) {
                    (Some(package), Some(target)) if package == target => "matched",
                    (Some(_), Some(_)) => "different",
                    _ => "unknown",
                }
            };
            json!({"status":status,"package":package,"target":target_value})
        };
        let core_comparison = compare_value(core_version, target_core);
        let java_comparison = compare_value(
            java_version.filter(|_| java_version_status == "available"),
            target_java,
        );
        let vendor_comparison = compare_value(
            java_vendor.filter(|_| java_vendor_status == "available"),
            target_vendor,
        );
        let plugin_comparison_status = if !target_provided {
            "not-compared"
        } else if plugin_status != "available" || package_plugins.is_none() || target_plugins.is_none() {
            "unknown"
        } else if package_plugins == target_plugins {
            "matched"
        } else {
            "different"
        };
        let plugin_comparison = json!({
            "status":plugin_comparison_status,
            "packageCount":self.plugin_inventory().len(),
            "targetCount":target.and_then(|value| value["plugins"].as_array()).map_or(Value::Null, |plugins| json!(plugins.len()))
        });

        let field_statuses = [
            core_comparison["status"].as_str().unwrap_or("unknown"),
            java_comparison["status"].as_str().unwrap_or("unknown"),
            vendor_comparison["status"].as_str().unwrap_or("unknown"),
            plugin_comparison_status,
        ];
        let comparison_status = if !target_provided {
            "not-compared"
        } else if field_statuses.contains(&"different") {
            "different"
        } else if field_statuses.iter().all(|status| *status == "matched") {
            "matched"
        } else {
            "unknown"
        };
        let completeness = self.catalog
            .get("completeness")
            .and_then(|value| value["status"].as_str())
            .or_else(|| self.collection().and_then(|value| value["status"].as_str()))
            .unwrap_or("unknown");
        let environment_status = if controller.is_none() {
            "unknown"
        } else if core_version.is_some()
            && java_version_status == "available"
            && java_vendor_status == "available"
            && plugin_status == "available"
            && package_plugins.is_some()
        {
            "available"
        } else if core_version.is_some() || java_version.is_some() || !self.plugin_inventory().is_empty() {
            "partial"
        } else {
            "unknown"
        };
        let compatibility_status = if comparison_status == "different" {
            "environment-different"
        } else if comparison_status == "matched" && completeness == "complete" && environment_status == "available" {
            "not-proven"
        } else {
            "unknown"
        };

        let mut warnings = Vec::new();
        if completeness != "complete" {
            warnings.push(json!({
                "code":if completeness == "partial" {"package-incomplete"} else {"package-completeness-unknown"},
                "completeness":completeness
            }));
        }
        if controller.is_none() {
            warnings.push(json!({"code":"controller-metadata-unavailable"}));
        }
        if core_version.is_none() {
            warnings.push(json!({"code":"package-core-version-unknown"}));
        }
        if java_version_status != "available" {
            warnings.push(json!({"code":"package-java-version-unavailable","status":java_version_status}));
        }
        if java_vendor_status != "available" {
            warnings.push(json!({"code":"package-java-vendor-unavailable","status":java_vendor_status}));
        }
        if plugin_status != "available" || package_plugins.is_none() {
            warnings.push(json!({"code":"package-plugin-inventory-unavailable","status":plugin_status}));
        }
        if !target_provided {
            warnings.push(json!({"code":"target-environment-not-provided"}));
        } else if comparison_status == "unknown" {
            warnings.push(json!({"code":"environment-comparison-incomplete"}));
        } else if comparison_status == "different" {
            warnings.push(json!({"code":"target-environment-differs"}));
        } else {
            warnings.push(json!({"code":"environment-match-does-not-prove-compatibility"}));
        }
        if let Some(outcomes) = self.collection().and_then(|value| value["sources"].as_array()) {
            for outcome in outcomes {
                let status = outcome["status"].as_str().unwrap_or_default();
                if !matches!(status, "succeeded") {
                    let mut warning = json!({
                        "code":format!("collection-{status}"),
                        "surface":outcome["surface"],
                        "name":outcome["name"],
                        "sourceId":outcome["sourceId"],
                        "errorCode":outcome["errorCode"]
                    });
                    if let Some(object) = warning.as_object_mut() {
                        object.retain(|_, value| !value.is_null());
                    }
                    warnings.push(warning);
                }
            }
        } else {
            warnings.push(json!({"code":"collection-outcomes-not-recorded"}));
        }

        json!({
            "status":compatibility_status,
            "environmentComparisonStatus":comparison_status,
            "package":{
                "profile":self.profile(),
                "createdAt":self.manifest["createdAt"],
                "catalogId":self.catalog["catalogId"],
                "snapshotId":self.catalog["snapshotId"],
                "completeness":completeness
            },
            "packageEnvironment":{
                "status":environment_status,
                "controller":controller.and_then(|value| value["identity"].clone().into()),
                "jenkinsCore":{"status":if core_version.is_some() {"available"} else {"unknown"},"version":core_version},
                "java":{
                    "versionStatus":java_version_status,
                    "version":java_version,
                    "vendorStatus":java_vendor_status,
                    "vendor":java_vendor
                },
                "plugins":{"status":plugin_status,"count":self.plugin_inventory().len()}
            },
            "targetEnvironment":{
                "status":if target_provided {"provided"} else {"not-provided"},
                "coreVersion":target_core,
                "javaVersion":target_java,
                "javaVendor":target_vendor,
                "pluginCount":plugin_comparison["targetCount"]
            },
            "comparison":{
                "jenkinsCore":core_comparison,
                "javaVersion":java_comparison,
                "javaVendor":vendor_comparison,
                "plugins":plugin_comparison
            },
            "warnings":warnings
        })
    }

    pub fn search(&self, query: &str, kind: Option<&str>, source: Option<&str>) -> Vec<Value> {
        self.search_with_options(&SearchOptions {
            query: query.to_owned(),
            kind: kind.map(str::to_owned),
            source: source.map(str::to_owned),
            ..SearchOptions::default()
        })
    }

    pub fn search_with_options(&self, options: &SearchOptions) -> Vec<Value> {
        self.search_page(options, 0).expect("zero offset is always valid").results
    }

    pub fn search_page(&self, options: &SearchOptions, offset: usize) -> Result<SearchPage, RuntimeError> {
        if offset > MAX_SEARCH_OFFSET {
            return Err(RuntimeError(format!("search offset exceeds limit ({MAX_SEARCH_OFFSET})")));
        }
        let query = options.query.to_lowercase();
        let prefix = options.prefix.as_deref().map(str::to_lowercase);
        let limit = options.limit.clamp(1, MAX_SEARCH_RESULTS);
        let selection_limit = offset + limit;
        let mut best = BinaryHeap::<(String, String, usize)>::with_capacity(selection_limit);
        let mut total = 0usize;

        for (index, symbol) in self.symbols().iter().enumerate() {
            let source_id = symbol["sourceId"].as_str().unwrap_or_default();
            let source = self.source_index.get(source_id).and_then(|index| self.sources().get(*index));
            let source_kind = source.and_then(|item| item["kind"].as_str()).unwrap_or_default();
            let matches_controller = options.controller.as_deref().is_none_or(|controller| {
                let identity = &self.catalog["controller"]["identity"];
                identity["id"].as_str() == Some(controller)
                    || identity["displayName"].as_str() == Some(controller)
                    || identity["baseUrl"].as_str() == Some(controller)
                    || (source_kind == "jenkins-controller" && source.is_some_and(|item| {
                        item["id"].as_str() == Some(controller) || item["name"].as_str() == Some(controller)
                    }))
            });
            let matches_plugin = options.plugin.as_deref().is_none_or(|plugin| {
                let declared_dependency = symbol["executionConstraints"]["pluginDependencies"]
                    .as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(plugin)))
                    || symbol["availability"]["pluginSourceIds"].as_array()
                        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(plugin)));
                (source_kind == "plugin" && (source_id == plugin
                    || source.is_some_and(|item| item["name"].as_str() == Some(plugin))))
                    || declared_dependency
            });
            let matches_owner = options.owner.as_deref().is_none_or(|owner| {
                ["owner", "namespace", "class", "type", "declaringClass"].iter()
                    .filter_map(|key| symbol.get(key).and_then(Value::as_str))
                    .any(|value| value == owner || value.starts_with(&format!("{owner}.")))
            });
            let matches_type = options.type_name.as_deref().is_none_or(|type_name| {
                let matches_name = |value: &str| value == type_name || value.contains(type_name);
                ["type", "returnType", "genericReturnType", "namespace", "class", "signature", "superClass", "declaringClass"]
                    .iter().filter_map(|key| symbol.get(key).and_then(Value::as_str)).any(matches_name)
                    || symbol["interfaces"].as_array().is_some_and(|values| values.iter().filter_map(Value::as_str).any(matches_name))
                    || symbol["parameters"].as_array().is_some_and(|values| values.iter().any(|parameter| {
                        ["type", "genericType"].iter().filter_map(|key| parameter.get(key).and_then(Value::as_str)).any(matches_name)
                    }))
                    || symbol["properties"].as_array().is_some_and(|values| values.iter().any(|property| {
                        ["readType", "writeType"].iter().filter_map(|key| property.get(key).and_then(Value::as_str)).any(matches_name)
                    }))
            });
            let matches_prefix = prefix.as_deref().is_none_or(|prefix| {
                symbol["name"].as_str().unwrap_or_default().to_lowercase().starts_with(prefix)
            });
            let matches_context = options.contexts.as_ref().is_none_or(|contexts| {
                let declared = symbol["contexts"].as_array();
                let execution = symbol["executionConstraints"]["contexts"].as_array();
                let has_context_metadata = declared.is_some_and(|values| !values.is_empty())
                    || execution.is_some_and(|values| !values.is_empty());
                has_context_metadata && contexts.iter().any(|context| {
                    declared.is_some_and(|values| values.iter().any(|value| value.as_str() == Some(context)))
                        || execution.is_some_and(|values| values.iter().any(|value| value.as_str() == Some(context)))
                })
            });
            let matches_kind = options.kinds.as_ref().is_none_or(|kinds| {
                kinds.iter().any(|kind| symbol["kind"].as_str() == Some(kind))
            });
            let matches_source_kind = options.source_kind.as_deref().is_none_or(|kind| source_kind == kind);
            let matches_query = query.is_empty()
                || ["name", "description", "signature", "namespace", "returnType", "genericReturnType", "superClass", "declaringClass"]
                    .iter().filter_map(|key| symbol.get(key).and_then(Value::as_str))
                    .any(|value| value.to_lowercase().contains(&query))
                || symbol["interfaces"].as_array().is_some_and(|values| values.iter().filter_map(Value::as_str).any(|value| value.to_lowercase().contains(&query)))
                || symbol["parameters"].as_array().is_some_and(|values| values.iter().any(|parameter| {
                    ["name", "type", "genericType"].iter().filter_map(|key| parameter.get(key).and_then(Value::as_str)).any(|value| value.to_lowercase().contains(&query))
                }))
                || symbol["properties"].as_array().is_some_and(|values| values.iter().any(|property| {
                    ["name", "readType", "writeType"].iter().filter_map(|key| property.get(key).and_then(Value::as_str)).any(|value| value.to_lowercase().contains(&query))
                }));
            let matches = matches_query
                && matches_prefix
                && matches_context
                && matches_kind
                && matches_source_kind
                && options.kind.as_deref().is_none_or(|value| symbol["kind"].as_str() == Some(value))
                && options.source.as_deref().is_none_or(|value| source_id == value)
                && matches_controller
                && matches_plugin
                && matches_owner
                && matches_type
                && options.confidence.as_deref().is_none_or(|value| symbol["confidence"].as_str() == Some(value));

            if matches {
                total += 1;
                let key = (
                    symbol["name"].as_str().unwrap_or_default().to_owned(),
                    symbol["id"].as_str().unwrap_or_default().to_owned(),
                    index,
                );
                if best.len() < selection_limit {
                    best.push(key);
                } else if best.peek().is_some_and(|largest| key < *largest) {
                    best.pop();
                    best.push(key);
                }
            }
        }

        let mut selected = best.into_vec();
        selected.sort_unstable();
        let results: Vec<_> = selected.into_iter().skip(offset)
            .map(|(_, _, index)| self.enrich(&self.symbols()[index]))
            .collect();
        let returned = results.len();
        Ok(SearchPage {
            offset,
            limit,
            total,
            has_more: total > offset.saturating_add(returned),
            results,
        })
    }

    pub fn coverage(&self) -> &[Value] {
        self.catalog["coverage"].as_array().map(Vec::as_slice).unwrap_or(&[])
    }

    /// Returns explicit package-level and per-source collection outcomes.
    /// Older packages without this additive manifest field return None.
    pub fn collection(&self) -> Option<&Value> {
        self.manifest.get("collection")
    }

    pub fn get(&self, name_or_id: &str) -> Result<Value, RuntimeError> {
        let matches = self.symbol_index.get(name_or_id).cloned().unwrap_or_default();
        match matches.as_slice() {
            [] => Err(RuntimeError(format!(
                "symbol '{name_or_id}' was not found"
            ))),
            [index] => Ok(self.enrich_details(&self.symbols()[*index])),
            many => Err(RuntimeError(format!(
                "symbol '{name_or_id}' is ambiguous ({} matches); use a symbol ID or filter by source",
                many.len()
            ))),
        }
    }

    fn enrich_details(&self, symbol: &Value) -> Value {
        let mut result = self.enrich(symbol);
        let class_name = qualified_name(symbol);
        let mut members = if symbol["kind"].as_str() == Some("class") {
            self.member_index.get(&class_name).or_else(|| self.member_index.get(symbol["name"].as_str().unwrap_or_default()))
                .into_iter().flatten().filter(|index| self.symbols()[**index]["kind"].as_str() != Some("class"))
                .map(|index| self.enrich_details(&self.symbols()[*index])).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if symbol["kind"].as_str() == Some("class") {
            if let Some(properties) = symbol["properties"].as_array() {
                let source = self.source_detail(symbol["sourceId"].as_str().unwrap_or_default())
                    .unwrap_or(Value::Null);
                for property in properties {
                    let mut member = property.clone();
                    if let Some(object) = member.as_object_mut() {
                        object.insert("kind".into(), json!("property"));
                        object.insert("sourceId".into(), symbol["sourceId"].clone());
                        object.insert("declaringClass".into(), json!(class_name));
                        object.insert("source".into(), source.clone());
                        object.insert("profile".into(), json!(self.profile()));
                        object.insert("provenance".into(), symbol["provenance"].clone());
                        object.insert("evidence".into(), symbol["evidence"].clone());
                        let mut references = Vec::new();
                        if let Some(name) = property["readType"].as_str() {
                            references.push(self.resolve_type_reference("readType", name, symbol));
                        }
                        if let Some(name) = property["writeType"].as_str() {
                            references.push(self.resolve_type_reference("writeType", name, symbol));
                        }
                        object.insert("typeReferences".into(), json!(references));
                    }
                    members.push(member);
                }
            }
        }
        let mut type_references = Vec::new();
        if let Some(name) = symbol["superClass"].as_str() {
            type_references.push(self.resolve_type_reference("superClass", name, symbol));
        }
        if let Some(interfaces) = symbol["interfaces"].as_array() {
            for name in interfaces.iter().filter_map(Value::as_str) {
                type_references.push(self.resolve_type_reference("interface", name, symbol));
            }
        }
        if let Some(name) = symbol["returnType"].as_str() {
            type_references.push(self.resolve_type_reference("returnType", name, symbol));
        }
        if let Some(name) = symbol["genericReturnType"].as_str() {
            type_references.push(self.resolve_type_reference("genericReturnType", name, symbol));
        }
        if let Some(parameters) = symbol["parameters"].as_array() {
            for parameter in parameters {
                if let Some(name) = parameter["type"].as_str() {
                    type_references.push(self.resolve_type_reference("parameter", name, symbol));
                }
                if let Some(name) = parameter["genericType"].as_str() {
                    type_references.push(self.resolve_type_reference("genericParameter", name, symbol));
                }
            }
        }
        if let Some(object) = result.as_object_mut() {
            if symbol["kind"].as_str() == Some("class") {
                let member_status = symbol["membersStatus"].as_str().unwrap_or_else(|| {
                    if symbol["methodsTruncated"] == true || symbol["propertiesTruncated"] == true { "partial" }
                    else if members.is_empty() { "unknown" }
                    else { "available" }
                });
                object.insert("members".into(), json!(members));
                object.insert("membersStatus".into(), json!(member_status));
            }
            object.insert("typeReferences".into(), json!(type_references));
            object.insert("executionConstraintsSummary".into(), self.execution_constraints_summary(symbol, true));
        }
        result
    }

    fn execution_constraints_summary(&self, symbol: &Value, include_plugin_details: bool) -> Value {
        let constraints = &symbol["executionConstraints"];
        let mut context_values = Vec::new();
        for values in [symbol["contexts"].as_array(), constraints["contexts"].as_array()].into_iter().flatten() {
            for value in values.iter().filter_map(Value::as_str) {
                if !context_values.iter().any(|existing| existing == value) {
                    context_values.push(value.to_owned());
                }
            }
        }
        let availability = symbol["availability"]["status"].as_str().unwrap_or("unknown");
        let mut plugin_ids = Vec::new();
        for ids in [constraints["pluginDependencies"].as_array(), symbol["availability"]["pluginSourceIds"].as_array()].into_iter().flatten() {
            for id in ids.iter().filter_map(Value::as_str) {
                if !plugin_ids.iter().any(|existing| existing == id) {
                    plugin_ids.push(id.to_owned());
                }
            }
        }
        let plugin_details = if include_plugin_details {
            plugin_ids.iter().map(|id| {
                let source = self.source_detail(id);
                let inventory = self.plugin_inventory().iter().find(|plugin| {
                    plugin["sourceId"].as_str() == Some(id.as_str()) || plugin["shortName"].as_str() == Some(id.as_str())
                });
                json!({
                    "id":id,
                    "status": if source.is_some() || inventory.is_some() { "known" } else { "unknown" },
                    "source":source,
                    "inventory":inventory
                })
            }).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        json!({
            "cps": constraints["cps"].as_str().unwrap_or("unknown"),
            "sandbox": constraints["sandbox"].as_str().unwrap_or("unknown"),
            "contexts": if context_values.is_empty() { json!({"status":"unknown","values":[]}) } else { json!({"status":"known","values":context_values}) },
            "pluginAvailability": {"status":availability,"pluginIds":plugin_ids,"plugins":plugin_details}
        })
    }

    fn resolve_type_reference(&self, role: &str, name: &str, owner: &Value) -> Value {
        let indices = self.symbol_index.get(name).into_iter().flatten()
            .filter(|index| self.symbols()[**index]["kind"].as_str() == Some("class"))
            .copied().collect::<Vec<_>>();
        let status = if indices.len() > 1 { "ambiguous" }
            else if let Some(index) = indices.first() {
                match self.symbols()[*index]["typeStatus"].as_str() {
                    Some("inaccessible") => "inaccessible",
                    Some("partial") => "partial",
                    Some("unresolved") => "unknown",
                    _ => "resolved",
                }
            } else if owner["typeStatus"].as_str() == Some("inaccessible") { "inaccessible" }
            else { "unknown" };
        json!({
            "role":role,
            "name":name,
            "status":status,
            "symbols":indices.into_iter().map(|index| self.enrich(&self.symbols()[index])).collect::<Vec<_>>()
        })
    }

    pub fn examples(&self, name_or_id: &str) -> Result<Value, RuntimeError> {
        let symbol = self.get(name_or_id)?;
        Ok(serde_json::json!({
            "id":symbol["id"], "name":symbol["name"],
            "examples":symbol["examples"].as_array().cloned().unwrap_or_default(),
            "source":symbol["source"], "profile":symbol["profile"], "confidence":symbol["confidence"]
        }))
    }

    pub fn pipeline_step(&self, name: &str) -> Result<Value, RuntimeError> {
        let matches: Vec<_> = self
            .symbols()
            .iter()
            .filter(|s| {
                s["kind"].as_str() == Some("step")
                    && (s["name"].as_str() == Some(name) || s["id"].as_str() == Some(name))
            })
            .collect();
        match matches.as_slice() {
            [] => Err(RuntimeError(format!(
                "Pipeline step '{name}' was not found"
            ))),
            [symbol] => Ok(self.enrich_details(symbol)),
            many => Err(RuntimeError(format!(
                "Pipeline step '{name}' is ambiguous ({} matches); use a symbol ID",
                many.len()
            ))),
        }
    }

    fn enrich(&self, symbol: &Value) -> Value {
        let source_id = symbol["sourceId"].as_str().unwrap_or_default();
        let source = self.source_detail(source_id);
        let mut result = symbol.clone();
        if let Some(object) = result.as_object_mut() {
            object.insert("source".into(), source.unwrap_or(Value::Null));
            object.insert("profile".into(), Value::String(self.profile().into()));
            object.insert("executionConstraintsSummary".into(), self.execution_constraints_summary(symbol, false));
            object.insert("controller".into(), self.controller()
                .and_then(|controller| controller.get("identity"))
                .cloned().unwrap_or(Value::Null));
        }
        result
    }
}


fn plugin_version_map(plugins: &[Value]) -> Option<BTreeMap<String, String>> {
    let mut versions = BTreeMap::new();
    for plugin in plugins {
        let short_name = plugin["shortName"].as_str().filter(|value| !value.is_empty())?;
        let version = plugin["version"].as_str().filter(|value| !value.is_empty())?;
        if versions.insert(short_name.to_owned(), version.to_owned()).is_some() {
            return None;
        }
    }
    Some(versions)
}

fn build_source_index(catalog: &Value) -> BTreeMap<String, usize> {
    catalog["sources"].as_array().into_iter().flatten().enumerate()
        .filter_map(|(index, source)| source["id"].as_str().map(|id| (id.to_owned(), index)))
        .collect()
}

fn qualified_name(symbol: &Value) -> String {
    if symbol["kind"].as_str() == Some("class") {
        if let Some(signature) = symbol["signature"].as_str().filter(|value| !value.is_empty()) {
            return signature.to_owned();
        }
    }
    let name = symbol["name"].as_str().unwrap_or_default();
    symbol["namespace"].as_str().filter(|namespace| !namespace.is_empty())
        .map(|namespace| format!("{namespace}.{name}"))
        .unwrap_or_else(|| name.to_owned())
}

fn build_symbol_indexes(catalog: &Value) -> (BTreeMap<String, Vec<usize>>, BTreeMap<String, Vec<usize>>) {
    let mut symbols_by_name = BTreeMap::<String, Vec<usize>>::new();
    let mut members_by_class = BTreeMap::<String, Vec<usize>>::new();
    for (index, symbol) in catalog["symbols"].as_array().into_iter().flatten().enumerate() {
        let qualified = qualified_name(symbol);
        let signature = (symbol["kind"].as_str() == Some("class")).then(|| symbol["signature"].as_str()).flatten();
        for key in [symbol["id"].as_str(), symbol["name"].as_str(), Some(qualified.as_str()), signature].into_iter().flatten() {
            let indices = symbols_by_name.entry(key.to_owned()).or_default();
            if indices.last().copied() != Some(index) { indices.push(index); }
        }
        if let Some(class) = symbol["declaringClass"].as_str() {
            members_by_class.entry(class.to_owned()).or_default().push(index);
        }
    }
    (symbols_by_name, members_by_class)
}

fn add_package_bytes(total: &mut u64, bytes: u64) -> Result<(), RuntimeError> {
    *total = total.saturating_add(bytes);
    if *total > MAX_PACKAGE_BYTES {
        return Err(RuntimeError(format!(
            "package exceeds aggregate size limit ({MAX_PACKAGE_BYTES} bytes)"
        )));
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, RuntimeError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| RuntimeError(format!("cannot inspect {label}: {e}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RuntimeError(format!("{label} must be a regular file (symbolic links are not allowed)")));
    }
    if metadata.len() > limit {
        return Err(RuntimeError(format!("{label} exceeds size limit ({limit} bytes)")));
    }
    let file = fs::File::open(path).map_err(|e| RuntimeError(format!("cannot read {label}: {e}")))?;
    let mut contents = Vec::with_capacity(metadata.len().min(limit) as usize);
    file.take(limit + 1).read_to_end(&mut contents)
        .map_err(|e| RuntimeError(format!("cannot read {label}: {e}")))?;
    if contents.len() as u64 > limit {
        return Err(RuntimeError(format!("{label} exceeds size limit ({limit} bytes)")));
    }
    Ok(contents)
}

fn resolve_package_file(package_dir: &Path, name: &str) -> Result<PathBuf, RuntimeError> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') {
        return Err(RuntimeError(format!(
            "unsafe checksum path '{name}': expected a relative package path"
        )));
    }
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RuntimeError(format!(
            "unsafe checksum path '{name}': absolute and traversal paths are not allowed"
        )));
    }

    let root = fs::canonicalize(package_dir).map_err(|e| {
        RuntimeError(format!(
            "cannot resolve package directory {}: {e}",
            package_dir.display()
        ))
    })?;
    let mut current = root.clone();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|e| {
            RuntimeError(format!("cannot access checksummed package file '{name}': {e}"))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(RuntimeError(format!(
                "unsafe checksum path '{name}': symbolic links are not allowed"
            )));
        }
    }
    let resolved = fs::canonicalize(&current).map_err(|e| {
        RuntimeError(format!("cannot resolve checksummed package file '{name}': {e}"))
    })?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err(RuntimeError(format!(
            "unsafe checksum path '{name}': expected a regular file inside the package"
        )));
    }
    Ok(resolved)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_schema(value: &Value, schema: &Value, label: &str) -> Result<(), RuntimeError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| RuntimeError(format!("invalid {label} schema: {e}")))?;
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RuntimeError(format!(
            "invalid {label}:\n- {}",
            errors.join("\n- ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::Mutex,
    };

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    fn runtime() -> Runtime {
        let manifest = serde_json::json!({"profile":"test"});
        let catalog: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json"))
                .unwrap();
        Runtime::from_validated_parts(manifest, catalog)
    }

    fn write_package(directory: &Path) {
        let catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        write_package_with_catalog(directory, &catalog);
    }

    fn write_package_with_catalog(directory: &Path, catalog: &Value) {
        let catalog_bytes = serde_json::to_vec(catalog).unwrap();
        fs::write(directory.join("catalog.json"), &catalog_bytes).unwrap();
        let checksum = hex(&Sha256::digest(&catalog_bytes));
        let mut source_revisions = serde_json::Map::new();
        let mut provenance = serde_json::Map::new();
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let Some(id) = source["id"].as_str() else { continue; };
            source_revisions.insert(id.into(), source["revision"].clone());
            provenance.insert(id.into(), serde_json::json!({
                "name": source["name"],
                "kind": source["kind"],
                "revision": source["revision"],
                "url": source.get("url").cloned().unwrap_or(Value::Null)
            }));
        }
        let manifest = serde_json::json!({
            "packageVersion": "1.0.0",
            "catalogSchemaVersion": "1.0.0",
            "profile": "test",
            "createdAt": "2026-01-01T00:00:00Z",
            "catalog": "catalog.json",
            "sourceRevisions": source_revisions,
            "provenance": provenance,
            "checksums": {"catalog.json": checksum},
            "toolVersions": {}
        });
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        ).unwrap();
    }

    #[test]
    fn rejects_malformed_manifest_json() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("manifest.json"), "{").unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("invalid JSON in"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links_for_checksummed_files() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let catalog = directory.path().join("catalog.json");
        let target = directory.path().join("catalog-copy.json");
        fs::rename(&catalog, &target).unwrap();
        std::os::unix::fs::symlink(&target, &catalog).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("symbolic links are not allowed"));
    }

    #[test]
    fn rejects_manifest_larger_than_limit_before_deserialization() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("manifest.json"), vec![b' '; (MAX_MANIFEST_BYTES + 1) as usize]).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("manifest exceeds size limit"));
    }

    #[test]
    fn rejects_oversized_checksummed_files_before_reading_them() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let oversized = directory.path().join("oversized.bin");
        fs::File::create(&oversized).unwrap().set_len(MAX_PACKAGE_FILE_BYTES + 1).unwrap();
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"]["oversized.bin"] = Value::String("0".repeat(64));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("oversized.bin exceeds size limit"));
    }

    #[test]
    fn rejects_excessive_checksum_entries_during_manifest_parsing() {
        let directory = tempfile::tempdir().unwrap();
        let mut checksums = serde_json::Map::new();
        for index in 0..=MAX_CHECKSUM_ENTRIES {
            checksums.insert(format!("file-{index:05}"), Value::String("0".repeat(64)));
        }
        fs::write(
            directory.path().join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({"checksums": checksums})).unwrap(),
        ).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("checksum entry count exceeds limit"));
    }

    #[test]
    fn rejects_catalogs_with_too_many_sources_or_symbols() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        catalog["sources"] = Value::Array(vec![Value::Null; MAX_CATALOG_SOURCES + 1]);
        write_package_with_catalog(directory.path(), &catalog);
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("catalog source count exceeds limit"));

        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        catalog["symbols"] = Value::Array(vec![Value::Null; MAX_CATALOG_SYMBOLS + 1]);
        write_package_with_catalog(directory.path(), &catalog);
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("catalog symbol count exceeds limit"));
    }

    #[test]
    fn search_with_small_limit_keeps_only_the_lexicographically_first_matches() {
        let mut runtime = runtime();
        let symbols = runtime.catalog["symbols"].as_array_mut().unwrap();
        for index in 0..10_000 {
            symbols.push(serde_json::json!({
                "id": format!("top-k:{index:05}"),
                "name": format!("match-{index:05}"),
                "kind": "method",
                "sourceId": "workflow-basic-steps",
                "confidence": "unknown",
                "provenance": ["test"]
            }));
        }
        let options = SearchOptions { query: "match-".into(), limit: 3, ..SearchOptions::default() };
        let results = runtime.search_with_options(&options);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["name"], "match-00000");
        assert_eq!(results[2]["name"], "match-00002");
    }

    #[test]
    fn opens_a_valid_checksummed_package() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let loaded = Runtime::open(directory.path()).unwrap();
        assert_eq!(loaded.profile(), "test");
    }

    #[test]
    fn accepts_legacy_package_without_additive_completeness_fields() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let loaded = Runtime::open(directory.path()).unwrap();

        assert!(loaded.catalog().get("coverage").is_none());
        assert!(loaded.catalog().get("snapshotId").is_none());
        assert!(loaded.catalog().get("completeness").is_none());
        assert!(loaded.collection().is_none());
    }

    #[test]
    fn rejects_unsupported_package_format_version() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["packageVersion"] = Value::String("2.0.0".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("/packageVersion"));
    }

    #[test]
    fn verifies_every_declared_package_file() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        fs::write(directory.path().join("metadata.txt"), "metadata").unwrap();
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"]["metadata.txt"] = Value::String("a".repeat(64));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("checksum mismatch for package file 'metadata.txt'"));
    }

    #[test]
    fn rejects_manifest_source_revision_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["sourceRevisions"]["platform-lib"] = Value::String("wrong-revision".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("/sourceRevisions/platform-lib"));
        assert!(error.contains("does not match catalog source revision"));
    }

    #[test]
    fn rejects_secret_fields_in_manifest_provenance_metadata() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["provenance"]["platform-lib"]["futureMetadata"] =
            serde_json::json!({"apiKey": "runtime-secret-value"});
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("secret-bearing fields"));
        assert!(!error.contains("runtime-secret-value"));
    }

    #[test]
    fn rejects_manifest_provenance_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["provenance"]["platform-lib"]["name"] = Value::String("different-library".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("/provenance/platform-lib/name"));
    }

    #[test]
    fn runtime_uses_catalog_semantics_for_evidence_and_field_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        catalog["symbols"][0]["fieldEvidence"] = serde_json::json!({"madeUpField":"verified"});
        write_package_with_catalog(directory.path(), &catalog);
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("field is not a schema-defined symbol attribute"));

        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        catalog["symbols"][0]["evidence"][0]["sourceId"] = Value::String("platform-lib".into());
        write_package_with_catalog(directory.path(), &catalog);
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("incompatible with source 'platform-lib'"));
    }

    #[test]
    fn rejects_catalog_tampering_before_json_parsing() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        fs::write(directory.path().join("catalog.json"), "not json").unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("checksum mismatch"));
    }

    #[test]
    fn rejects_missing_checksummed_files() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        fs::remove_file(directory.path().join("catalog.json")).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("cannot access checksummed package file"));
    }

    #[test]
    fn rejects_path_traversal_checksum_entries() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"]["../outside.json"] = Value::String("a".repeat(64));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("unsafe checksum path"));
    }

    #[test]
    fn rejects_malformed_checksum_values() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"]["catalog.json"] = Value::String("not-a-digest".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("invalid manifest"));
    }

    #[test]
    fn rejects_duplicate_checksum_entries() {
        let directory = tempfile::tempdir().unwrap();
        let catalog = include_str!("../../../tests/fixtures/catalog-valid.json");
        fs::write(directory.path().join("catalog.json"), catalog).unwrap();
        let checksum = hex(&Sha256::digest(catalog.as_bytes()));
        let manifest = format!(
            r#"{{"packageVersion":"1.0.0","catalogSchemaVersion":"1.0.0","profile":"test","createdAt":"2026-01-01T00:00:00Z","catalog":"catalog.json","sourceRevisions":{{}},"provenance":{{}},"checksums":{{"catalog.json":"{checksum}","catalog.json":"{checksum}"}},"toolVersions":{{}}}}"#
        );
        fs::write(directory.path().join("manifest.json"), manifest).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("duplicate checksum entry"));
    }

    #[test]
    fn rejects_catalog_without_a_checksum_entry() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"] = serde_json::json!({});
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("has no checksum entry"));
    }

    #[test]
    fn loads_bundled_schemas_from_an_unrelated_working_directory() {
        let _lock = CWD_LOCK.lock().unwrap();
        let package = tempfile::tempdir().unwrap();
        write_package(package.path());
        let unrelated = tempfile::tempdir().unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(unrelated.path()).unwrap();
        let result = Runtime::open(package.path());
        std::env::set_current_dir(previous).unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn exact_id_and_name_lookup() {
        let r = runtime();
        assert_eq!(r.get("deployApp").unwrap()["name"], "deployApp");
        assert_eq!(
            r.get("platform-lib:deployApp").unwrap()["id"],
            "platform-lib:deployApp"
        );
    }

    #[test]
    fn missing_symbol_is_actionable() {
        assert!(runtime()
            .get("missing")
            .unwrap_err()
            .to_string()
            .contains("not found"));
    }

    #[test]
    fn search_filters_by_kind_and_text() {
        assert_eq!(
            runtime()
                .search("deploy", Some("global-variable"), None)
                .len(),
            1
        );
        assert!(runtime().search("deploy", Some("step"), None).is_empty());
    }

    #[test]
    fn search_is_bounded_and_has_stable_order() {
        let mut runtime = runtime();
        let symbols = runtime.catalog["symbols"].as_array_mut().unwrap();
        for index in 0..10_000 {
            symbols.push(serde_json::json!({
                "id": format!("generated:{index:03}"),
                "name": format!("generated{index:03}"),
                "kind": "method",
                "sourceId": "workflow-basic-steps",
                "confidence": "unknown",
                "provenance": ["test"]
            }));
        }
        let options = SearchOptions { limit: 10, ..SearchOptions::default() };
        let first = runtime.search_with_options(&options);
        let second = runtime.search_with_options(&options);
        assert_eq!(first.len(), 10);
        assert_eq!(first, second);
        assert_eq!(first[0]["name"], "ShellStep");
    }

    #[test]
    fn search_filters_by_plugin_and_confidence() {
        let runtime = runtime();
        let plugin = SearchOptions {
            plugin: Some("workflow-basic-steps".into()),
            ..SearchOptions::default()
        };
        let results = runtime.search_with_options(&plugin);
        assert!(results.iter().any(|symbol| symbol["name"] == "sh"));
        assert!(results.iter().all(|symbol| {
            symbol["sourceId"] == "workflow-basic-steps"
                || symbol["executionConstraints"]["pluginDependencies"].as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "workflow-basic-steps"))
        }));

        let unknown_constraint = SearchOptions {
            kind: Some("class".into()),
            ..SearchOptions::default()
        };
        let unknown_result = runtime.search_with_options(&unknown_constraint);
        assert_eq!(unknown_result[0]["executionConstraintsSummary"]["cps"], "unknown");
        assert_eq!(unknown_result[0]["executionConstraintsSummary"]["contexts"]["status"], "unknown");

        let inferred = SearchOptions {
            confidence: Some("inferred".into()),
            ..SearchOptions::default()
        };
        let results = runtime.search_with_options(&inferred);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "deployApp");
    }

    #[test]
    fn search_filters_by_owner_and_type_metadata() {
        let mut runtime = runtime();
        let symbol = &mut runtime.catalog["symbols"][3];
        symbol["namespace"] = Value::String("org.jenkinsci.plugins.workflow".into());
        symbol["returnType"] = Value::String("java.lang.String".into());
        let options = SearchOptions {
            owner: Some("org.jenkinsci.plugins.workflow".into()),
            type_name: Some("String".into()),
            ..SearchOptions::default()
        };
        let results = runtime.search_with_options(&options);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "ShellStep");

        let parameter_type = SearchOptions {
            type_name: Some("Map".into()),
            ..SearchOptions::default()
        };
        let results = runtime.search_with_options(&parameter_type);
        assert!(results.iter().any(|symbol| symbol["name"] == "deployApp"));
    }

    #[test]
    fn ambiguous_names_are_reported() {
        let mut r = runtime();
        let mut duplicate = r.catalog["symbols"][2].clone();
        duplicate["id"] = Value::String("second-deploy".into());
        r.catalog["symbols"].as_array_mut().unwrap().push(duplicate);
        let (symbol_index, member_index) = build_symbol_indexes(&r.catalog);
        r.symbol_index = symbol_index;
        r.member_index = member_index;
        assert!(r
            .get("deployApp")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
    }

    #[test]
    fn detail_resolves_type_references_and_declared_members_from_catalog_evidence() {
        let mut runtime = runtime();
        let class_name = "org.jenkinsci.plugins.workflow.steps.durable_task.ShellStep";
        runtime.catalog["symbols"][3]["namespace"] = Value::String("workflow-basic-steps".into());
        runtime.catalog["symbols"][3]["signature"] = Value::String(class_name.into());
        runtime.catalog["symbols"][3]["superClass"] = Value::String("org.jenkinsci.plugins.workflow.steps.Step".into());
        runtime.catalog["symbols"][3]["properties"] = serde_json::json!([{"name":"label","readType":"java.lang.String","accessors":["getLabel"],"confidence":"inferred"}]);
        runtime.catalog["symbols"].as_array_mut().unwrap().push(serde_json::json!({
            "id":"workflow-basic-steps:Step:class",
            "name":"Step",
            "namespace":"org.jenkinsci.plugins.workflow.steps",
            "signature":"org.jenkinsci.plugins.workflow.steps.Step",
            "kind":"class",
            "sourceId":"workflow-basic-steps",
            "confidence":"verified",
            "provenance":["test"]
        }));
        runtime.catalog["symbols"].as_array_mut().unwrap().push(serde_json::json!({
            "id":"workflow-basic-steps:ShellStep:launch",
            "name":"launch",
            "kind":"method",
            "sourceId":"workflow-basic-steps",
            "declaringClass":class_name,
            "returnType":"java.lang.String",
            "confidence":"verified",
            "provenance":["test"]
        }));
        let (symbol_index, member_index) = build_symbol_indexes(&runtime.catalog);
        runtime.symbol_index = symbol_index;
        runtime.member_index = member_index;

        let detail = runtime.get(class_name).unwrap();
        assert!(detail["members"].as_array().unwrap().iter().any(|member| member["name"] == "launch"));
        let property = detail["members"].as_array().unwrap().iter().find(|member| member["name"] == "label").unwrap();
        assert_eq!(property["kind"], "property");
        assert_eq!(property["confidence"], "inferred");
        assert_eq!(property["typeReferences"][0]["name"], "java.lang.String");
        assert_eq!(detail["typeReferences"][0]["role"], "superClass");
        assert_eq!(detail["typeReferences"][0]["status"], "resolved");
        assert_eq!(detail["typeReferences"][0]["symbols"][0]["name"], "Step");
        assert_eq!(detail["membersStatus"], "available");
        let step_index = runtime.symbols().iter().position(|symbol| symbol["id"] == "workflow-basic-steps:Step:class").unwrap();
        runtime.catalog["symbols"][step_index]["typeStatus"] = json!("partial");
        let partial = runtime.get(class_name).unwrap();
        assert_eq!(partial["typeReferences"][0]["status"], "partial");
        runtime.catalog["symbols"][step_index]["typeStatus"] = json!("inaccessible");
        let inaccessible = runtime.get(class_name).unwrap();
        assert_eq!(inaccessible["typeReferences"][0]["status"], "inaccessible");
        assert_eq!(runtime.search_page(&SearchOptions { kind: Some("class".into()), ..SearchOptions::default() }, 0).unwrap().total, 2);
    }

    #[test]
    fn detail_reports_unknown_constraints_without_promoting_confidence() {
        let detail = runtime().get("deployApp").unwrap();
        assert_eq!(detail["confidence"], "inferred");
        assert_eq!(detail["profile"], "test");
        assert_eq!(detail["source"]["revision"], "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(detail["source"]["manifestProvenance"]["status"], "unknown");
        assert_eq!(detail["executionConstraintsSummary"]["cps"], "unknown");
        assert_eq!(detail["executionConstraintsSummary"]["sandbox"], "unknown");
        assert_eq!(detail["executionConstraintsSummary"]["contexts"]["status"], "known");
        assert_eq!(detail["executionConstraintsSummary"]["pluginAvailability"]["status"], "unknown");
        assert_eq!(detail["typeReferences"][0]["status"], "unknown");
    }


    #[test]
    fn detail_combines_context_evidence_and_resolves_plugin_availability() {
        let detail = runtime().get("sh").unwrap();
        let contexts = detail["executionConstraintsSummary"]["contexts"]["values"].as_array().unwrap();
        assert!(contexts.iter().any(|value| value == "steps"));
        assert!(contexts.iter().any(|value| value == "pipeline"));
        assert_eq!(detail["executionConstraintsSummary"]["pluginAvailability"]["status"], "available");
        assert_eq!(detail["executionConstraintsSummary"]["pluginAvailability"]["plugins"][0]["inventory"]["shortName"], "workflow-basic-steps");
    }

    #[test]
    fn controller_filter_and_context_filter_do_not_match_unknown_contexts() {
        let runtime = runtime();
        let controller = SearchOptions {
            controller: Some("jenkins-ci".into()),
            ..SearchOptions::default()
        };
        assert!(!runtime.search_with_options(&controller).is_empty());
        let other_controller = SearchOptions {
            controller: Some("another-controller".into()),
            ..SearchOptions::default()
        };
        assert!(runtime.search_with_options(&other_controller).is_empty());

        let unknown_context = SearchOptions {
            contexts: Some(vec!["steps".into()]),
            kinds: Some(vec!["class".into()]),
            ..SearchOptions::default()
        };
        assert!(runtime.search_with_options(&unknown_context).is_empty());
    }

    #[test]
    fn plugin_inventory_status_preserves_unavailable_capability_evidence() {
        let mut runtime = runtime();
        runtime.catalog["controller"]["capabilities"].as_array_mut().unwrap().push(
            json!({"name":"plugin-manager-api","status":"unavailable"})
        );
        assert_eq!(runtime.plugin_inventory_status(), "unavailable");
    }

    #[test]
    fn plugin_inventory_status_preserves_unknown_inventory_evidence() {
        let mut runtime = runtime();
        runtime.catalog["controller"]["capabilities"].as_array_mut().unwrap().push(
            json!({"name":"plugin-inventory","status":"unknown"})
        );
        assert_eq!(runtime.plugin_inventory_status(), "unknown");
    }

    #[test]
    fn runtime_exposes_controller_plugins_and_all_package_sources() {
        let runtime = runtime();
        assert_eq!(runtime.controller().unwrap()["coreVersion"], "2.479.1");
        assert_eq!(runtime.plugins().len(), 1);
        assert!(runtime.sources().iter().any(|source| source["kind"] == "shared-library"));
    }

    #[test]
    fn aggregate_package_size_accepts_boundary_and_rejects_first_byte_over() {
        let mut total = 0;
        add_package_bytes(&mut total, MAX_PACKAGE_BYTES).unwrap();
        assert_eq!(total, MAX_PACKAGE_BYTES);
        let error = add_package_bytes(&mut total, 1).unwrap_err().to_string();
        assert!(error.contains("aggregate size limit"));
    }

    #[test]
    fn paginated_search_has_stable_order_and_no_gaps_or_duplicates() {
        let mut runtime = runtime();
        let symbols = runtime.catalog["symbols"].as_array_mut().unwrap();
        for index in 0..23 {
            symbols.push(serde_json::json!({
                "id": format!("page:{index:03}"),
                "name": format!("page-{index:03}"),
                "kind": "step",
                "sourceId": "controller",
                "contexts": ["steps"],
                "confidence": "verified",
                "provenance": ["test"]
            }));
        }
        let options = SearchOptions { kind: Some("step".into()), limit: 7, ..SearchOptions::default() };
        let first = runtime.search_page(&options, 0).unwrap();
        let second = runtime.search_page(&options, 7).unwrap();
        let third = runtime.search_page(&options, 14).unwrap();
        let fourth = runtime.search_page(&options, 21).unwrap();
        assert_eq!(first.total, 24);
        assert!(first.has_more && second.has_more && third.has_more);
        assert!(!fourth.has_more);
        let ids: Vec<_> = [first, second, third, fourth].into_iter()
            .flat_map(|page| page.results.into_iter().map(|symbol| symbol["id"].as_str().unwrap().to_owned()))
            .collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), 24);
        assert_eq!(unique.len(), 24);
    }

    #[test]
    fn search_page_rejects_offsets_above_the_documented_ceiling() {
        let error = runtime().search_page(&SearchOptions::default(), MAX_SEARCH_OFFSET + 1)
            .unwrap_err().to_string();
        assert!(error.contains("search offset exceeds limit"));
    }

    #[test]
    fn prefix_context_kind_and_source_kind_filters_are_applied_before_pagination() {
        let runtime = runtime();
        let step = SearchOptions {
            prefix: Some("sh".into()),
            contexts: Some(vec!["steps".into()]),
            kinds: Some(vec!["step".into()]),
            limit: 1,
            ..SearchOptions::default()
        };
        let page = runtime.search_page(&step, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.results[0]["name"], "sh");

        let library = SearchOptions {
            prefix: Some("depl".into()),
            contexts: Some(vec!["pipeline".into()]),
            kinds: Some(vec!["global-variable".into()]),
            source_kind: Some("shared-library".into()),
            limit: 1,
            ..SearchOptions::default()
        };
        let page = runtime.search_page(&library, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.results[0]["name"], "deployApp");
    }

}
