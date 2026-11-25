use crate::Runtime;
use serde_json::Value;
use std::fmt::Write;

fn groovy_type(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        Some("boolean" | "Boolean") => "Boolean",
        Some("byte" | "Byte") => "Byte",
        Some("short" | "Short") => "Short",
        Some("int" | "Integer") => "Integer",
        Some("long" | "Long") => "Long",
        Some("float" | "Float") => "Float",
        Some("double" | "Double") => "Double",
        Some("char" | "Character") => "Character",
        Some("String") => "String",
        Some("Object") => "Object",
        Some("Number") => "Number",
        Some("Void") => "Void",
        Some("void") => "void",
        Some("def") => "def",
        Some("Map") => "Map",
        Some("List") => "List",
        Some("Set") => "Set",
        Some("Collection") => "Collection",
        Some("Iterable") => "Iterable",
        Some("Iterator") => "Iterator",
        Some("File") => "File",
        Some("URL") => "URL",
        Some("URI") => "URI",
        Some("Closure") => "Closure",
        Some("GString") => "GString",
        _ => "Object",
    }
}

fn identifier(value: &str, fallback: &str) -> String {
    let mut result: String = value.chars().enumerate().map(|(i, c)| {
        if (c.is_ascii_alphanumeric() || c == '_' || c == '$') && !(i == 0 && c.is_ascii_digit()) {
            c
        } else { '_' }
    }).collect();
    if result.is_empty() { result = fallback.into(); }
    if matches!(result.as_str(), "class" | "def" | "return" | "new" | "this" | "super"
        | "void" | "int" | "long" | "boolean" | "if" | "else" | "for" | "while") {
        result.push('_');
    }
    result
}

fn doc(out: &mut String, symbol: &Value) {
    let description = symbol["description"].as_str().unwrap_or("");
    let signature = symbol["signature"].as_str().unwrap_or("");
    let _ = writeln!(out, "    /**");
    if !description.is_empty() {
        for line in description.lines() {
            let safe = line.replace("*/", "* /");
            let _ = writeln!(out, "     * {}", safe.trim());
        }
    }
    if !signature.is_empty() {
        let _ = writeln!(out, "     * Catalog signature: {}", signature.replace("*/", "* /"));
    }
    if let Some(overloads) = symbol["overloads"].as_array() {
        for overload in overloads.iter().filter_map(Value::as_str) {
            let _ = writeln!(out, "     * Catalog overload: {}", overload.replace("*/", "* /"));
        }
    }
    if let Some(confidence) = symbol["confidence"].as_str() {
        let _ = writeln!(out, "     * Confidence: {confidence}");
    }
    let _ = writeln!(out, "     */");
}

fn params_from_signature(signature: &str) -> Option<Vec<(String, String)>> {
    let open = signature.find('(')?;
    let close = signature.rfind(')')?;
    if close < open { return None; }
    let raw = signature[open + 1..close].trim();
    if raw.is_empty() { return Some(Vec::new()); }
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (index, ch) in raw.char_indices() {
        match ch {
            '<' | '[' => depth += 1,
            '>' | ']' => depth -= 1,
            ',' if depth == 0 => { parts.push(raw[start..index].trim()); start = index + 1; }
            _ => {}
        }
    }
    parts.push(raw[start..].trim());
    Some(parts.iter().enumerate().map(|(index, part)| {
        if let Some((ty, name)) = part.rsplit_once(' ') {
            (groovy_type(Some(ty)).into(), identifier(name, &format!("arg{index}")))
        } else {
            ("Object".into(), format!("arg{index}"))
        }
    }).collect())
}

fn parameters(symbol: &Value, signature: Option<&str>) -> Vec<(String, String)> {
    if let Some(parsed) = signature.and_then(params_from_signature) { return parsed; }
    symbol["parameters"].as_array().into_iter().flatten().enumerate().map(|(index, p)| {
        (groovy_type(p["type"].as_str()).into(),
         identifier(p["name"].as_str().unwrap_or(""), &format!("arg{index}")))
    }).collect()
}

fn method(out: &mut String, symbol: &Value, name: &str, signature: Option<&str>, return_type: &str) {
    let args = parameters(symbol, signature).iter().map(|(ty, name)| format!("{ty} {name}"))
        .collect::<Vec<_>>().join(", ");
    let _ = writeln!(out, "    {return_type} {name}({args}) {{ }}");
}

fn render_symbol(out: &mut String, symbol: &Value) {
    doc(out, symbol);
    let kind = symbol["kind"].as_str().unwrap_or("");
    let name = identifier(symbol["name"].as_str().unwrap_or("Api"), "Api");
    let return_type = groovy_type(symbol["returnType"].as_str());
    let method_name = if kind == "global-variable" { "call".to_string() } else { name.clone() };
    method(out, symbol, &method_name, symbol["signature"].as_str(), return_type);
    if let Some(overloads) = symbol["overloads"].as_array() {
        for overload in overloads.iter().filter_map(Value::as_str) {
            method(out, symbol, &method_name, Some(overload), return_type);
        }
    }
}

pub fn render_groovy_stubs(runtime: &Runtime) -> String {
    let profile = identifier(runtime.profile(), "profile");
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by jenkins-knowledge. DO NOT EDIT.");
    let _ = writeln!(out, "// Profile: {}", runtime.profile());
    let _ = writeln!(out, "// Package version: {}", runtime.manifest()["packageVersion"].as_str().unwrap_or("unknown"));
    let _ = writeln!(out, "// Catalog: {}", runtime.catalog()["catalogId"].as_str().unwrap_or("unknown"));
    let _ = writeln!(out, "package jenkins.generated.{profile}\n");

    let mut steps: Vec<_> = runtime.symbols().iter().filter(|s| s["kind"] == "step").collect();
    steps.sort_by_key(|s| (s["name"].as_str().unwrap_or(""), s["id"].as_str().unwrap_or("")));
    if !steps.is_empty() {
        let _ = writeln!(out, "class PipelineSteps {{");
        for symbol in steps { render_symbol(&mut out, symbol); }
        let _ = writeln!(out, "}}\n");
    }

    let mut libraries: Vec<_> = runtime.symbols().iter().filter(|s| {
        let kind = s["kind"].as_str().unwrap_or("");
        let source_id = s["sourceId"].as_str();
        let is_library = runtime.sources().iter().any(|src| src["id"].as_str() == source_id && src["kind"] == "shared-library");
        is_library && matches!(kind, "global-variable" | "method" | "class" | "property")
    }).collect();
    libraries.sort_by_key(|s| (s["name"].as_str().unwrap_or(""), s["id"].as_str().unwrap_or("")));
    for symbol in libraries {
        if symbol["kind"] == "class" {
            doc(&mut out, symbol);
            let class_name = identifier(symbol["name"].as_str().unwrap_or("ApiType"), "ApiType");
            let _ = writeln!(out, "class {class_name} {{ }}\n");
        } else {
            let class_name = if symbol["kind"] == "global-variable" {
                identifier(symbol["name"].as_str().unwrap_or("Global"), "Global")
            } else {
                identifier(symbol["namespace"].as_str().unwrap_or("SharedLibraryApi"), "SharedLibraryApi")
            };
            let _ = writeln!(out, "class {class_name} {{");
            render_symbol(&mut out, symbol);
            let _ = writeln!(out, "}}\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_runtime() -> Runtime {
        let mut catalog: Value = serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json")).unwrap();
        catalog["symbols"][0]["overloads"] = json!(["sh(String script)"]);
        catalog["symbols"].as_array_mut().unwrap().push(json!({
            "id":"platform-lib:publish",
            "name":"publish",
            "kind":"method",
            "sourceId":"platform-lib",
            "namespace":"DeployUtils",
            "signature":"publish(String target)",
            "description":"Publishes a deployment.",
            "parameters":[{"name":"target","type":"String"}],
            "returnType":"String",
            "overloads":["publish(Map config)"],
            "confidence":"verified",
            "provenance":["src/DeployUtils.groovy"]
        }));
        catalog["symbols"].as_array_mut().unwrap().push(json!({
            "id":"platform-lib:DeploymentOptions",
            "name":"DeploymentOptions",
            "kind":"class",
            "sourceId":"platform-lib",
            "description":"Deployment configuration type.",
            "confidence":"verified",
            "provenance":["src/DeploymentOptions.groovy"]
        }));
        let manifest = json!({"packageVersion":"1.2.3","profile":"fixture","catalog":"catalog.json"});
        Runtime::from_validated_parts(manifest, catalog)
    }

    #[test]
    fn golden_stub_output_preserves_signatures_docs_and_profile() {
        let actual = render_groovy_stubs(&fixture_runtime());
        assert_eq!(actual, include_str!("../../../tests/fixtures/groovy-stubs.groovy"));
    }

    #[test]
    fn unknown_types_are_conservative_and_names_are_safe() {
        assert_eq!(groovy_type(Some("com.example.Unknown")), "Object");
        assert_eq!(identifier("bad-name", "fallback"), "bad_name");
    }

    #[test]
    fn overload_signatures_are_split_without_breaking_generics() {
        assert_eq!(params_from_signature("call(Map<String, Object> config, String env)").unwrap(),
            vec![("Object".into(), "config".into()), ("String".into(), "env".into())]);
    }
}
