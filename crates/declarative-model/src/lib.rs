use catalog_validator::validate_catalog;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Construct {
    pub name: &'static str,
    pub description: &'static str,
    pub contexts: &'static [&'static str],
    pub constraints: &'static [&'static str],
}

const CONSTRUCTS: &[Construct] = &[
    Construct { name: "pipeline", description: "Root of a Declarative Pipeline definition.", contexts: &["jenkinsfile"], constraints: &["Must be the root Declarative Pipeline construct.", "Contains agent, stages, and optional pipeline-level directives."] },
    Construct { name: "agent", description: "Selects the executor for a Pipeline or stage.", contexts: &["pipeline", "stage"], constraints: &["Required at pipeline level unless agent none is selected.", "A stage-level agent overrides the pipeline-level agent."] },
    Construct { name: "stages", description: "Groups the ordered stages of a Pipeline.", contexts: &["pipeline"], constraints: &["Must be a direct child of pipeline.", "Contains stage declarations."] },
    Construct { name: "stage", description: "Defines a named unit of work in a Pipeline.", contexts: &["stages"], constraints: &["Must be a direct child of stages.", "Contains steps and may contain stage-level directives or nested stages."] },
    Construct { name: "steps", description: "Contains executable Pipeline steps.", contexts: &["stage"], constraints: &["Must be inside a stage.", "Contains executable steps rather than Declarative directives."] },
    Construct { name: "post", description: "Declares condition-specific actions after a Pipeline or stage completes.", contexts: &["pipeline", "stage"], constraints: &["Contains post-condition blocks such as always, success, failure, unstable, changed, fixed, regression, aborted, and cleanup."] },
    Construct { name: "when", description: "Controls whether a stage is executed.", contexts: &["stage"], constraints: &["Must be inside a stage.", "Supports condition directives; exact availability depends on Jenkins and installed plugins."] },
    Construct { name: "environment", description: "Defines environment variables for a Pipeline or stage.", contexts: &["pipeline", "stage"], constraints: &["Values are Groovy expressions or strings evaluated by Declarative Pipeline."] },
    Construct { name: "tools", description: "Selects configured tools for a Pipeline or stage.", contexts: &["pipeline", "stage"], constraints: &["Tool names and versions depend on Jenkins tool installations and plugins."] },
    Construct { name: "options", description: "Configures Pipeline or stage execution options.", contexts: &["pipeline", "stage"], constraints: &["Available options depend on Jenkins core and installed plugins."] },
    Construct { name: "parameters", description: "Declares build parameters for a Pipeline.", contexts: &["pipeline"], constraints: &["Parameter types and options depend on Jenkins core and installed plugins."] },
    Construct { name: "triggers", description: "Configures automatic Pipeline triggers.", contexts: &["pipeline"], constraints: &["Trigger directives depend on Jenkins core and installed plugins."] },
    Construct { name: "input", description: "Requests input before a stage proceeds.", contexts: &["stage"], constraints: &["Stage-level directive; may pause execution pending user input."] },
    Construct { name: "libraries", description: "Loads Shared Libraries for a Pipeline.", contexts: &["pipeline"], constraints: &["Availability and behavior depend on the Pipeline: Shared Groovy Libraries plugin."] },
    Construct { name: "parallel", description: "Runs nested stages in parallel.", contexts: &["stage"], constraints: &["Contains nested stage declarations; cannot be combined with matrix in the same stage."] },
    Construct { name: "matrix", description: "Expands a stage across axes of values.", contexts: &["stage"], constraints: &["Requires axes and nested stages.", "Cannot be combined with parallel in the same stage."] },
    Construct { name: "script", description: "Runs an imperative block inside Declarative Pipeline.", contexts: &["steps"], constraints: &["Must be inside steps.", "Use only for logic that cannot be expressed with Declarative syntax."] },
];

pub fn constructs() -> &'static [Construct] { CONSTRUCTS }

pub fn catalog_symbols(source_id: &str) -> Vec<Value> {
    CONSTRUCTS.iter().map(|item| json!({
        "id": format!("declarative:{}", item.name),
        "name": item.name,
        "kind": "declarative-directive",
        "sourceId": source_id,
        "description": item.description,
        "contexts": item.contexts,
        "constraints": item.constraints,
        "confidence": "inferred",
        "provenance": ["Declarative Pipeline construct model; verify against target Jenkins/plugin versions"]
    })).collect()
}

pub fn catalog_fragment(source_id: &str) -> Value {
    json!({ "symbols": catalog_symbols(source_id) })
}

pub fn runtime_symbols(runtime: &Value, source_id: &str) -> Vec<Value> {
    let mut symbols = Vec::new();
    for registry in runtime["extensionRegistries"].as_array().into_iter().flatten() {
        if registry["name"] != "declarative-extensions" {
            continue;
        }
        for (index, item) in registry["items"].as_array().into_iter().flatten().enumerate() {
            let (Some(category), Some(name), Some(class_name)) = (
                item["category"].as_str(),
                item["name"].as_str(),
                item["className"].as_str(),
            ) else {
                continue;
            };
            if !matches!(
                category,
                "agent" | "option" | "when" | "post-condition" | "trigger" | "parameter"
            ) {
                continue;
            }
            let name_kind = item["nameKind"].as_str().unwrap_or("implementation-class");
            let confidence = if name_kind == "dsl-token" {
                "verified"
            } else {
                "unknown"
            };
            let id = format!(
                "{source_id}:runtime:{category}:{name}:{class_name}:{index}"
            );
            let mut symbol = json!({
                "id": id,
                "name": name,
                "kind": "declarative-directive",
                "sourceId": source_id,
                "signature": class_name,
                "confidence": confidence,
                "provenance": ["jenkins-script-console", "declarative-extensions", category],
                "fieldEvidence": {
                    "name": confidence,
                    "implementationClass": "verified"
                },
                "extensionCategory": category,
                "nameKind": name_kind,
                "descriptorClass": item["descriptorClass"],
                "evidence": [{
                    "sourceId": source_id,
                    "kind": "controller-observed",
                    "confidence": confidence,
                    "provenance": "Registered Declarative or Jenkins descriptor",
                    "retrievedAt": runtime["collectedAt"],
                    "location": format!("script-console/declarative-extensions/{category}"),
                    "sourceKind": "jenkins-controller"
                }]
            });
            if let Some(plugin) = item["plugin"].as_str() {
                symbol["namespace"] = json!(plugin);
            }
            if let Some(display_name) = item["displayName"].as_str() {
                symbol["displayName"] = json!(display_name);
            }
            if let Some(contexts) = item["contexts"].as_array() {
                symbol["contexts"] = json!(contexts);
            }
            if let Some(parameters) = item["parameters"].as_array() {
                symbol["parameters"] = json!(parameters);
                symbol["fieldEvidence"]["parameters"] = json!("verified");
            }
            for field in [
                "stageOnly",
                "canUseInStage",
                "allowedChildrenCount",
                "invisible",
            ] {
                if let Some(value) = item.get(field) {
                    if value.is_boolean() || value.is_i64() {
                        symbol[field] = value.clone();
                    }
                }
            }
            symbols.push(symbol);
        }
    }
    symbols
}


pub fn validate_fragment(fragment: &Value, schema: &Value, source_id: &str) -> Result<(), String> {
    let symbols = fragment.get("symbols").and_then(Value::as_array)
        .ok_or_else(|| "fragment must contain a symbols array".to_string())?;
    let names: BTreeSet<_> = CONSTRUCTS.iter().map(|item| item.name).collect();
    for symbol in symbols {
        if symbol.get("kind").and_then(Value::as_str) != Some("declarative-directive") {
            return Err("Declarative constructs must use kind declarative-directive".into());
        }
        if symbol.get("sourceId").and_then(Value::as_str) != Some(source_id) {
            return Err("all constructs must reference the supplied source id".into());
        }
        let name = symbol.get("name").and_then(Value::as_str)
            .ok_or_else(|| "construct name is required".to_string())?;
        if !names.contains(name) {
            return Err(format!("unsupported Declarative construct '{name}'"));
        }
    }
    let catalog = json!({
        "schemaVersion": "1.0.0",
        "catalogId": "declarative-model-validation",
        "generatedAt": "2026-01-01T00:00:00Z",
        "sources": [{
            "id": source_id, "kind": "jenkins-controller",
            "name": "Declarative Pipeline model", "revision": "model-v1"
        }],
        "symbols": symbols
    });
    validate_catalog(&catalog, schema).map_err(|errors| errors.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).unwrap()
    }

    #[test]
    fn models_core_declarative_constructs_and_contexts() {
        let names: Vec<_> = constructs().iter().map(|item| item.name).collect();
        for name in ["pipeline", "agent", "stages", "stage", "steps", "post", "when"] {
            assert!(names.contains(&name), "missing {name}");
        }
        assert_eq!(constructs().iter().find(|item| item.name == "when").unwrap().contexts, &["stage"]);
        assert!(constructs().iter().find(|item| item.name == "agent").unwrap().contexts.contains(&"pipeline"));
    }

    #[test]
    fn emits_schema_valid_declarative_symbols() {
        let fragment = catalog_fragment("controller-2-479");
        validate_fragment(&fragment, &schema(), "controller-2-479").unwrap();
        assert!(fragment["symbols"].as_array().unwrap().iter()
            .all(|symbol| symbol["kind"] == "declarative-directive"));
    }

    #[test]
    fn maps_core_and_plugin_declarative_extensions_with_explicit_evidence() {
        let runtime = json!({
            "collectedAt": "2026-11-26T00:00:00Z",
            "extensionRegistries": [{
                "name": "declarative-extensions",
                "items": [
                    {
                        "category": "agent",
                        "name": "any",
                        "nameKind": "dsl-token",
                        "className": "org.jenkinsci.plugins.pipeline.modeldefinition.agent.impl.Any",
                        "contexts": ["pipeline", "stage"],
                        "plugin": "pipeline-model-definition"
                    },
                    {
                        "category": "agent",
                        "name": "kubernetes",
                        "nameKind": "dsl-token",
                        "className": "org.csanchez.jenkins.plugins.kubernetes.pipeline.KubernetesDeclarativeAgent",
                        "contexts": ["pipeline", "stage"],
                        "plugin": "kubernetes",
                        "parameters": [{"name": "label", "type": "java.lang.String"}]
                    },
                    {
                        "category": "parameter",
                        "name": "StringParameterDefinition",
                        "nameKind": "implementation-class",
                        "className": "hudson.model.StringParameterDefinition",
                        "contexts": ["pipeline"]
                    }
                ]
            }]
        });
        let symbols = runtime_symbols(&runtime, "declarative-model-controller");
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0]["name"], "any");
        assert_eq!(symbols[0]["confidence"], "verified");
        assert_eq!(symbols[1]["namespace"], "kubernetes");
        assert_eq!(symbols[1]["parameters"][0]["name"], "label");
        assert_eq!(symbols[2]["confidence"], "unknown");
        assert_eq!(symbols[2]["fieldEvidence"]["name"], "unknown");
        assert_eq!(symbols[0]["evidence"][0]["kind"], "controller-observed");
    }

    #[test]
    fn refuses_wrong_source_and_unknown_constructs() {
        let mut fragment = catalog_fragment("controller");
        fragment["symbols"][0]["sourceId"] = json!("another-source");
        assert!(validate_fragment(&fragment, &schema(), "controller").unwrap_err().contains("source id"));

        let mut fragment = catalog_fragment("controller");
        fragment["symbols"][0]["name"] = json!("madeUpDirective");
        assert!(validate_fragment(&fragment, &schema(), "controller").unwrap_err().contains("unsupported"));
    }

    #[test]
    fn stage_context_rules_are_explicit() {
        let stage = constructs().iter().find(|item| item.name == "stage").unwrap();
        assert_eq!(stage.contexts, &["stages"]);
        assert!(stage.constraints.iter().any(|rule| rule.contains("direct child")));
    }
}
