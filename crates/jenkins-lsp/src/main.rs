use jenkins_runtime::{Runtime, SearchOptions, MAX_SEARCH_OFFSET, MAX_SEARCH_RESULTS};
use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{json, Value};
use std::{collections::HashMap, error::Error, path::{Path, PathBuf}, process};

fn log(connection: &Connection, message: impl Into<String>, kind: i64) {
    let notification = Notification::new(
        "window/logMessage".into(),
        json!({"type": kind, "message": message.into()}),
    );
    let _ = connection.sender.send(Message::Notification(notification));
}

fn open_runtime(settings: &Value, workspace_root: Option<&Path>) -> Result<Runtime, Box<dyn Error>> {
    let package = settings.get("packagePath").and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or("set jenkinsKnowledge.packagePath to a Knowledge Package directory")?;
    let package_path = PathBuf::from(package);
    let package_path = if package_path.is_absolute() {
        package_path
    } else if let Some(root) = workspace_root {
        root.join(package_path)
    } else {
        package_path
    };
    let runtime = Runtime::open(package_path)?;
    if let Some(profile) = settings.get("profile").and_then(Value::as_str) {
        if !profile.is_empty() && profile != runtime.profile() {
            return Err(format!("configured profile '{profile}' does not match package profile '{}'; select the matching package", runtime.profile()).into());
        }
    }
    Ok(runtime)
}

fn settings(params: &Value) -> &Value {
    let settings = params.get("settings").unwrap_or(params);
    settings.get("jenkinsKnowledge").unwrap_or(settings)
}

fn word_at(text: &str, line_number: usize, utf16_column: usize) -> Option<String> {
    let line = text.lines().nth(line_number)?;
    let mut byte_column = 0;
    let mut units = 0;
    for (index, character) in line.char_indices() {
        if units >= utf16_column { break; }
        units += character.len_utf16();
        byte_column = index + character.len_utf8();
    }
    if units < utf16_column { byte_column = line.len(); }

    let mut start = byte_column;
    while start > 0 {
        let previous = line[..start].char_indices().next_back()?;
        if !(previous.1.is_alphanumeric() || matches!(previous.1, '_' | '$')) { break; }
        start = previous.0;
    }
    let mut end = byte_column;
    while end < line.len() {
        let character = line[end..].chars().next()?;
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) { break; }
        end += character.len_utf8();
    }
    (start < end).then(|| line[start..end].to_string())
}

fn prefix_at(text: &str, line_number: usize, utf16_column: usize) -> String {
    let Some(line) = text.lines().nth(line_number) else { return String::new(); };
    let mut byte_column = 0;
    let mut units = 0;
    for (index, character) in line.char_indices() {
        if units + character.len_utf16() > utf16_column { break; }
        units += character.len_utf16();
        byte_column = index + character.len_utf8();
    }
    let mut start = byte_column;
    while start > 0 {
        let Some((index, character)) = line[..start].char_indices().next_back() else { break; };
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) { break; }
        start = index;
    }
    line[start..byte_column].to_owned()
}

const MAX_CONTEXT_SCAN_BYTES: usize = 1024 * 1024;

fn relevant_block(name: &str) -> Option<&'static str> {
    match name {
        "pipeline" | "stages" | "stage" | "steps" | "post" | "script" | "agent"
        | "environment" | "options" | "parameters" | "triggers" | "tools" | "when"
        | "parallel" | "matrix" | "axes" | "excludes" | "input" => {
            // The matched token is one of the static literals above.
            Some(match name {
                "pipeline" => "pipeline",
                "stages" => "stages",
                "stage" => "stage",
                "steps" => "steps",
                "post" => "post",
                "script" => "script",
                "agent" => "agent",
                "environment" => "environment",
                "options" => "options",
                "parameters" => "parameters",
                "triggers" => "triggers",
                "tools" => "tools",
                "when" => "when",
                "parallel" => "parallel",
                "matrix" => "matrix",
                "axes" => "axes",
                "excludes" => "excludes",
                "input" => "input",
                _ => unreachable!(),
            })
        }
        _ => None,
    }
}

fn skip_quoted_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let triple = bytes.get(start..start + 3) == Some(&[quote, quote, quote]);
    let delimiter = if triple { 3 } else { 1 };
    let mut index = start + delimiter;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if triple && bytes.get(index..index + 3) == Some(&[quote, quote, quote]) {
            return index + 3;
        } else if !triple && bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn skip_slashy_string(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == b'/' {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn skip_dollar_slashy_string(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index < bytes.len() {
        if bytes[index] == b'$' && (bytes.get(index + 1) == Some(&b'$') || bytes.get(index + 1) == Some(&b'/')) {
            index += 2;
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'$') {
            return index + 2;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn slashy_string_can_start(bytes: &[u8], index: usize) -> bool {
    let mut previous = index;
    while previous > 0 && bytes[previous - 1].is_ascii_whitespace() {
        previous -= 1;
    }
    if previous == 0 {
        return true;
    }
    if matches!(bytes[previous - 1], b'=' | b'(' | b'[' | b'{' | b',' | b':' | b';'
        | b'!' | b'?' | b'&' | b'|' | b'~' | b'>') {
        return true;
    }
    let end = previous;
    while previous > 0 && (bytes[previous - 1].is_ascii_alphanumeric()
        || matches!(bytes[previous - 1], b'_' | b'$')) {
        previous -= 1;
    }
    let word = &bytes[previous..end];
    word == b"return" || word == b"throw" || word == b"case"
        || word == b"in" || word == b"assert"
}

/// Return the relevant Declarative block stack before the cursor. This is a
/// bounded lexical recognizer, not a Groovy parser: it skips comments and all
/// Groovy string literal forms and only accepts block names at statement starts
/// outside map/list literals and ordinary Groovy closures.
#[derive(Clone, Debug)]
struct IdentifierOccurrence {
    name: String,
    start: usize,
    end: usize,
    line: usize,
    character: usize,
    end_character: usize,
    statement_start: bool,
    declarative: bool,
    context: Option<&'static str>,
}

fn scan_source(source: &str, collect_identifiers: bool) -> Option<(Vec<&'static str>, Vec<IdentifierOccurrence>)> {
    if source.len() > MAX_CONTEXT_SCAN_BYTES {
        return None;
    }
    let bytes = source.as_bytes();
    let mut stack: Vec<Option<&'static str>> = Vec::new();
    let mut candidate = None;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut statement_start = true;
    let mut last_significant = None;
    let mut index = 0usize;
    let mut identifiers = Vec::new();

    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len()
                    && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'$' if bytes.get(index + 1) == Some(&b'/') => {
                index = skip_dollar_slashy_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'$');
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'"');
            }
            b'/' if slashy_string_can_start(bytes, index) => {
                index = skip_slashy_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'\"');
            }
            b'\n' => {
                if parentheses == 0 && brackets == 0
                    && !matches!(last_significant, Some(b'.' | b'=' | b',' | b':' | b'(' | b'['
                        | b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'?' | b'!')) {
                    statement_start = true;
                    if last_significant == Some(b')') {
                        candidate = None;
                    }
                }
                index += 1;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            b'{' => {
                let declarative_parent = stack.iter().all(|block| {
                    block.is_some_and(|name| name != "script")
                });
                let opened = if declarative_parent && parentheses == 0 && brackets == 0 {
                    candidate.take()
                } else {
                    None
                };
                stack.push(opened);
                statement_start = true;
                last_significant = Some(b'{');
                index += 1;
            }
            b'}' => {
                stack.pop();
                candidate = None;
                statement_start = true;
                last_significant = Some(b'}');
                index += 1;
            }
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                statement_start = false;
                last_significant = Some(b'(');
                index += 1;
            }
            b')' => {
                parentheses = parentheses.saturating_sub(1);
                statement_start = false;
                last_significant = Some(b')');
                index += 1;
            }
            b'[' => {
                brackets = brackets.saturating_add(1);
                candidate = None;
                statement_start = false;
                last_significant = Some(b'[');
                index += 1;
            }
            b']' => {
                brackets = brackets.saturating_sub(1);
                candidate = None;
                statement_start = false;
                last_significant = Some(b']');
                index += 1;
            }
            b';' => {
                candidate = None;
                statement_start = true;
                last_significant = Some(b';');
                index += 1;
            }
            b':' => {
                candidate = None;
                statement_start = false;
                last_significant = Some(b':');
                index += 1;
            }
            b'.' | b'=' | b'+' | b'-' | b'*' | b'%' | b'!' | b'?' | b'&' | b'|' | b'~' => {
                candidate = None;
                statement_start = false;
                last_significant = Some(bytes[index]);
                index += 1;
            }
            byte if byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$') => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || matches!(bytes[index], b'_' | b'$'))
                {
                    index += 1;
                }
                let in_groovy_expression = parentheses > 0 || brackets > 0
                    || stack.iter().any(|block| block.is_none() || *block == Some("script"));
                if collect_identifiers {
                    if identifiers.len() >= 20_000 { return None; }
                    identifiers.push(IdentifierOccurrence {
                        name: source[start..index].to_owned(),
                        start,
                        end: index,
                        line: 0,
                        character: 0,
                        end_character: 0,
                        statement_start,
                        declarative: stack.contains(&Some("pipeline"))
                            && stack.iter().all(Option::is_some)
                            && !stack.contains(&Some("script")),
                        context: if stack.iter().any(Option::is_none) {
                            Some("scripted")
                        } else {
                            stack.last().copied().flatten().or(Some("scripted"))
                        },
                    });
                }
                if statement_start && !in_groovy_expression {
                    candidate = relevant_block(&source[start..index]);
                }
                statement_start = false;
                last_significant = Some(bytes[index - 1]);
            }
            _ => {
                candidate = None;
                statement_start = false;
                last_significant = Some(bytes[index]);
                index += 1;
            }
        }
    }

    if collect_identifiers {
        let mut line_starts = vec![0usize];
        for (position, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' { line_starts.push(position + 1); }
        }
        for identifier in &mut identifiers {
            let line_index = line_starts.partition_point(|start| *start <= identifier.start).saturating_sub(1);
            let line_start = line_starts[line_index];
            identifier.line = line_index;
            identifier.character = source[line_start..identifier.start].encode_utf16().count();
            identifier.end_character = identifier.character + source[identifier.start..identifier.end].encode_utf16().count();
        }
    }

    Some((stack.into_iter().flatten().collect(), identifiers))
}

fn scan_block_contexts(source: &str) -> Option<Vec<&'static str>> {
    scan_source(source, false).map(|(blocks, _)| blocks)
}

fn scan_identifiers(source: &str) -> Option<Vec<IdentifierOccurrence>> {
    scan_source(source, true).map(|(_, identifiers)| identifiers)
}

fn completion_context(text: &str, line: usize, character: usize) -> (String, Vec<String>, Vec<String>) {
    let prefix = prefix_at(text, line, character);
    let lines: Vec<_> = text.lines().take(line).collect();
    let mut before = lines.join("\n");
    if let Some(current) = text.lines().nth(line) {
        if !before.is_empty() { before.push('\n'); }
        let byte_column = current.char_indices()
            .scan(0usize, |units, (index, ch)| {
                let current_units = *units;
                *units += ch.len_utf16();
                Some((index, ch, current_units))
            })
            .take_while(|(_, ch, units)| *units + ch.len_utf16() <= character)
            .map(|(index, ch, _)| index + ch.len_utf8())
            .last().unwrap_or(0);
        before.push_str(&current[..byte_column]);
    }

    let blocks = scan_block_contexts(&before).unwrap_or_default();
    let in_script = blocks.contains(&"script");
    let in_steps = blocks.contains(&"steps");
    let in_post = blocks.contains(&"post");
    let declarative = blocks.contains(&"pipeline") && !in_script;
    let scripted = !declarative || in_script;
    let step_context = scripted || in_steps || in_post;
    let include_api_types = scripted;
    let mut contexts = vec!["pipeline".to_owned()];
    if scripted { contexts.push("scripted".into()); }
    for block in blocks {
        if block != "pipeline" && !contexts.iter().any(|context| context == block) {
            contexts.push(block.to_owned());
        }
    }
    let mut kinds = Vec::new();
    if step_context { kinds.push("step".into()); }
    kinds.push("global-variable".into());
    if declarative && !in_steps && !in_script { kinds.push("declarative-directive".into()); }
    if include_api_types {
        kinds.extend(["class".into(), "method".into(), "property".into(), "constructor".into()]);
    }
    (prefix, contexts, kinds)
}

fn supported_symbol(symbol: &Value) -> bool {
    let kind = symbol["kind"].as_str().unwrap_or_default();
    let source_kind = symbol["source"]["kind"].as_str().unwrap_or_default();
    matches!(kind, "step" | "declarative-directive" | "global-variable")
        || matches!(kind, "class" | "method" | "property" | "constructor")
            && matches!(source_kind, "plugin" | "jenkins-core" | "jenkins-controller" | "shared-library")
}

fn symbol_markdown(runtime: &Runtime, symbol: &Value) -> String {
    let detail = symbol["id"].as_str().and_then(|id| runtime.get(id).ok())
        .unwrap_or_else(|| symbol.clone());
    let name = detail["name"].as_str().unwrap_or("Unknown symbol");
    let kind = detail["kind"].as_str().unwrap_or("symbol");
    let signature = detail["signature"].as_str().unwrap_or("Signature unavailable");
    let description = detail["description"].as_str().unwrap_or("No description available.");
    let confidence = detail["confidence"].as_str().unwrap_or("unknown");
    let source = &detail["source"];
    let source_name = source["name"].as_str().or_else(|| detail["sourceId"].as_str()).unwrap_or("unknown source");
    let source_kind = source["kind"].as_str().unwrap_or("unknown kind");
    let revision = source["revision"].as_str().unwrap_or("unknown revision");
    let availability = detail["availability"]["status"].as_str().unwrap_or("unknown");
    let mut sections = vec![format!("**{name}** · {kind}"), format!("`{signature}`"), description.to_owned()];

    if let Some(parameters) = detail["parameters"].as_array().filter(|items| !items.is_empty()) {
        let rendered = parameters.iter().map(|parameter| {
            let name = parameter["name"].as_str().unwrap_or("arg");
            let ty = parameter["type"].as_str().or_else(|| parameter["genericType"].as_str()).unwrap_or("unknown type");
            let default = parameter["defaultValue"].as_str().map(|value| format!(" = `{value}`")).unwrap_or_default();
            let description = parameter["description"].as_str().map(|value| format!(" — {value}")).unwrap_or_default();
            format!("- `{name}: {ty}{default}`{description}")
        }).collect::<Vec<_>>().join("\n");
        sections.push(format!("**Parameters**\n{rendered}"));
    }
    if let Some(return_type) = detail["returnType"].as_str() {
        sections.push(format!("**Returns:** `{return_type}`"));
    }
    if let Some(overloads) = detail["overloads"].as_array().filter(|items| !items.is_empty()) {
        let rendered = overloads.iter().filter_map(Value::as_str).map(|value| format!("- `{value}`")).collect::<Vec<_>>().join("\n");
        if !rendered.is_empty() { sections.push(format!("**Overloads**\n{rendered}")); }
    }
    sections.push(format!("**Source:** {source_name} ({source_kind}) · revision `{revision}`"));
    if let Some(provenance) = source["manifestProvenance"]["name"].as_str() {
        sections.push(format!("**Source provenance:** {provenance}"));
    }
    sections.push(format!("**Availability:** {availability}"));
    let plugin_records = detail["executionConstraintsSummary"]["pluginAvailability"]["plugins"].as_array().cloned().unwrap_or_default();
    if !plugin_records.is_empty() {
        let rendered = plugin_records.iter().map(|plugin| {
            let inventory = &plugin["inventory"];
            let name = inventory["shortName"].as_str().or_else(|| inventory["name"].as_str()).unwrap_or_else(|| plugin["id"].as_str().unwrap_or("unknown plugin"));
            let version = inventory["version"].as_str().unwrap_or("unknown version");
            let active = inventory["active"].as_bool().map(|value| if value { "active" } else { "inactive" }).unwrap_or("state unknown");
            format!("- `{name}` · {version} · {active}")
        }).collect::<Vec<_>>().join("\n");
        let heading = if source_kind == "plugin" { "Plugin" } else { "Plugin dependencies" };
        sections.push(format!("**{heading}**\n{rendered}"));
    }
    if let Some(reason) = detail["availability"]["reason"].as_str() {
        sections.push(format!("**Availability evidence:** {reason}"));
    }
    let constraints = &detail["executionConstraintsSummary"];
    sections.push(format!("**CPS:** {} · **Sandbox:** {}",
        constraints["cps"].as_str().unwrap_or("unknown"),
        constraints["sandbox"].as_str().unwrap_or("unknown")));
    if let Some(contexts) = constraints["contexts"]["values"].as_array() {
        let contexts = contexts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
        if !contexts.is_empty() { sections.push(format!("**Contexts:** {contexts}")); }
    }
    if let Some(references) = detail["typeReferences"].as_array().filter(|items| !items.is_empty()) {
        let rendered = references.iter().map(|reference| format!("- `{}`: {} ({})",
            reference["role"].as_str().unwrap_or("type"),
            reference["name"].as_str().unwrap_or("unknown"),
            reference["status"].as_str().unwrap_or("unknown")))
            .collect::<Vec<_>>().join("\n");
        sections.push(format!("**Type references**\n{rendered}"));
    }
    if kind == "class" {
        let members = detail["members"].as_array().into_iter().flatten().take(30).map(|member| {
            format!("- `{}` ({})", member["signature"].as_str().or_else(|| member["name"].as_str()).unwrap_or("unknown member"), member["kind"].as_str().unwrap_or("member"))
        }).collect::<Vec<_>>().join("\n");
        sections.push(format!("**Members ({})**\n{}", detail["membersStatus"].as_str().unwrap_or("unknown"), if members.is_empty() { "No member declarations are available." } else { &members }));
    }
    if let Some(provenance) = detail["provenance"].as_array().filter(|items| !items.is_empty()) {
        let provenance = provenance.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
        sections.push(format!("**Provenance:** {provenance}"));
    }
    if let Some(fields) = detail["fieldEvidence"].as_object().filter(|fields| !fields.is_empty()) {
        let rendered = fields.iter().map(|(field, value)| format!("{field}: {}", value.as_str().unwrap_or("unknown"))).collect::<Vec<_>>().join(", ");
        sections.push(format!("**Field evidence:** {rendered}"));
    }
    sections.push(format!("**Confidence:** {confidence} · **Profile:** {}", runtime.profile()));
    sections.join("\n\n")
}

fn completion_for_document(
    runtime: Option<&Runtime>,
    uri: &str,
    text: &str,
    line: usize,
    character: usize,
) -> Value {
    if is_jenkinsfile_uri(uri) {
        completion(runtime, text, line, character)
    } else {
        json!({"isIncomplete": false, "items": []})
    }
}

fn completion(runtime: Option<&Runtime>, text: &str, line: usize, character: usize) -> Value {
    let Some(runtime) = runtime else { return json!({"isIncomplete": false, "items": []}); };
    let (prefix, contexts, kinds) = completion_context(text, line, character);
    let common = SearchOptions {
        prefix: Some(prefix),
        contexts: Some(contexts),
        limit: MAX_SEARCH_RESULTS,
        ..SearchOptions::default()
    };
    let primary = SearchOptions { kinds: Some(kinds.clone()), ..common.clone() };
    let mut library_kinds = vec!["global-variable".into()];
    let api_kinds = vec!["class".into(), "method".into(), "property".into(), "constructor".into()];
    let include_api_types = kinds.iter().any(|kind| kind == "class");
    if include_api_types { library_kinds.extend(api_kinds.clone()); }
    let library = SearchOptions {
        kinds: Some(library_kinds),
        source_kind: Some("shared-library".into()),
        ..common.clone()
    };
    let mut queries = vec![primary, library];
    if kinds.iter().any(|kind| kind == "step") {
        // Controller GDSL declarations are inherently Pipeline steps, but older
        // Jenkins GDSL responses do not carry explicit context metadata.
        queries.push(SearchOptions {
            kinds: Some(vec!["step".into()]),
            contexts: None,
            source_kind: Some("jenkins-controller".into()),
            ..common.clone()
        });
    }
    if include_api_types {
        queries.push(SearchOptions { kinds: Some(api_kinds.clone()), source_kind: Some("plugin".into()), ..common.clone() });
        queries.push(SearchOptions { kinds: Some(api_kinds.clone()), source_kind: Some("jenkins-core".into()), ..common.clone() });
        queries.push(SearchOptions { kinds: Some(api_kinds), source_kind: Some("jenkins-controller".into()), ..common });
    }
    let pages = queries.into_iter()
        .map(|options| runtime.search_page(&options, 0).expect("zero offset is valid"))
        .collect::<Vec<_>>();
    let mut is_incomplete = pages.iter().any(|page| page.has_more);
    let mut symbols = pages.into_iter().flat_map(|page| page.results).collect::<Vec<_>>();
    symbols.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str())
        .then_with(|| left["id"].as_str().cmp(&right["id"].as_str())));
    symbols.dedup_by(|left, right| left["id"] == right["id"]);
    symbols.retain(supported_symbol);
    is_incomplete |= symbols.len() > MAX_SEARCH_RESULTS;
    symbols.truncate(MAX_SEARCH_RESULTS);

    let items: Vec<Value> = symbols.iter().map(|symbol| {
        let name = symbol["name"].as_str().unwrap_or_default();
        let kind = symbol["kind"].as_str().unwrap_or_default();
        let lsp_kind = match kind {
            "step" => 3,
            "declarative-directive" => 14,
            "global-variable" => 6,
            "class" => 7,
            "property" => 10,
            _ => 2,
        };
        let signature = symbol["signature"].as_str().unwrap_or_default();
        let source = symbol["source"]["name"].as_str()
            .or_else(|| symbol["sourceId"].as_str()).unwrap_or("unknown source");
        json!({
            "label": name,
            "kind": lsp_kind,
            "detail": if signature.is_empty() { format!("{kind} · {source}") } else { format!("{signature} · {source}") },
            "documentation": {"kind": "markdown", "value": symbol_markdown(runtime, symbol)},
            "insertText": name,
            "filterText": name,
            "sortText": format!("{kind}-{name}"),
            "data": {"id": symbol["id"], "profile": runtime.profile()}
        })
    }).collect();
    json!({"isIncomplete": is_incomplete, "items": items})
}

fn collection_outcome(runtime: Option<&Runtime>) -> Value {
    runtime.and_then(Runtime::collection).cloned().unwrap_or_else(|| json!({
        "status": "unknown",
        "sources": [],
        "details": "This package predates explicit collection outcome metadata or no package is loaded"
    }))
}

fn hover(runtime: Option<&Runtime>, word: Option<String>) -> Value {
    let (Some(runtime), Some(word)) = (runtime, word) else { return Value::Null; };
    let options = SearchOptions { query: word.clone(), limit: MAX_SEARCH_RESULTS, ..SearchOptions::default() };
    let mut symbols: Vec<_> = runtime.search_with_options(&options).into_iter()
        .filter(|symbol| supported_symbol(symbol) && symbol["name"].as_str() == Some(word.as_str())).collect();
    symbols.sort_by(|left, right| left["signature"].as_str().cmp(&right["signature"].as_str())
        .then_with(|| left["id"].as_str().cmp(&right["id"].as_str())));
    if symbols.is_empty() { return Value::Null; }
    let sections = symbols.iter().map(|symbol| symbol_markdown(runtime, symbol)).collect::<Vec<_>>().join("\n\n---\n\n");
    json!({"contents": {"kind": "markdown", "value": sections}})
}

fn parse_source_location(value: &str) -> Option<(String, Option<usize>)> {
    let (path, line) = match value.rsplit_once(':') {
        Some((path, suffix)) => match suffix
            .split_whitespace()
            .next()
            .and_then(|line| line.parse::<usize>().ok())
        {
            Some(line) => (path, Some(line)),
            None => (value, None),
        },
        None => (value, None),
    };
    let path = path.trim_start_matches("./");
    let allowed_extension = [".groovy", ".java", ".kt", ".scala"].iter().any(|extension| path.ends_with(extension));
    if !allowed_extension || path.is_empty() || path.starts_with('/') || path.contains('\\')
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || !path.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some((path.to_owned(), line.filter(|line| *line > 0)))
}

fn source_location(symbol: &Value) -> Option<Value> {
    let source = &symbol["source"];
    if !matches!(source["kind"].as_str(), Some("shared-library" | "plugin" | "jenkins-core")) {
        return None;
    }
    let repository = source["url"].as_str()?.trim_end_matches('/').trim_end_matches(".git");
    let repository_path = repository.strip_prefix("https://github.com/")?;
    if repository.contains(['?', '#', '@'])
        || repository_path.split('/').count() != 2
        || repository_path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    let revision = source["revision"].as_str()?;
    if revision.is_empty() || revision.contains(['/', '?', '#', '@']) || revision == "." || revision == ".." {
        return None;
    }
    let location = symbol["evidence"].as_array().into_iter().flatten()
        .filter_map(|item| item["location"].as_str())
        .chain(symbol["provenance"].as_array().into_iter().flatten().filter_map(Value::as_str))
        .find_map(parse_source_location)?;
    let (path, line) = location;
    let mut uri = format!("{repository}/blob/{revision}/{path}");
    if let Some(line) = line { uri.push_str(&format!("#L{line}")); }
    let line = line.unwrap_or(1).saturating_sub(1);
    Some(json!({
        "uri":uri,
        "range":{
            "start":{"line":line,"character":0},
            "end":{"line":line,"character":0}
        }
    }))
}

fn definition(runtime: Option<&Runtime>, word: Option<String>) -> Value {
    let (Some(runtime), Some(word)) = (runtime, word) else { return Value::Null; };
    let options = SearchOptions { query: word.clone(), limit: MAX_SEARCH_RESULTS, ..SearchOptions::default() };
    let locations = runtime.search_with_options(&options).into_iter()
        .filter(|symbol| supported_symbol(symbol) && symbol["name"].as_str() == Some(word.as_str()))
        .filter_map(|symbol| symbol["id"].as_str().and_then(|id| runtime.get(id).ok()))
        .filter_map(|detail| source_location(&detail))
        .collect::<Vec<_>>();
    if locations.is_empty() { Value::Null } else { json!(locations) }
}

fn is_jenkinsfile_uri(uri: &str) -> bool {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "Jenkinsfile" || name.starts_with("Jenkinsfile.") || name.ends_with(".jenkinsfile")
}

fn has_verified_symbol_evidence(symbol: &Value) -> bool {
    symbol["evidence"].as_array().is_some_and(|items| {
        items.iter().any(|item| item["confidence"].as_str() == Some("verified"))
    })
}

fn field_is_verified(symbol: &Value, field: &str) -> bool {
    symbol["fieldEvidence"][field].as_str() == Some("verified")
}

fn verified_declared_contexts(symbol: &Value) -> Vec<String> {
    let mut contexts = Vec::new();
    let candidates = [
        (field_is_verified(symbol, "contexts"), symbol["contexts"].as_array()),
        (field_is_verified(symbol, "executionConstraints"), symbol["executionConstraints"]["contexts"].as_array()),
    ];
    for (verified, values) in candidates {
        if !verified { continue; }
        for value in values.into_iter().flatten().filter_map(Value::as_str) {
            if !contexts.iter().any(|context| context == value) { contexts.push(value.to_owned()); }
        }
    }
    contexts
}

fn diagnostic_for(
    detail: &Value,
    token: &IdentifierOccurrence,
    code: &str,
    severity: u8,
    message: String,
    profile: &str,
) -> Value {
    let source = &detail["source"];
    json!({
        "range":{
            "start":{"line":token.line,"character":token.character},
            "end":{"line":token.line,"character":token.end_character}
        },
        "severity":severity,
        "code":code,
        "source":"Jenkins Knowledge",
        "message":message,
        "data":{
            "symbolId":detail["id"],
            "sourceId":detail["sourceId"],
            "sourceName":source["name"],
            "sourceRevision":source["revision"],
            "sourceProvenance":source["manifestProvenance"],
            "provenance":detail["provenance"],
            "evidence":detail["evidence"],
            "confidence":detail["confidence"],
            "profile":profile
        }
    })
}

fn diagnostics_for_document(runtime: Option<&Runtime>, uri: &str, text: &str) -> Vec<Value> {
    let Some(runtime) = runtime else { return Vec::new(); };
    if !is_jenkinsfile_uri(uri) { return Vec::new(); }
    let Some(tokens) = scan_identifiers(text) else { return Vec::new(); };
    let mut by_name: HashMap<&str, Vec<&Value>> = HashMap::new();
    for symbol in runtime.symbols() {
        if let Some(name) = symbol["name"].as_str() { by_name.entry(name).or_default().push(symbol); }
    }
    let mut details = HashMap::<String, Value>::new();
    let mut diagnostics = Vec::new();
    for token in &tokens {
        let Some(candidates) = by_name.get(token.name.as_str()) else { continue; };
        if candidates.len() != 1 { continue; }
        for symbol in candidates {
            if symbol["confidence"].as_str() != Some("verified") || !has_verified_symbol_evidence(symbol) { continue; }
            let Some(id) = symbol["id"].as_str() else { continue; };
            let detail = details.entry(id.to_owned()).or_insert_with(|| runtime.get(id).unwrap_or_else(|_| (*symbol).clone()));
            let source_name = detail["source"]["name"].as_str()
                .or_else(|| detail["sourceId"].as_str()).unwrap_or("unknown source");
            let revision = detail["source"]["revision"].as_str().unwrap_or("unknown revision");
            let evidence = format!("Source: {source_name} · revision: {revision} · confidence: verified");

            if symbol["kind"] == "declarative-directive" && token.statement_start {
                let contexts = verified_declared_contexts(symbol);
                if !contexts.is_empty() {
                    let current = if token.declarative { token.context.unwrap_or("pipeline") } else { token.context.unwrap_or("scripted") };
                    if !contexts.iter().any(|context| context == current) {
                        diagnostics.push(diagnostic_for(
                            detail, token, "jenkins.declarative.invalid-context", 1,
                            format!("Declarative directive '{}' is not valid in the '{}' context. Known valid contexts: {}. {evidence}",
                                token.name, current, contexts.join(", ")),
                            runtime.profile(),
                        ));
                    }
                }
            }

            let plugin_dependency = detail["source"]["kind"] == "plugin"
                || detail["availability"]["pluginSourceIds"].as_array().is_some_and(|items| !items.is_empty())
                || detail["executionConstraints"]["pluginDependencies"].as_array().is_some_and(|items| !items.is_empty());
            if plugin_dependency && field_is_verified(symbol, "availability")
                && detail["availability"]["status"] == "unavailable" {
                let reason = detail["availability"]["reason"].as_str().unwrap_or("the catalog marks the required plugin as unavailable");
                diagnostics.push(diagnostic_for(
                    detail, token, "jenkins.plugin.unavailable", 2,
                    format!("'{}' depends on an unavailable Jenkins plugin: {reason}. {evidence}", token.name),
                    runtime.profile(),
                ));
            }
            if field_is_verified(symbol, "executionConstraints")
                && detail["executionConstraints"]["cps"] == "incompatible" {
                diagnostics.push(diagnostic_for(
                    detail, token, "jenkins.pipeline.cps-incompatible", 1,
                    format!("'{}' is known to be incompatible with Pipeline CPS. {evidence}", token.name),
                    runtime.profile(),
                ));
            }
            if field_is_verified(symbol, "executionConstraints")
                && detail["executionConstraints"]["sandbox"] == "rejected" {
                diagnostics.push(diagnostic_for(
                    detail, token, "jenkins.pipeline.sandbox-rejected", 1,
                    format!("'{}' is known to be rejected by the Script Security sandbox. {evidence}", token.name),
                    runtime.profile(),
                ));
            }
            if diagnostics.len() >= 200 { return diagnostics; }
        }
    }
    diagnostics
}

fn publish_diagnostics(connection: &Connection, uri: &str, runtime: Option<&Runtime>, text: &str) {
    let notification = Notification::new(
        "textDocument/publishDiagnostics".into(),
        json!({"uri":uri,"diagnostics":diagnostics_for_document(runtime, uri, text)}),
    );
    let _ = connection.sender.send(Message::Notification(notification));
}

fn percent_decode_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = ((bytes[index + 1] as char).to_digit(16), (bytes[index + 2] as char).to_digit(16)) {
                decoded.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn workspace_root(initialization: &Value) -> Option<PathBuf> {
    initialization.pointer("/initializationOptions/workspaceRoot").and_then(Value::as_str)
        .or_else(|| initialization.get("rootPath").and_then(Value::as_str))
        .map(PathBuf::from)
        .or_else(|| initialization.get("rootUri").and_then(Value::as_str)
            .and_then(|uri| uri.strip_prefix("file://"))
            .map(percent_decode_path).map(PathBuf::from))
        .or_else(|| initialization.pointer("/workspaceFolders/0/uri").and_then(Value::as_str)
            .and_then(|uri| uri.strip_prefix("file://"))
            .map(percent_decode_path).map(PathBuf::from))
}

fn select_runtime(settings: &Value, runtime: &mut Option<Runtime>, root: Option<&Path>) -> Result<String, String> {
    match open_runtime(settings, root) {
        Ok(next) => {
            let profile = next.profile().to_owned();
            *runtime = Some(next);
            Ok(profile)
        }
        Err(error) => {
            *runtime = None;
            Err(error.to_string())
        }
    }
}

fn run(connection: Connection, initialization: Value) -> Result<(), Box<dyn Error>> {
    let root = workspace_root(&initialization);
    let mut runtime = match open_runtime(initialization.get("initializationOptions").unwrap_or(&Value::Null), root.as_deref()) {
        Ok(runtime) => Some(runtime),
        Err(error) => { log(&connection, error.to_string(), 1); None }
    };
    let mut documents: HashMap<String, String> = HashMap::new();

    while let Ok(message) = connection.receiver.recv() {
        match message {
            Message::Request(request) if request.method == "jenkinsKnowledge/reload" => {
                match select_runtime(settings(&request.params), &mut runtime, root.as_deref()) {
                    Ok(profile) => {
                        log(&connection, format!("Loaded Knowledge Package profile '{profile}'"), 3);
                        for (uri, text) in &documents { publish_diagnostics(&connection, uri, runtime.as_ref(), text); }
                        connection.sender.send(Message::Response(Response::new_ok(
                            request.id, json!({"status":"loaded","profile":profile})
                        )))?;
                    }
                    Err(error) => {
                        log(&connection, error.clone(), 1);
                        for (uri, text) in &documents { publish_diagnostics(&connection, uri, None, text); }
                        connection.sender.send(Message::Response(Response::new_err(
                            request.id, lsp_server::ErrorCode::InvalidParams as i32, error
                        )))?;
                    }
                }
            }
            Message::Request(request) => handle_request(&connection, request, runtime.as_ref(), &documents)?,
            Message::Notification(notification) => match notification.method.as_str() {
                "exit" => break,
                "textDocument/didOpen" => {
                    if let (Some(uri), Some(text)) = (
                        notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                        notification.params.pointer("/textDocument/text").and_then(Value::as_str),
                    ) {
                        documents.insert(uri.into(), text.into());
                        publish_diagnostics(&connection, uri, runtime.as_ref(), text);
                    }
                }
                "textDocument/didChange" => {
                    if let (Some(uri), Some(text)) = (
                        notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                        notification.params.pointer("/contentChanges/0/text").and_then(Value::as_str),
                    ) {
                        documents.insert(uri.into(), text.into());
                        publish_diagnostics(&connection, uri, runtime.as_ref(), text);
                    }
                }
                "textDocument/didClose" => {
                    if let Some(uri) = notification.params.pointer("/textDocument/uri").and_then(Value::as_str) {
                        documents.remove(uri);
                        publish_diagnostics(&connection, uri, None, "");
                    }
                }
                "workspace/didChangeConfiguration" => {
                    match select_runtime(settings(&notification.params), &mut runtime, root.as_deref()) {
                        Ok(profile) => log(&connection, format!("Loaded Knowledge Package profile '{profile}'"), 3),
                        Err(error) => log(&connection, error, 1),
                    }
                    for (uri, text) in &documents { publish_diagnostics(&connection, uri, runtime.as_ref(), text); }
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn search_options(params: &Value, limit: usize) -> SearchOptions {
    SearchOptions {
        query: params.get("query").and_then(Value::as_str).unwrap_or("").to_owned(),
        kind: params.get("kind").and_then(Value::as_str).map(str::to_owned),
        source: params.get("source").and_then(Value::as_str).map(str::to_owned),
        controller: params.get("controller").and_then(Value::as_str).map(str::to_owned),
        plugin: params.get("plugin").and_then(Value::as_str).map(str::to_owned),
        owner: params.get("owner").and_then(Value::as_str).map(str::to_owned),
        type_name: params.get("type").and_then(Value::as_str).map(str::to_owned),
        confidence: params.get("confidence").and_then(Value::as_str).map(str::to_owned),
        prefix: None,
        contexts: params.get("context").and_then(Value::as_str).map(|value| vec![value.to_owned()]),
        kinds: None,
        source_kind: params.get("sourceKind").and_then(Value::as_str).map(str::to_owned),
        limit,
    }
}

fn dispatch_knowledge_request(runtime: &Runtime, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "jenkinsKnowledge/list" => {
            let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
                return Err(format!("limit must be between 1 and {MAX_SEARCH_RESULTS}"));
            }
            let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            if offset > MAX_SEARCH_OFFSET {
                return Err(format!("offset must be between 0 and {MAX_SEARCH_OFFSET}"));
            }
            let options = search_options(params, limit);
            let page = runtime.search_page(&options, offset).map_err(|error| error.to_string())?;
            let returned = page.results.len();
            let next_offset = if page.has_more { Some(page.offset + returned) } else { None };
            Ok(json!({
                "profile":runtime.profile(),
                "offset":page.offset,
                "limit":page.limit,
                "total":page.total,
                "returned":returned,
                "hasMore":page.has_more,
                "nextOffset":next_offset,
                "results":page.results
            }))
        }
        "jenkinsKnowledge/get" => {
            let name = params.get("nameOrId").or_else(|| params.get("id"))
                .and_then(Value::as_str).filter(|value| !value.is_empty())
                .ok_or_else(|| "nameOrId or id is required".to_string())?;
            runtime.get(name).map_err(|error| error.to_string())
        }
        "jenkinsKnowledge/controller" => Ok(json!({
            "profile":runtime.profile(),
            "controller":runtime.controller_details()
        })),
        "jenkinsKnowledge/plugins" => Ok(json!({
            "profile":runtime.profile(),
            "status":runtime.plugin_inventory_status(),
            "plugins":runtime.plugins()
        })),
        "jenkinsKnowledge/sources" => Ok(json!({
            "profile":runtime.profile(),
            "sources":runtime.source_details()
        })),
        _ => Err(format!("unsupported Knowledge request: {method}")),
    }
}

fn handle_request(connection: &Connection, request: Request, runtime: Option<&Runtime>,
    documents: &HashMap<String, String>) -> Result<(), Box<dyn Error>> {
    let result = match request.method.as_str() {
        "shutdown" => Value::Null,
        "textDocument/completion" => {
            let uri = request.params.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or_default();
            let line = request.params.pointer("/position/line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let character = request.params.pointer("/position/character").and_then(Value::as_u64).unwrap_or(0) as usize;
            let text = documents.get(uri).map(String::as_str).unwrap_or("");
            completion_for_document(runtime, uri, text, line, character)
        },
        "jenkinsKnowledge/coverage" => json!({
            "profile": runtime.map(Runtime::profile),
            "coverage": runtime.map(Runtime::coverage).unwrap_or(&[])
        }),
        "jenkinsKnowledge/collection" => collection_outcome(runtime),
        "jenkinsKnowledge/search" => {
            let Some(runtime) = runtime else {
                connection.sender.send(Message::Response(Response::new_err(
                    request.id, lsp_server::ErrorCode::InvalidRequest as i32,
                    "No Knowledge Package is loaded".to_string()
                )))?;
                return Ok(());
            };
            let limit = request.params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
                connection.sender.send(Message::Response(Response::new_err(
                    request.id, lsp_server::ErrorCode::InvalidParams as i32,
                    format!("limit must be between 1 and {MAX_SEARCH_RESULTS}")
                )))?;
                return Ok(());
            }
            let options = search_options(&request.params, limit);
            json!({"profile": runtime.profile(), "limit": limit, "results": runtime.search_with_options(&options)})
        },
        "jenkinsKnowledge/list" | "jenkinsKnowledge/get" | "jenkinsKnowledge/controller" | "jenkinsKnowledge/plugins" | "jenkinsKnowledge/sources" => {
            let Some(runtime) = runtime else {
                connection.sender.send(Message::Response(Response::new_err(
                    request.id, lsp_server::ErrorCode::InvalidRequest as i32,
                    "No Knowledge Package is loaded".to_string()
                )))?;
                return Ok(());
            };
            match dispatch_knowledge_request(runtime, &request.method, &request.params) {
                Ok(result) => result,
                Err(error) => {
                    connection.sender.send(Message::Response(Response::new_err(
                        request.id, lsp_server::ErrorCode::InvalidParams as i32, error
                    )))?;
                    return Ok(());
                }
            }
        },
        "textDocument/hover" => {
            let uri = request.params.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or_default();
            let line = request.params.pointer("/position/line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let character = request.params.pointer("/position/character").and_then(Value::as_u64).unwrap_or(0) as usize;
            let word = if is_jenkinsfile_uri(uri) {
                documents.get(uri).and_then(|text| word_at(text, line, character))
            } else { None };
            hover(runtime, word)
        }
        "textDocument/definition" => {
            let uri = request.params.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or_default();
            let line = request.params.pointer("/position/line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let character = request.params.pointer("/position/character").and_then(Value::as_u64).unwrap_or(0) as usize;
            let word = if is_jenkinsfile_uri(uri) {
                documents.get(uri).and_then(|text| word_at(text, line, character))
            } else { None };
            definition(runtime, word)
        }
        _ => {
            let response = Response::new_err(request.id, lsp_server::ErrorCode::MethodNotFound as i32,
                format!("unsupported method: {}", request.method));
            connection.sender.send(Message::Response(response))?;
            return Ok(());
        }
    };
    connection.sender.send(Message::Response(Response::new_ok(request.id, result)))?;
    Ok(())
}

fn main() {
    if let Err(error) = serve() {
        eprintln!("jenkins-lsp: {error}");
        process::exit(1);
    }
}

pub fn serve() -> Result<(), Box<dyn Error>> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, initialization) = connection.initialize_start()?;
    connection.initialize_finish(initialize_id, json!({
        "capabilities": {
            "textDocumentSync": {"openClose": true, "change": 1},
            "completionProvider": {"resolveProvider": false, "triggerCharacters": ["."]},
            "hoverProvider": true,
            "definitionProvider": true
        },
        "serverInfo": {"name": "jenkins-knowledge", "version": env!("CARGO_PKG_VERSION")}
    }))?;
    run(connection, initialization)?;
    io_threads.join()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::fs;

    static NEXT_PACKAGE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn fixture_runtime(profile: &str) -> Runtime {
        fixture_runtime_with_symbols(profile, Vec::new())
    }

    fn fixture_package(profile: &str, extra_symbols: Vec<Value>) -> std::path::PathBuf {
        let package = std::env::temp_dir().join(format!(
            "jenkins-lsp-test-{}-{}",
            process::id(),
            NEXT_PACKAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&package);
        fs::create_dir_all(&package).unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        if let Some(source) = catalog["sources"].as_array_mut().unwrap().iter_mut()
            .find(|source| source["id"] == "platform-lib")
        {
            source["url"] = json!("https://github.com/example/jenkins-shared-library");
        }
        catalog["symbols"].as_array_mut().unwrap().extend(extra_symbols);
        let catalog_bytes = serde_json::to_vec(&catalog).unwrap();
        fs::write(package.join("catalog.json"), &catalog_bytes).unwrap();
        let checksum = sha2::Sha256::digest(&catalog_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut source_revisions = serde_json::Map::new();
        let mut provenance = serde_json::Map::new();
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let Some(id) = source["id"].as_str() else { continue; };
            source_revisions.insert(id.into(), source["revision"].clone());
            provenance.insert(id.into(), json!({
                "name": source["name"],
                "kind": source["kind"],
                "revision": source["revision"],
                "url": source.get("url").cloned().unwrap_or(Value::Null)
            }));
        }
        fs::write(package.join("manifest.json"), json!({
            "packageVersion": "1.0.0",
            "catalogSchemaVersion": "1.0.0",
            "profile": profile,
            "createdAt": "2026-01-01T00:00:00Z",
            "catalog": "catalog.json",
            "sourceRevisions": source_revisions,
            "provenance": provenance,
            "checksums": {"catalog.json": checksum},
            "toolVersions": {},
            "collection": {
                "status": "partial",
                "profile": "standard",
                "startedAt": "2026-01-01T00:00:00Z",
                "completedAt": "2026-01-01T00:00:01Z",
                "sources": [{
                    "id": "controller:ci:pipeline-gdsl",
                    "kind": "pipeline-gdsl",
                    "name": "ci",
                    "surface": "pipeline-gdsl",
                    "status": "unavailable",
                    "errorCode": "http-unavailable"
                }]
            }
        }).to_string()).unwrap();
        package
    }

    fn fixture_runtime_with_symbols(profile: &str, extra_symbols: Vec<Value>) -> Runtime {
        let package = fixture_package(profile, extra_symbols);
        let runtime = Runtime::open(&package).unwrap();
        fs::remove_dir_all(package).unwrap();
        runtime
    }

    #[test]
    fn exposes_collection_completeness_without_changing_symbol_confidence() {
        let runtime = fixture_runtime("fixture");
        let collection = collection_outcome(Some(&runtime));
        assert_eq!(collection["status"], "partial");
        assert_eq!(collection["sources"][0]["status"], "unavailable");
        assert_eq!(runtime.get("sh").unwrap()["confidence"], "verified");
        assert_eq!(collection_outcome(None)["status"], "unknown");
    }

    #[test]
    fn finds_identifier_at_cursor_using_utf16_columns() {
        assert_eq!(word_at("écho deployApp()", 0, 10).as_deref(), Some("deployApp"));
    }

    #[test]
    fn ignores_punctuation_and_empty_lines() {
        assert_eq!(word_at("sh 'hello world'", 0, 2).as_deref(), Some("sh"));
        assert_eq!(word_at("sh 'hello world'", 0, 3), None);
        assert_eq!(word_at("first\nsecond", 1, 3).as_deref(), Some("second"));
    }

    fn context_at_end(text: &str) -> (String, Vec<String>, Vec<String>) {
        let line = text.lines().count().saturating_sub(1);
        let character = text.lines().last().unwrap_or_default().encode_utf16().count();
        completion_context(text, line, character)
    }

    #[test]
    fn nested_closures_do_not_close_their_declarative_parent_context() {
        let source = "pipeline {\n stages {\n stage('build') {\n steps {\n script {\n sh 'inside'\n }\n sh";
        let (_, contexts, kinds) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "pipeline"));
        assert!(contexts.iter().any(|context| context == "stages"));
        assert!(contexts.iter().any(|context| context == "stage"));
        assert!(contexts.iter().any(|context| context == "steps"));
        assert!(!contexts.iter().any(|context| context == "script"));
        assert_eq!(kinds, vec!["step", "global-variable"]);
    }

    #[test]
    fn ignores_block_like_text_in_comments_and_quoted_strings() {
        let source = r#"pipeline {
    // script { steps {
    /* post { stage('fake') { } } */
    stages {
        stage('real') {
            steps {
                def single = 'script { }'
                def double = "post { }"
                def tripleSingle = '''agent { }'''
                def tripleDouble = """environment { }"""
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post", "agent", "environment"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn escaped_quotes_do_not_end_strings_early() {
        let source = r#"pipeline {
    def double = "steps { \" stage { }"
    def single = 'script { \' post { }'
    steps {
        sh
"#;
        let (_, contexts, _) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "steps"));
        assert!(!contexts.iter().any(|context| context == "stage"));
        assert!(!contexts.iter().any(|context| context == "script"));
        assert!(!contexts.iter().any(|context| context == "post"));
    }

    #[test]
    fn incomplete_blocks_and_unbalanced_closers_are_handled_safely() {
        let source = "pipeline {\n stages {\n stage('unfinished') {\n steps {\n sh";
        let (_, contexts, kinds) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "steps"));
        assert_eq!(kinds, vec!["step", "global-variable"]);

        let (_, contexts, _) = context_at_end("}}}} pipeline {");
        assert!(contexts.iter().any(|context| context == "pipeline"));
    }

    #[test]
    fn nested_declarative_contexts_unwind_one_block_at_a_time() {
        let pipeline = "pipeline {\n stages {\n stage('build') {\n steps {\n sh";
        assert_eq!(
            scan_block_contexts(pipeline).unwrap(),
            vec!["pipeline", "stages", "stage", "steps"]
        );
        assert_eq!(
            scan_block_contexts(&format!("{pipeline}\n }}")).unwrap(),
            vec!["pipeline", "stages", "stage"]
        );
        assert_eq!(
            scan_block_contexts(&format!("{pipeline}\n }}\n }}")).unwrap(),
            vec!["pipeline", "stages"]
        );
        assert_eq!(
            scan_block_contexts(&format!("{pipeline}\n }}\n }}\n }}")).unwrap(),
            vec!["pipeline"]
        );
        assert!(scan_block_contexts(&format!("{pipeline}\n }}\n }}\n }}\n }}"))
            .unwrap().is_empty());
    }

    #[test]
    fn context_scan_is_bounded_for_very_large_documents() {
        let source = format!("{}pipeline {{", " ".repeat(MAX_CONTEXT_SCAN_BYTES));
        assert!(scan_block_contexts(&source).is_none());
    }

    #[test]
    fn ignores_directive_text_and_braces_in_slashy_and_dollar_slashy_strings() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def slashy = /script { } stage('fake') { } \/ and \/\/ text/
                def dollar = $/post { } pipeline { } escaped $/ slash and $$ sign /$
                def interpolated = "value @@INTERP@@ [script: { steps { } }] }"
                sh
"#.replace("@@INTERP@@", "\u{24}\u{7b}");
        let (_, contexts, _) = context_at_end(&source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn map_list_literals_and_method_calls_do_not_open_declarative_contexts() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def map = [stage: { script { } }, nested: [post: { }]]
                def list = [{ agent { } }, [steps: { }]]
                def value = helper(stage: [script: { }])
                helper.stage()
                def result = stage('not a block')
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post", "agent"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn ordinary_closures_and_script_bodies_do_not_create_declarative_contexts() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def closure = {
                    stage { script { } }
                }
                script {
                    stage { steps { } }
                }
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        assert!(!contexts.iter().any(|context| context == "script"), "{contexts:?}");
    }

    #[test]
    fn nested_completion_keeps_step_symbols_and_excludes_declarative_directives() {
        let runtime = fixture_runtime("fixture");
        let source = "pipeline {\n stages {\n stage('build') {\n steps {\n script {\n sh 'inside'\n }\n sh";
        let result = completion(Some(&runtime), source, 7, 2);
        let labels: Vec<_> = result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"sh"));
        assert!(!labels.contains(&"agent"));
    }

    #[test]
    fn supported_symbols_include_steps_globals_and_selected_types() {
        assert!(supported_symbol(&json!({"kind":"step"})));
        assert!(supported_symbol(&json!({"kind":"global-variable","source":{"kind":"jenkins-controller"}})));
        assert!(supported_symbol(&json!({"kind":"class","source":{"kind":"plugin"}})));
        assert!(supported_symbol(&json!({"kind":"class","source":{"kind":"jenkins-controller"}})));
        assert!(supported_symbol(&json!({"kind":"method","source":{"kind":"shared-library"}})));
        assert!(!supported_symbol(&json!({"kind":"directive","source":{"kind":"jenkins"}})));
    }

    #[test]
    fn completion_is_restricted_to_jenkinsfile_documents() {
        let runtime = fixture_runtime("fixture");
        let jenkinsfile = completion_for_document(
            Some(&runtime),
            "file:///workspace/Jenkinsfile",
            "node {\n  de",
            1,
            4,
        );
        assert!(jenkinsfile["items"].as_array().unwrap().iter()
            .any(|item| item["label"] == "deployApp"));

        let groovy = completion_for_document(
            Some(&runtime),
            "file:///workspace/ordinary.groovy",
            "node {\n  de",
            1,
            4,
        );
        assert_eq!(groovy, json!({"isIncomplete": false, "items": []}));
    }

    #[test]
    fn completion_and_hover_use_the_selected_catalog() {
        let runtime = fixture_runtime("fixture");
        let items = completion(Some(&runtime), "", 0, 0);
        let labels: Vec<_> = items["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"sh"));
        assert!(labels.contains(&"deployApp"));
        assert!(!labels.contains(&"agent"));

        let details = hover(Some(&runtime), Some("sh".into()));
        let value = details["contents"]["value"].as_str().unwrap();
        assert!(value.contains("sh(Map args)"));
        assert!(value.contains("Runs a shell command."));
        assert!(value.contains("Jenkins fixture"));
        assert!(value.contains("verified"));
        assert!(value.contains("fixture"));
    }
    #[test]
    fn completion_uses_prefix_and_declarative_or_scripted_context() {
        let runtime = fixture_runtime("fixture");
        let declarative = completion(Some(&runtime), "pipeline {\n  agent ", 1, 8);
        let declarative_labels: Vec<_> = declarative["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(declarative_labels.contains(&"agent"));

        let scripted = completion(Some(&runtime), "node {\n  sh", 1, 4);
        let scripted_labels: Vec<_> = scripted["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(scripted_labels.contains(&"sh"));
        assert!(!scripted_labels.contains(&"agent"));
        assert!(scripted_labels.iter().all(|label| label.starts_with("sh")));
    }

    #[test]
    fn completion_finds_prefix_matches_beyond_the_old_global_first_hundred() {
        let mut extra = Vec::new();
        for index in 0..120 {
            extra.push(json!({
                "id": format!("library:aaa-{index:03}"),
                "name": format!("aaa-{index:03}"),
                "kind": "global-variable",
                "sourceId": "platform-lib",
                "contexts": ["pipeline"],
                "confidence": "verified",
                "provenance": ["test"]
            }));
        }
        extra.push(json!({
            "id": "library:zzTarget",
            "name": "zzTarget",
            "kind": "global-variable",
            "sourceId": "platform-lib",
            "contexts": ["pipeline"],
            "confidence": "verified",
            "provenance": ["test"]
        }));
        let runtime = fixture_runtime_with_symbols("fixture", extra);
        let result = completion(Some(&runtime), "node {\n  zz", 1, 4);
        let labels: Vec<_> = result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"zzTarget"));
        assert_eq!(labels.len(), 1);
        assert_eq!(result["isIncomplete"], false);
    }



    #[test]
    fn lsp_search_options_preserve_controller_context_and_source_filters() {
        let runtime = fixture_runtime("fixture");
        let options = search_options(&json!({
            "controller":"jenkins-ci",
            "sourceKind":"jenkins-controller",
            "context":"steps"
        }), 50);
        let page = runtime.search_page(&options, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.results[0]["name"], "sh");
    }


    #[test]
    fn lsp_read_only_detail_and_list_requests_share_runtime_metadata() {
        let runtime = fixture_runtime("fixture");
        let detail = dispatch_knowledge_request(&runtime, "jenkinsKnowledge/get", &json!({"nameOrId":"sh"})).unwrap();
        assert_eq!(detail["confidence"], "verified");
        assert_eq!(detail["executionConstraintsSummary"]["pluginAvailability"]["status"], "available");

        let list = dispatch_knowledge_request(&runtime, "jenkinsKnowledge/list", &json!({
            "sourceKind":"shared-library", "limit":1, "offset":0
        })).unwrap();
        assert_eq!(list["total"], 1);
        assert_eq!(list["results"][0]["name"], "deployApp");
        assert_eq!(list["results"][0]["source"]["manifestProvenance"]["kind"], "shared-library");

        let controller = dispatch_knowledge_request(&runtime, "jenkinsKnowledge/controller", &json!({})).unwrap();
        let plugins = dispatch_knowledge_request(&runtime, "jenkinsKnowledge/plugins", &json!({})).unwrap();
        let sources = dispatch_knowledge_request(&runtime, "jenkinsKnowledge/sources", &json!({})).unwrap();
        assert_eq!(controller["controller"]["identity"]["id"], "jenkins-ci");
        assert_eq!(plugins["status"], "partial");
        assert!(sources["sources"].as_array().unwrap().iter().any(|source| source["kind"] == "shared-library"));
    }


    #[test]
    fn completion_includes_controller_gdsl_steps_without_context_metadata() {
        let runtime = fixture_runtime_with_symbols("completion-profile", vec![json!({
            "id":"controller:controllerGdslStep:step",
            "name":"controllerGdslStep",
            "kind":"step",
            "sourceId":"controller",
            "signature":"controllerGdslStep()",
            "description":"A step observed from the Jenkins Pipeline GDSL endpoint.",
            "confidence":"verified",
            "provenance":["GET /pipeline-syntax/gdsl"]
        })]);

        let result = completion(Some(&runtime), r#"node {
  controllerGdslStep"#, 1, 20);
        assert!(result["items"].as_array().unwrap().iter()
            .any(|item| item["label"] == "controllerGdslStep"));
    }

    #[test]
    fn completion_includes_controller_globals_and_selected_plugin_types() {
        let runtime = fixture_runtime_with_symbols("completion-profile", vec![
            json!({
                "id":"plugin:jenkinsGlobal:global-variable",
                "name":"jenkinsGlobal",
                "kind":"global-variable",
                "namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps",
                "signature":"jenkinsGlobal",
                "description":"A plugin-provided Pipeline global.",
                "confidence":"verified",
                "contexts":["pipeline","scripted"],
                "provenance":["plugin:workflow-basic-steps"]
            }),
            json!({
                "id":"plugin:PluginType:class",
                "name":"PluginType",
                "kind":"class",
                "namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps",
                "signature":"org.jenkinsci.plugins.workflow.PluginType",
                "description":"A selected Jenkins plugin API type.",
                "confidence":"verified",
                "contexts":["pipeline","scripted"],
                "provenance":["plugin:workflow-basic-steps"],
                "typeStatus":"available"
            }),
            json!({
                "id":"controller:ControllerType:class",
                "name":"ControllerType",
                "kind":"class",
                "namespace":"hudson.model",
                "sourceId":"controller",
                "signature":"hudson.model.ControllerType",
                "description":"A type observed on the selected Jenkins controller.",
                "confidence":"verified",
                "contexts":["pipeline","scripted"],
                "provenance":["controller:runtime-snapshot"],
                "typeStatus":"available"
            }),
            json!({
                "id":"plugin:PluginType:run",
                "name":"run",
                "kind":"method",
                "namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps",
                "signature":"run()",
                "declaringClass":"org.jenkinsci.plugins.workflow.PluginType",
                "description":"Runs the plugin type operation.",
                "confidence":"verified",
                "contexts":["pipeline","scripted"],
                "provenance":["plugin:workflow-basic-steps"]
            })
        ]);
        let global_result = completion(Some(&runtime), "node {\n  jenkinsG", 1, 10);
        let global_labels = global_result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect::<Vec<_>>();
        assert!(global_labels.contains(&"jenkinsGlobal"));
        let type_result = completion(Some(&runtime), "node {\n  PluginT", 1, 9);
        let type_labels = type_result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect::<Vec<_>>();
        assert!(type_labels.contains(&"PluginType"));
        let controller_result = completion(Some(&runtime), "node {\n  ControllerT", 1, 13);
        assert!(controller_result["items"].as_array().unwrap().iter().any(|item| item["label"] == "ControllerType"));
        let type_hover = hover(Some(&runtime), Some("PluginType".into()));
        assert!(type_hover["contents"]["value"].as_str().unwrap().contains("Members (available)"));
        assert!(type_hover["contents"]["value"].as_str().unwrap().contains("run()"));
    }

    #[test]
    fn scripted_and_declarative_jenkinsfile_fixtures_keep_context_scopes_distinct() {
        let runtime = fixture_runtime("fixture");
        let scripted = include_str!("../../../tests/fixtures/jenkinsfiles/scripted.Jenkinsfile");
        let step_line = scripted.lines().position(|line| line.contains("sh 'make'")).unwrap();
        let step_column = scripted.lines().nth(step_line).unwrap().find("sh").unwrap() + 2;
        let step_items = completion(Some(&runtime), scripted, step_line, step_column);
        assert!(step_items["items"].as_array().unwrap().iter().any(|item| item["label"] == "sh"));
        let library_line = scripted.lines().position(|line| line.contains("deployApp()")).unwrap();
        let library_column = scripted.lines().nth(library_line).unwrap().find("deployApp").unwrap() + "deployApp".len();
        let library_items = completion(Some(&runtime), scripted, library_line, library_column);
        assert!(library_items["items"].as_array().unwrap().iter().any(|item| item["label"] == "deployApp"));

        let declarative = include_str!("../../../tests/fixtures/jenkinsfiles/declarative.Jenkinsfile");
        let agent_line = declarative.lines().position(|line| line.trim_start().starts_with("agent")).unwrap();
        let agent_column = declarative.lines().nth(agent_line).unwrap().find("agent").unwrap() + "agent".len();
        let agent_items = completion(Some(&runtime), declarative, agent_line, agent_column);
        let agent_labels = agent_items["items"].as_array().unwrap().iter().filter_map(|item| item["label"].as_str()).collect::<Vec<_>>();
        assert!(agent_labels.contains(&"agent"));
        assert!(!agent_labels.contains(&"sh"));
        let steps_line = declarative.lines().position(|line| line.contains("sh 'make'")).unwrap();
        let steps_column = declarative.lines().nth(steps_line).unwrap().find("sh").unwrap() + 2;
        let steps_items = completion(Some(&runtime), declarative, steps_line, steps_column);
        let labels = steps_items["items"].as_array().unwrap().iter().filter_map(|item| item["label"].as_str()).collect::<Vec<_>>();
        assert!(labels.contains(&"sh"));
        assert!(!labels.contains(&"agent"));
    }

    #[test]
    fn hover_reports_overloads_parameters_plugin_source_constraints_and_provenance() {
        let runtime = fixture_runtime_with_symbols("hover-profile", vec![
            json!({
                "id":"plugin:helper:string",
                "name":"helper",
                "kind":"method",
                "namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps",
                "signature":"helper(String text)",
                "description":"Formats a text value.",
                "parameters":[{"name":"text","type":"java.lang.String","description":"Text to format."}],
                "returnType":"java.lang.String",
                "confidence":"verified",
                "contexts":["pipeline","scripted"],
                "provenance":["plugin:workflow-basic-steps"],
                "availability":{"status":"available","pluginSourceIds":["workflow-basic-steps"]},
                "executionConstraints":{"cps":"compatible","sandbox":"approved","contexts":["pipeline","scripted"],"pluginDependencies":["workflow-basic-steps"]}
            }),
            json!({
                "id":"plugin:helper:map",
                "name":"helper",
                "kind":"method",
                "namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps",
                "signature":"helper(Map args)",
                "description":"Formats a map value.",
                "parameters":[{"name":"args","type":"java.util.Map","description":"Arguments to format."}],
                "returnType":"java.lang.String",
                "confidence":"verified",
                "contexts":["pipeline","scripted"],
                "provenance":["plugin:workflow-basic-steps"],
                "availability":{"status":"available","pluginSourceIds":["workflow-basic-steps"]},
                "executionConstraints":{"cps":"compatible","sandbox":"approved","contexts":["pipeline","scripted"],"pluginDependencies":["workflow-basic-steps"]}
            })
        ]);
        let result = hover(Some(&runtime), Some("helper".into()));
        let markdown = result["contents"]["value"].as_str().unwrap();
        assert!(markdown.contains("helper(String text)"));
        assert!(markdown.contains("helper(Map args)"));
        assert!(markdown.contains("Text to format."));
        assert!(markdown.contains("workflow-basic-steps"));
        assert!(markdown.contains("2.27"));
        assert!(markdown.contains("CPS:** compatible"));
        assert!(markdown.contains("Sandbox:** approved"));
        assert!(markdown.contains("Confidence:** verified"));
        assert!(markdown.contains("Profile:** hover-profile"));
    }

    #[test]
    fn catalog_language_features_are_limited_to_jenkinsfile_documents() {
        assert!(is_jenkinsfile_uri("file:///workspace/Jenkinsfile"));
        assert!(is_jenkinsfile_uri("file:///workspace/Jenkinsfile.groovy"));
        assert!(is_jenkinsfile_uri("file:///workspace/ci.jenkinsfile"));
        assert!(!is_jenkinsfile_uri("file:///workspace/vars/build.groovy"));
        assert!(!is_jenkinsfile_uri("file:///workspace/src/Library.groovy"));
    }

    #[test]
    fn diagnostics_require_verified_explicit_evidence_and_skip_comments_strings_and_unknowns() {
        let runtime = fixture_runtime_with_symbols("diagnostic-profile", vec![
            json!({
                "id":"plugin:invalidDirective:declarative-directive",
                "name":"invalidDirective","kind":"declarative-directive","namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps","signature":"invalidDirective","description":"A directive valid only at pipeline level.",
                "confidence":"verified","contexts":["pipeline"],"fieldEvidence":{"contexts":"verified"},"provenance":["plugin:workflow-basic-steps"],
                "evidence":[{"sourceId":"workflow-basic-steps","kind":"controller-observed","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"2.27"}]
            }),
            json!({
                "id":"plugin:unavailableStep:step",
                "name":"unavailableStep","kind":"step","namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps","signature":"unavailableStep()","description":"Unavailable plugin step.",
                "confidence":"verified","contexts":["pipeline","steps"],"fieldEvidence":{"availability":"verified"},"provenance":["plugin:workflow-basic-steps"],
                "evidence":[{"sourceId":"workflow-basic-steps","kind":"controller-observed","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"2.27","location":"workflow-step:unavailableStep"}],
                "availability":{"status":"unavailable","reason":"Plugin is not installed.","pluginSourceIds":["workflow-basic-steps"]}
            }),
            json!({
                "id":"plugin:cpsBad:step",
                "name":"cpsBad","kind":"step","namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps","signature":"cpsBad()","description":"CPS-incompatible step.",
                "confidence":"verified","contexts":["pipeline","steps"],"fieldEvidence":{"executionConstraints":"verified"},"provenance":["plugin:workflow-basic-steps"],
                "evidence":[{"sourceId":"workflow-basic-steps","kind":"controller-observed","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"2.27"}],
                "executionConstraints":{"cps":"incompatible","sandbox":"unknown","contexts":["pipeline","steps"]}
            }),
            json!({
                "id":"plugin:sandboxBad:step",
                "name":"sandboxBad","kind":"step","namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps","signature":"sandboxBad()","description":"Sandbox-rejected step.",
                "confidence":"verified","contexts":["pipeline","steps"],"fieldEvidence":{"executionConstraints":"verified"},"provenance":["plugin:workflow-basic-steps"],
                "evidence":[{"sourceId":"workflow-basic-steps","kind":"controller-observed","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"2.27"}],
                "executionConstraints":{"cps":"unknown","sandbox":"rejected","contexts":["pipeline","steps"]}
            }),
            json!({
                "id":"plugin:unverifiedAvailability:step",
                "name":"unverifiedAvailability","kind":"step","namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps","signature":"unverifiedAvailability()","description":"Availability has no field-level evidence.",
                "confidence":"verified","contexts":["pipeline","steps"],"provenance":["plugin:workflow-basic-steps"],
                "availability":{"status":"unavailable","reason":"not installed","pluginSourceIds":["workflow-basic-steps"]}
            }),
            json!({
                "id":"plugin:unknownBad:step",
                "name":"unknownBad","kind":"step","namespace":"org.jenkinsci.plugins.workflow",
                "sourceId":"workflow-basic-steps","signature":"unknownBad()","description":"Step with unknown constraints.",
                "confidence":"verified","contexts":["pipeline","steps"],"fieldEvidence":{"availability":"unknown","executionConstraints":"unknown"},"provenance":["plugin:workflow-basic-steps"],
                "availability":{"status":"unknown","pluginSourceIds":["workflow-basic-steps"]},
                "executionConstraints":{"cps":"unknown","sandbox":"not-inspected","contexts":["pipeline","steps"]}
            })
        ]);
        let text = "pipeline {\n  stages {\n    stage('build') {\n      steps {\n        invalidDirective\n        unavailableStep()\n        cpsBad()\n        sandboxBad()\n        unknownBad()\n        unverifiedAvailability()\n        // unavailableStep cpsBad\n        def message = \\\"sandboxBad\\\"\n      }\n    }\n  }\n}";
        let diagnostics = diagnostics_for_document(Some(&runtime), "file:///workspace/Jenkinsfile", text);
        assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
        let codes = diagnostics.iter().filter_map(|item| item["code"].as_str()).collect::<Vec<_>>();
        assert!(codes.contains(&"jenkins.declarative.invalid-context"));
        assert!(codes.contains(&"jenkins.plugin.unavailable"));
        assert!(codes.contains(&"jenkins.pipeline.cps-incompatible"));
        assert!(codes.contains(&"jenkins.pipeline.sandbox-rejected"));
        assert!(diagnostics.iter().all(|item| item["data"]["confidence"] == "verified"));
        assert!(diagnostics.iter().all(|item| item["data"]["profile"] == "diagnostic-profile"));
        assert!(diagnostics.iter().any(|item| item["data"]["sourceRevision"] == "2.27"));
        assert!(diagnostics.iter().any(|item| item["data"]["evidence"].as_array().is_some_and(|evidence| !evidence.is_empty())));
        assert!(diagnostics_for_document(Some(&runtime), "file:///workspace/vars/Library.groovy", text).is_empty());
    }

    #[test]
    fn definition_uses_safe_source_file_evidence_and_revision_pinned_urls() {
        let runtime = fixture_runtime_with_symbols("definition-profile", vec![json!({
            "id":"platform-lib:buildThing:global-variable",
            "name":"buildThing","kind":"global-variable","namespace":"vars","sourceId":"platform-lib",
            "signature":"buildThing","description":"A Shared Library global.","confidence":"verified",
            "contexts":["pipeline","scripted"],"provenance":["vars/buildThing.groovy"],
            "evidence":[{"sourceId":"platform-lib","kind":"source-scanned","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"0123456789abcdef0123456789abcdef01234567","location":"vars/buildThing.groovy:12"}]
        }), json!({
            "id":"platform-lib:annotatedGlobal:global-variable",
            "name":"annotatedGlobal","kind":"global-variable","namespace":"vars","sourceId":"platform-lib",
            "signature":"annotatedGlobal","description":"A global with an annotated source location.","confidence":"verified",
            "contexts":["pipeline","scripted"],"provenance":["vars/example1.groovy:1 (global script)"],
            "evidence":[{"sourceId":"platform-lib","kind":"source-scanned","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"0123456789abcdef0123456789abcdef01234567","location":"vars/example1.groovy:1 (global script)"}]
        })]);
        let locations = definition(Some(&runtime), Some("buildThing".into()));
        assert_eq!(locations.as_array().unwrap().len(), 1);
        let location = &locations[0];
        assert!(location["uri"].as_str().unwrap().contains("/blob/0123456789abcdef0123456789abcdef01234567/vars/buildThing.groovy#L12"));
        assert_eq!(location["range"]["start"]["line"], 11);
        let annotated = definition(Some(&runtime), Some("annotatedGlobal".into()));
        assert!(
            annotated[0]["uri"]
                .as_str()
                .unwrap()
                .ends_with("/vars/example1.groovy#L1")
        );
        assert_eq!(annotated[0]["range"]["start"]["line"], 0);
        assert!(definition(Some(&runtime), Some("sh".into())).is_null());
        assert!(parse_source_location("../secrets.groovy:4").is_none());
    }

    #[test]
    fn explicit_package_reload_replaces_profile_and_discards_stale_runtime_on_failure() {
        let first = fixture_package("profile-a", vec![json!({
            "id":"profile-a:only","name":"profileAOnly","kind":"global-variable","namespace":"vars",
            "sourceId":"platform-lib","signature":"profileAOnly","confidence":"verified","provenance":["vars/profileAOnly.groovy"]
        })]);
        let second = fixture_package("profile-b", vec![json!({
            "id":"profile-b:only","name":"profileBOnly","kind":"step","namespace":"org.jenkinsci.plugins.workflow",
            "sourceId":"workflow-basic-steps","signature":"profileBOnly()","confidence":"verified","contexts":["pipeline","steps"],
            "fieldEvidence":{"availability":"verified"},
            "evidence":[{"sourceId":"workflow-basic-steps","kind":"controller-observed","confidence":"verified","retrievedAt":"2026-01-01T00:00:00Z","revision":"2.27"}],
            "availability":{"status":"unavailable","reason":"not installed","pluginSourceIds":["workflow-basic-steps"]},
            "provenance":["plugin:workflow-basic-steps"]
        })]);
        let mut runtime = Some(Runtime::open(&first).unwrap());
        assert_eq!(runtime.as_ref().unwrap().profile(), "profile-a");
        let root = second.parent().unwrap();
        let relative = second.file_name().unwrap().to_string_lossy();
        let selected = select_runtime(&json!({"packagePath":relative,"profile":"profile-b"}), &mut runtime, Some(root)).unwrap();
        assert_eq!(selected, "profile-b");
        let names = runtime.as_ref().unwrap().symbols().iter().filter_map(|symbol| symbol["name"].as_str()).collect::<Vec<_>>();
        assert!(names.contains(&"profileBOnly"));
        assert!(!names.contains(&"profileAOnly"));
        let completion_result = completion(runtime.as_ref(), "node {\n  profileB", 1, 11);
        assert!(completion_result["items"].as_array().unwrap().iter().any(|item| item["label"] == "profileBOnly"));
        let hover_result = hover(runtime.as_ref(), Some("profileBOnly".into()));
        assert!(hover_result["contents"]["value"].as_str().unwrap().contains("profile-b"));
        let diagnostics = diagnostics_for_document(runtime.as_ref(), "file:///workspace/Jenkinsfile", "node {\n  profileBOnly()\n}");
        assert_eq!(diagnostics.len(), 1);
        assert!(select_runtime(&json!({"packagePath":first.to_string_lossy(),"profile":"wrong-profile"}), &mut runtime, None).is_err());
        assert!(runtime.is_none(), "failed profile selection must not retain stale symbols");
        assert!(completion(runtime.as_ref(), "node {\n  profileB", 1, 11)["items"].as_array().unwrap().is_empty());
        assert!(hover(runtime.as_ref(), Some("profileBOnly".into())).is_null());
        assert!(diagnostics_for_document(runtime.as_ref(), "file:///workspace/Jenkinsfile", "node {\n  profileBOnly()\n}").is_empty());
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

}
