use jenkins_runtime::{Runtime, SearchOptions, SearchPage, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_OFFSET, MAX_SEARCH_RESULTS};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
        .ok_or_else(|| format!("'{key}' must be a non-empty string"))
}
fn optional<'a>(args: &'a Value, key: &str) -> Option<&'a str> { args.get(key).and_then(Value::as_str) }
fn content(value: Value) -> Value {
    json!({"structuredContent":value.clone(),"content":[{"type":"text","text":serde_json::to_string_pretty(&value).unwrap_or_else(|e|e.to_string())}]})
}
fn tools() -> Value {
    json!([
      {"name":"search_api","description":"Search symbols across controller, plugin, Pipeline, Declarative, Groovy, and Shared Library sources with controller/source/plugin/kind/context/confidence filters and bounded pagination.","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"kind":{"type":"string"},"source":{"type":"string"},"controller":{"type":"string"},"sourceKind":{"type":"string"},"context":{"type":"string"},"plugin":{"type":"string"},"owner":{"type":"string"},"type":{"type":"string"},"confidence":{"enum":["verified","inferred","unknown"]},"offset":{"type":"integer","minimum":0,"maximum":10000},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"results":{"type":"array","items":{"type":"object"}},"offset":{"type":"integer"},"limit":{"type":"integer"},"total":{"type":"integer"},"returned":{"type":"integer"},"hasMore":{"type":"boolean"},"nextOffset":{"type":["integer","null"]},"paginationLimitReached":{"type":"boolean"}},"required":["profile","results","offset","limit","total","returned","hasMore","nextOffset","paginationLimitReached"]}},
      {"name":"get_api_details","description":"Get one symbol by exact name or ID, including source provenance, resolved type references and members, and explicit known/unknown execution constraints.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"}},"required":["id","name","profile","source","confidence"],"additionalProperties":true}},
      {"name":"list_api","description":"List catalog symbols across all configured sources with controller/source/plugin/kind/context/confidence filters and bounded pagination.","inputSchema":{"type":"object","properties":{"kind":{"type":"string"},"source":{"type":"string"},"controller":{"type":"string"},"sourceKind":{"type":"string"},"context":{"type":"string"},"plugin":{"type":"string"},"owner":{"type":"string"},"type":{"type":"string"},"confidence":{"enum":["verified","inferred","unknown"]},"offset":{"type":"integer","minimum":0,"maximum":10000},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"results":{"type":"array","items":{"type":"object"}},"offset":{"type":"integer"},"limit":{"type":"integer"},"total":{"type":"integer"},"returned":{"type":"integer"},"hasMore":{"type":"boolean"},"nextOffset":{"type":["integer","null"]},"paginationLimitReached":{"type":"boolean"}},"required":["profile","results","offset","limit","total","returned","hasMore","nextOffset","paginationLimitReached"]}},
      {"name":"get_usage_examples","description":"Get catalog-backed usage examples for one exact symbol name or ID.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"},"examples":{"type":"array","items":{"type":"string"}}},"required":["id","name","profile","source","confidence","examples"],"additionalProperties":true}},
      {"name":"get_pipeline_step","description":"Get Jenkins Pipeline step metadata, source, type relationships, and execution constraints by exact name or symbol ID.","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"}},"required":["id","name","profile","source","confidence"],"additionalProperties":true}},
      {"name":"get_controller_inventory","description":"Return the selected package's controller identity, Jenkins Core and plugin inventory, Java runtime metadata with per-property availability status, capabilities, collection profile, snapshot freshness timestamps, and source provenance. Inventory and capability status explicitly remain unknown when not inspected.","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"freshness":{"type":"object"},"controller":{"type":["object","null"]},"core":{"type":"array"},"plugins":{"type":"object"},"capabilities":{"type":"object"},"collection":{"type":"object"},"provenance":{"type":"object"},"compatibility":{"type":"object"}},"required":["profile","freshness","controller","core","plugins","capabilities","collection","provenance","compatibility"],"additionalProperties":false}},
      {"name":"get_package_compatibility","description":"Report the environment recorded by the selected package, package completeness, missing metadata, and known collection limitations. An optional caller-supplied targetEnvironment is compared offline; an exact match is not proof of runtime compatibility.","inputSchema":{"type":"object","properties":{"targetEnvironment":{"type":"object","properties":{"coreVersion":{"type":"string"},"javaVersion":{"type":"string"},"javaVendor":{"type":"string"},"plugins":{"type":"array","items":{"type":"object","properties":{"shortName":{"type":"string"},"version":{"type":"string"}},"required":["shortName","version"],"additionalProperties":false}}},"additionalProperties":false}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"status":{"enum":["unknown","environment-different","not-proven"]},"environmentComparisonStatus":{"enum":["not-compared","matched","different","unknown"]},"package":{"type":"object"},"packageEnvironment":{"type":"object"},"targetEnvironment":{"type":"object"},"comparison":{"type":"object"},"warnings":{"type":"array","items":{"type":"object"}}},"required":["status","environmentComparisonStatus","package","packageEnvironment","targetEnvironment","comparison","warnings"],"additionalProperties":false}}
    ])
}
fn query_options(args: &Value, include_query: bool) -> Result<(SearchOptions, usize), String> {
    let offset = match args.get("offset") {
        None => 0,
        Some(value) => value.as_u64()
            .ok_or_else(|| "'offset' must be a non-negative integer".to_owned())?
            .try_into()
            .map_err(|_| "'offset' is too large".to_owned())?,
    };
    if offset > MAX_SEARCH_OFFSET {
        return Err(format!("'offset' must not exceed {MAX_SEARCH_OFFSET}"));
    }

    let limit = match args.get("limit") {
        None => DEFAULT_SEARCH_LIMIT,
        Some(value) => {
            let limit = value.as_u64()
                .ok_or_else(|| "'limit' must be an integer between 1 and 100".to_owned())?;
            if !(1..=MAX_SEARCH_RESULTS as u64).contains(&limit) {
                return Err(format!("'limit' must be between 1 and {MAX_SEARCH_RESULTS}"));
            }
            limit as usize
        }
    };
    Ok((SearchOptions {
        query: if include_query { optional(args,"query").unwrap_or("").to_owned() } else { String::new() },
        kind: optional(args,"kind").map(str::to_owned),
        source: optional(args,"source").map(str::to_owned),
        controller: optional(args,"controller").map(str::to_owned),
        plugin: optional(args,"plugin").map(str::to_owned),
        owner: optional(args,"owner").map(str::to_owned),
        type_name: optional(args,"type").map(str::to_owned),
        confidence: optional(args,"confidence").map(str::to_owned),
        prefix: None,
        contexts: optional(args,"context").map(|context| vec![context.to_owned()]),
        kinds: None,
        source_kind: optional(args,"sourceKind").map(str::to_owned),
        limit,
    }, offset))
}

fn next_offset(offset: usize, returned: usize, has_more: bool) -> Option<usize> {
    let next = offset.saturating_add(returned);
    (has_more && next <= MAX_SEARCH_OFFSET).then_some(next)
}

fn page_metadata(page: &SearchPage) -> Value {
    let returned = page.results.len();
    let next = next_offset(page.offset, returned, page.has_more);
    json!({
        "offset": page.offset,
        "limit": page.limit,
        "total": page.total,
        "returned": returned,
        "hasMore": page.has_more,
        "nextOffset": next,
        "paginationLimitReached": page.has_more && next.is_none()
    })
}

fn paginated_results(rt: &Runtime, options: &SearchOptions, offset: usize) -> Result<Value, String> {
    let page = rt.search_page(options, offset).map_err(|error| error.to_string())?;
    let mut response = page_metadata(&page);
    response["profile"] = json!(rt.profile());
    response["results"] = json!(page.results);
    Ok(content(response))
}

fn call_tool(rt: &Runtime, name: &str, args: &Value) -> Result<Value,String> {
    match name {
        "search_api" => {
            let (options, offset) = query_options(args, true)?;
            paginated_results(rt, &options, offset)
        },
        "list_api" => {
            let (options, offset) = query_options(args, false)?;
            paginated_results(rt, &options, offset)
        },
        "get_api_details" => rt.get(required(args,"name_or_id")?).map(content).map_err(|e|e.to_string()),
        "get_usage_examples" => rt.examples(required(args,"name_or_id")?).map(content).map_err(|e|e.to_string()),
        "get_pipeline_step" => rt.pipeline_step(required(args,"name")?).map(content).map_err(|e|e.to_string()),
        "get_controller_inventory" => Ok(content(controller_inventory(rt))),
        "get_package_compatibility" => Ok(content(rt.compatibility_report(args.get("targetEnvironment")))),
        _ => Err(format!("unknown tool '{name}'")),
    }
}

fn pagination_page(uri: &str) -> Result<(usize, usize), String> {
    let (_, query) = uri.split_once('?').unwrap_or((uri, ""));
    let mut offset = 0usize;
    let mut limit = DEFAULT_SEARCH_LIMIT;
    let mut seen_offset = false;
    let mut seen_limit = false;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').ok_or_else(|| "pagination parameters must use key=value".to_owned())?;
        match key {
            "offset" if !seen_offset => {
                offset = value.parse().map_err(|_| "offset must be a non-negative integer".to_owned())?;
                seen_offset = true;
            }
            "limit" if !seen_limit => {
                limit = value.parse().map_err(|_| "limit must be an integer".to_owned())?;
                seen_limit = true;
            }
            "offset" | "limit" => return Err(format!("duplicate pagination parameter '{key}'")),
            _ => return Err(format!("unsupported pagination parameter '{key}'")),
        }
    }
    if offset > MAX_SEARCH_OFFSET {
        return Err(format!("offset must not exceed {MAX_SEARCH_OFFSET}"));
    }
    if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
        return Err(format!("limit must be between 1 and {MAX_SEARCH_RESULTS}"));
    }
    Ok((offset, limit))
}

fn read_symbol_resource(rt: &Runtime, uri: &str, key: &str, kind: Option<&str>, source_kind: Option<&str>) -> Result<Value,String> {
    let (offset, limit) = pagination_page(uri)?;
    let options = SearchOptions {
        kind: kind.map(str::to_owned),
        source_kind: source_kind.map(str::to_owned),
        limit,
        ..SearchOptions::default()
    };
    let page = rt.search_page(&options, offset).map_err(|error| error.to_string())?;
    let mut response = page_metadata(&page);
    response["profile"] = json!(rt.profile());
    response[key] = json!(page.results);
    Ok(response)
}

fn controller_inventory(rt: &Runtime) -> Value {
    let controller = rt.controller_details();
    let sources = rt.source_details();
    let controller_id = controller.as_ref().and_then(|value| value["identity"]["id"].as_str());
    let controller_source = sources.iter().find(|source| {
        source["kind"] == "jenkins-controller" && source["id"].as_str() == controller_id
    }).or_else(|| sources.iter().find(|source| source["kind"] == "jenkins-controller")).cloned();
    let core_sources = sources.iter().filter(|source| source["kind"] == "jenkins-core").cloned().collect::<Vec<_>>();
    let plugin_sources = sources.iter().filter(|source| source["kind"] == "plugin").cloned().collect::<Vec<_>>();
    let collection = controller.as_ref().and_then(|value| value.get("collection")).cloned().unwrap_or(Value::Null);
    let observed_at = collection["retrievedAt"].as_str()
        .or_else(|| controller.as_ref().and_then(|value| value["runtime"]["collectedAt"].as_str()));
    let freshness = json!({
        "status": if observed_at.is_some() { "known" } else { "unknown" },
        "packageGeneratedAt": rt.manifest()["createdAt"],
        "controllerObservedAt": observed_at,
        "snapshotId": rt.catalog()["snapshotId"],
        "catalogId": rt.catalog()["catalogId"]
    });
    let capabilities = controller.as_ref().and_then(|value| value["capabilities"].as_array()).cloned();
    json!({
        "profile": rt.profile(),
        "freshness": freshness,
        "controller": controller,
        "core": core_sources,
        "plugins": {
            "status": rt.plugin_inventory_status(),
            "items": rt.plugins(),
            "sources": plugin_sources
        },
        "capabilities": {
            "status": if capabilities.is_some() { "known" } else { "unknown" },
            "items": capabilities.unwrap_or_default()
        },
        "collection": {
            "controller": collection,
            "package": rt.collection().cloned().unwrap_or(Value::Null),
            "completeness": rt.catalog()["completeness"]
        },
        "compatibility": rt.compatibility_report(None),
        "provenance": {
            "controllerSource": controller_source,
            "coreSources": core_sources.clone(),
            "pluginSources": plugin_sources.clone()
        }
    })
}

fn read_resource(rt: &Runtime, uri: &str) -> Result<Value,String> {
    match uri {
        "jenkins://catalog/manifest" => Ok(rt.manifest().clone()),
        "jenkins://catalog/collection" => Ok(rt.collection().cloned().unwrap_or_else(|| json!({
            "status": "unknown",
            "sources": [],
            "details": "This package predates explicit collection outcome metadata"
        }))),
        "jenkins://catalog/coverage" => Ok(json!({"profile":rt.profile(),"coverage":rt.coverage()})),
        "jenkins://catalog/controller" => Ok(json!({"profile":rt.profile(),"controller":rt.controller_details()})),
        "jenkins://catalog/controller-inventory" => Ok(controller_inventory(rt)),
        "jenkins://catalog/compatibility" => Ok(rt.compatibility_report(None)),
        "jenkins://catalog/plugins" => Ok(json!({"profile":rt.profile(),"status":rt.plugin_inventory_status(),"plugins":rt.plugins()})),
        "jenkins://catalog/sources" => Ok(json!({"profile":rt.profile(),"sources":rt.source_details()})),
        _ if uri == "jenkins://catalog/types" || uri.starts_with("jenkins://catalog/types?") => {
            let (offset, limit) = pagination_page(uri)?;
            let options = SearchOptions { kind: Some("class".into()), limit, ..SearchOptions::default() };
            let page = rt.search_page(&options, offset).map_err(|error| error.to_string())?;
            let mut response = page_metadata(&page);
            response["profile"] = json!(rt.profile());
            let types = page.results.iter().filter_map(|symbol| {
                symbol["id"].as_str().and_then(|id| rt.get(id).ok())
            }).collect::<Vec<_>>();
            response["types"] = json!(types);
            Ok(response)
        },
        _ if uri == "jenkins://catalog/pipeline-steps" || uri.starts_with("jenkins://catalog/pipeline-steps?") => {
            let (offset, limit) = pagination_page(uri)?;
            let options = SearchOptions { kind: Some("step".into()), limit, ..SearchOptions::default() };
            let page = rt.search_page(&options, offset).map_err(|error| error.to_string())?;
            let mut response = page_metadata(&page);
            response["profile"] = json!(rt.profile());
            let steps = page.results.iter().filter_map(|symbol| {
                symbol["id"].as_str().and_then(|id| rt.pipeline_step(id).ok())
            }).collect::<Vec<_>>();
            response["steps"] = json!(steps);
            Ok(response)
        },
        _ if uri == "jenkins://catalog/globals" || uri.starts_with("jenkins://catalog/globals?") =>
            read_symbol_resource(rt, uri, "globals", Some("global-variable"), None),
        _ if uri == "jenkins://catalog/declarative" || uri.starts_with("jenkins://catalog/declarative?") =>
            read_symbol_resource(rt, uri, "constructs", Some("declarative-directive"), None),
        _ if uri == "jenkins://catalog/library-symbols" || uri.starts_with("jenkins://catalog/library-symbols?") =>
            read_symbol_resource(rt, uri, "symbols", None, Some("shared-library")),
        _ if uri == "jenkins://catalog/plugin-symbols" || uri.starts_with("jenkins://catalog/plugin-symbols?") =>
            read_symbol_resource(rt, uri, "symbols", None, Some("plugin")),
        "jenkins://catalog/libraries" => Ok(json!({"profile":rt.profile(),"sources":rt.source_details().into_iter().filter(|source|source["kind"]=="shared-library").collect::<Vec<_>>()})),
        _ if uri.starts_with("jenkins://catalog/profiles/") => {
            let profile=uri.trim_start_matches("jenkins://catalog/profiles/");
            if profile!=rt.profile() { return Err(format!("profile '{profile}' is not loaded; launch the server with that profile's package")); }
            Ok(json!({"profile":rt.profile(),"manifest":rt.manifest(),"catalogId":rt.catalog()["catalogId"],"schemaVersion":rt.catalog()["schemaVersion"]}))
        }
        _ => Err(format!("unknown resource '{uri}'")),
    }
}
fn reply(id: Value, result: Value) -> Value { json!({"jsonrpc":"2.0","id":id,"result":result}) }
fn handle(rt: &Runtime, req: Value) -> Option<Value> {
    let id=req.get("id").cloned()?;
    let method=req.get("method").and_then(Value::as_str).unwrap_or("");
    let params=req.get("params").cloned().unwrap_or_else(||json!({}));
    let result: Result<Value,Value> = match method {
        "initialize" => Ok(json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{"subscribe":false,"listChanged":false}},"serverInfo":{"name":"jenkins-knowledge","version":env!("CARGO_PKG_VERSION")}})),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools":tools()})),
        "tools/call" => {
            let name=params.get("name").and_then(Value::as_str).unwrap_or("");
            let args=params.get("arguments").cloned().unwrap_or_else(||json!({}));
            match call_tool(rt,name,&args) { Ok(v)=>Ok(v), Err(e)=>Ok(json!({"content":[{"type":"text","text":e}],"isError":true})) }
        }
        "resources/list" => Ok(json!({"resources":[
            {"uri":"jenkins://catalog/manifest","name":"Knowledge package manifest","mimeType":"application/json"},
            {"uri":"jenkins://catalog/collection","name":"Package collection outcomes","description":"Package-level completeness and per-source succeeded, unavailable, permission-denied, failed, skipped, or unconfigured outcomes.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/coverage","name":"Catalog collection coverage","mimeType":"application/json"},
            {"uri":"jenkins://catalog/controller","name":"Jenkins controller metadata","mimeType":"application/json"},
            {"uri":"jenkins://catalog/controller-inventory","name":"Controller inventory and snapshot provenance","description":"Combined controller identity, Core/plugin inventory, capability status, collection profile, snapshot freshness, and source provenance.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/compatibility","name":"Package environment and compatibility report","description":"Offline comparison of recorded package environment with optional caller-supplied target metadata; matching versions do not prove compatibility.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/plugins","name":"Installed Jenkins plugins","mimeType":"application/json"},
            {"uri":"jenkins://catalog/sources","name":"Knowledge package sources","mimeType":"application/json"},
            {"uri":"jenkins://catalog/types","name":"Groovy types and members","description":"Paginated with ?offset=0&limit=50; each type includes catalog-declared members and resolved type references.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/libraries","name":"Shared Library sources","mimeType":"application/json"},
            {"uri":"jenkins://catalog/pipeline-steps","name":"Jenkins Pipeline steps","description":"Paginated with ?offset=0&limit=50; responses include resolved step details and execution constraints.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/globals","name":"Pipeline and Shared Library globals","description":"Paginated global-variable symbols with source and confidence metadata.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/declarative","name":"Declarative Pipeline constructs","description":"Paginated Declarative directive symbols with execution-context evidence.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/library-symbols","name":"Shared Library symbols","description":"Paginated symbols scanned from configured Shared Libraries.","mimeType":"application/json"},
            {"uri":"jenkins://catalog/plugin-symbols","name":"Plugin-provided symbols","description":"Paginated symbols provided by installed Jenkins plugins.","mimeType":"application/json"},
            {"uri":format!("jenkins://catalog/profiles/{}",rt.profile()),"name":"Selected profile","mimeType":"application/json"}
        ]})),
        "resources/read" => {
            let uri=params.get("uri").and_then(Value::as_str).unwrap_or("");
            match read_resource(rt,uri) {
                Ok(v)=>Ok(json!({"contents":[{"uri":uri,"mimeType":"application/json","text":serde_json::to_string_pretty(&v).unwrap_or_default()}]})),
                Err(e)=>Err(json!({"code":-32602,"message":e}))
            }
        }
        _ => Err(json!({"code":-32601,"message":format!("method not found: {method}")})),
    };
    Some(match result { Ok(v)=>reply(id,v), Err(e)=>json!({"jsonrpc":"2.0","id":id,"error":e}) })
}
pub fn serve(package: &str) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = Runtime::open(package)?;
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(&runtime, request),
            Err(error) => Some(json!({
                "jsonrpc":"2.0",
                "id":null,
                "error":{"code":-32700,"message":error.to_string()}
            }))
        };
        if let Some(response) = response {
            if writeln!(stdout, "{response}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut package = None;
    while let Some(arg) = args.next() {
        if arg == "--package" || arg == "-p" {
            package = args.next();
        }
    }
    let package = package
        .or_else(|| std::env::var("JENKINS_KNOWLEDGE_PACKAGE").ok())
        .unwrap_or_else(|| {
            eprintln!("Usage: jenkins-mcp --package <knowledge-package-dir>");
            std::process::exit(2)
        });
    if let Err(error) = serve(&package) {
        eprintln!("Cannot serve Knowledge Package: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_public_tool_has_an_object_input_schema() {
        let definitions = tools();
        let definitions = definitions.as_array().unwrap();
        let names: Vec<_> = definitions.iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert_eq!(names, ["search_api", "get_api_details", "list_api", "get_usage_examples", "get_pipeline_step", "get_controller_inventory", "get_package_compatibility"]);
        let search = definitions.iter().find(|tool| tool["name"] == "search_api").unwrap();
        for filter in ["controller", "sourceKind", "context", "plugin", "owner", "type", "confidence", "offset", "limit"] {
            assert!(search["inputSchema"]["properties"].get(filter).is_some());
        }
        for tool in definitions {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["properties"].is_object());
            assert_eq!(tool["outputSchema"]["type"], "object");
            assert!(tool["outputSchema"]["properties"].is_object());
        }
    }

    fn structured_tool_page(runtime: &Runtime, name: &str, args: Value) -> Value {
        call_tool(runtime, name, &args).unwrap()["structuredContent"].clone()
    }

    #[test]
    fn search_api_pages_cover_all_matches_without_duplicates() {
        let runtime = fixture_runtime();
        let pages: Vec<_> = [0, 5, 10].into_iter().map(|offset| {
            structured_tool_page(&runtime, "search_api", json!({
                "query": "page-",
                "offset": offset,
                "limit": 5
            }))
        }).collect();

        assert_eq!(pages[0]["total"], 12);
        assert_eq!(pages[0]["returned"], 5);
        assert_eq!(pages[0]["hasMore"], true);
        assert_eq!(pages[0]["nextOffset"], 5);
        assert_eq!(pages[1]["nextOffset"], 10);
        assert_eq!(pages[2]["returned"], 2);
        assert_eq!(pages[2]["hasMore"], false);
        assert!(pages[2]["nextOffset"].is_null());

        let ids: Vec<_> = pages.iter().flat_map(|page| {
            page["results"].as_array().unwrap().iter()
                .map(|symbol| symbol["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }).collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), 12);
        assert_eq!(unique.len(), 12);
    }

    #[test]
    fn list_api_pages_cover_all_filtered_symbols_without_duplicates() {
        let runtime = fixture_runtime();
        let pages: Vec<_> = [0, 5, 10].into_iter().map(|offset| {
            structured_tool_page(&runtime, "list_api", json!({
                "kind": "step",
                "offset": offset,
                "limit": 5
            }))
        }).collect();

        assert_eq!(pages[0]["total"], 13);
        assert_eq!(pages[0]["returned"], 5);
        assert_eq!(pages[0]["hasMore"], true);
        assert_eq!(pages[1]["hasMore"], true);
        assert_eq!(pages[2]["returned"], 3);
        assert_eq!(pages[2]["hasMore"], false);

        let ids: Vec<_> = pages.iter().flat_map(|page| {
            page["results"].as_array().unwrap().iter()
                .map(|symbol| symbol["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }).collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), 13);
        assert_eq!(unique.len(), 13);
    }

    #[test]
    fn search_and_list_tools_report_empty_and_default_pages() {
        let runtime = fixture_runtime();
        let empty = structured_tool_page(&runtime, "search_api", json!({"query": "no-such-symbol"}));
        assert_eq!(empty["total"], 0);
        assert_eq!(empty["returned"], 0);
        assert_eq!(empty["hasMore"], false);
        assert!(empty["nextOffset"].is_null());

        let default = structured_tool_page(&runtime, "list_api", json!({}));
        assert_eq!(default["offset"], 0);
        assert_eq!(default["limit"], DEFAULT_SEARCH_LIMIT);
        assert_eq!(default["total"], 18);
        assert_eq!(default["returned"], 18);
        assert_eq!(default["hasMore"], false);
    }


    #[test]
    fn search_api_exposes_controller_context_and_source_kind_filters() {
        let runtime = fixture_runtime();
        let page = structured_tool_page(&runtime, "search_api", json!({
            "query":"sh",
            "controller":"jenkins-ci",
            "sourceKind":"jenkins-controller",
            "context":"steps"
        }));
        assert_eq!(page["total"], 1);
        assert_eq!(page["results"][0]["name"], "sh");
        let mismatch = structured_tool_page(&runtime, "list_api", json!({"controller":"other"}));
        assert_eq!(mismatch["total"], 0);
    }

    #[test]
    fn controller_plugin_source_and_type_resources_are_available() {
        let runtime = fixture_runtime();
        let controller = read_resource(&runtime, "jenkins://catalog/controller").unwrap();
        let plugins = read_resource(&runtime, "jenkins://catalog/plugins").unwrap();
        let sources = read_resource(&runtime, "jenkins://catalog/sources").unwrap();
        let types = read_resource(&runtime, "jenkins://catalog/types").unwrap();
        let globals = read_resource(&runtime, "jenkins://catalog/globals").unwrap();
        let declarative = read_resource(&runtime, "jenkins://catalog/declarative").unwrap();
        let library_symbols = read_resource(&runtime, "jenkins://catalog/library-symbols").unwrap();
        let plugin_symbols = read_resource(&runtime, "jenkins://catalog/plugin-symbols").unwrap();
        assert_eq!(controller["controller"]["identity"]["id"], "jenkins-ci");
        assert_eq!(plugins["status"], "partial");
        assert_eq!(plugins["plugins"].as_array().unwrap().len(), 1);
        assert!(sources["sources"].as_array().unwrap().iter().any(|source| source["kind"] == "shared-library"));
        assert!(types["types"].as_array().unwrap().iter().any(|symbol| symbol["name"] == "ShellStep"));
        assert_eq!(types["total"], 1);
        assert_eq!(globals["globals"][0]["name"], "deployApp");
        assert_eq!(declarative["constructs"][0]["name"], "agent");
        assert_eq!(library_symbols["symbols"][0]["name"], "deployApp");
        assert_eq!(plugin_symbols["total"], 3);
        assert!(plugin_symbols["symbols"].as_array().unwrap().iter().all(|symbol| symbol["source"]["kind"] == "plugin"));
        let paged_types = read_resource(&runtime, "jenkins://catalog/types?offset=0&limit=1").unwrap();
        assert_eq!(paged_types["returned"], 1);
        assert_eq!(types["types"][0]["membersStatus"], "unknown");
        assert_eq!(types["types"][0]["controller"]["id"], "jenkins-ci");
    }
    #[test]
    fn compatibility_tool_and_resource_expose_package_limitations_without_a_target() {
        let runtime = fixture_runtime();
        let tool = structured_tool_page(&runtime, "get_package_compatibility", json!({}));
        let resource = read_resource(&runtime, "jenkins://catalog/compatibility").unwrap();
        let inventory = controller_inventory(&runtime);
        let listed = handle(&runtime, json!({"id":1,"method":"resources/list"})).unwrap();

        assert_eq!(tool["status"], "unknown");
        assert_eq!(tool["environmentComparisonStatus"], "not-compared");
        assert_eq!(tool["targetEnvironment"]["status"], "not-provided");
        assert_eq!(resource["status"], "unknown");
        assert_eq!(inventory["freshness"]["packageGeneratedAt"], runtime.manifest()["createdAt"]);
        assert!(inventory.get("compatibility").is_some());
        assert!(listed["result"]["resources"].as_array().unwrap().iter()
            .any(|resource| resource["uri"] == "jenkins://catalog/compatibility"));
    }

    #[test]
    fn compatibility_tool_reports_a_known_environment_difference() {
        let runtime = fixture_runtime();
        let report = structured_tool_page(&runtime, "get_package_compatibility", json!({
            "targetEnvironment": {
                "coreVersion":"2.480.0",
                "javaVersion":"17.0.12",
                "javaVendor":"Eclipse Adoptium",
                "plugins":[{"shortName":"workflow-basic-steps","version":"2.27"}]
            }
        }));
        assert_eq!(report["environmentComparisonStatus"], "different");
        assert_eq!(report["status"], "environment-different");
    }

    #[test]
    fn page_metadata_marks_when_runtime_offset_ceiling_blocks_next_page() {
        let page = SearchPage {
            offset: MAX_SEARCH_OFFSET,
            limit: 5,
            total: MAX_SEARCH_OFFSET + 6,
            has_more: true,
            results: vec![json!({}); 5],
        };
        let metadata = page_metadata(&page);
        assert_eq!(metadata["hasMore"], true);
        assert!(metadata["nextOffset"].is_null());
        assert_eq!(metadata["paginationLimitReached"], true);
    }

    #[test]
    fn search_and_list_tools_reject_invalid_offsets_and_limits() {
        let runtime = fixture_runtime();
        for arguments in [
            json!({"offset": -1}),
            json!({"offset": 1.5}),
            json!({"offset": "1"}),
            json!({"offset": MAX_SEARCH_OFFSET + 1}),
            json!({"limit": 0}),
            json!({"limit": -1}),
            json!({"limit": 1.5}),
            json!({"limit": MAX_SEARCH_RESULTS + 1}),
        ] {
            assert!(call_tool(&runtime, "search_api", &arguments).is_err(), "{arguments}");
            assert!(call_tool(&runtime, "list_api", &arguments).is_err(), "{arguments}");
        }
    }

    #[test]
    fn next_offset_never_advertises_a_page_outside_runtime_bounds() {
        assert_eq!(next_offset(0, 5, true), Some(5));
        assert_eq!(next_offset(MAX_SEARCH_OFFSET - 5, 5, true), Some(MAX_SEARCH_OFFSET));
        assert_eq!(next_offset(MAX_SEARCH_OFFSET, 5, true), None);
        assert_eq!(next_offset(0, 5, false), None);
    }

    #[test]
    fn detail_tools_require_their_lookup_argument() {
        let definitions = tools();
        for (name, argument) in [
            ("get_api_details", "name_or_id"),
            ("get_usage_examples", "name_or_id"),
            ("get_pipeline_step", "name"),
        ] {
            let tool = definitions.as_array().unwrap().iter().find(|tool| tool["name"] == name).unwrap();
            assert!(tool["inputSchema"]["required"].as_array().unwrap().iter().any(|v| v == argument));
        }
    }
    static NEXT_PACKAGE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn fixture_runtime() -> Runtime {
        use sha2::Digest;
        use std::fs;
        let package = std::env::temp_dir().join(format!(
            "jenkins-mcp-test-{}-{}",
            std::process::id(),
            NEXT_PACKAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&package);
        fs::create_dir_all(&package).unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        for index in 0..12 {
            catalog["symbols"].as_array_mut().unwrap().push(json!({
                "id": format!("page:{index:03}"),
                "name": format!("page-{index:03}"),
                "kind": "step",
                "sourceId": "controller",
                "contexts": ["steps"],
                "confidence": "verified",
                "provenance": ["test"]
            }));
        }
        let catalog_bytes = serde_json::to_vec(&catalog).unwrap();
        fs::write(package.join("catalog.json"), &catalog_bytes).unwrap();
        let checksum = sha2::Sha256::digest(&catalog_bytes)
            .iter().map(|byte| format!("{byte:02x}")).collect::<String>();
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
            "profile": "fixture",
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
        let runtime = Runtime::open(&package).unwrap();
        fs::remove_dir_all(package).unwrap();
        runtime
    }

    #[test]
    fn collection_resource_preserves_partial_outcomes_and_legacy_unknown() {
        let runtime = fixture_runtime();
        let collection = read_resource(&runtime, "jenkins://catalog/collection").unwrap();
        assert_eq!(collection["status"], "partial");
        assert_eq!(collection["sources"][0]["status"], "unavailable");
        assert_eq!(read_resource(&runtime, "jenkins://catalog/manifest").unwrap()["collection"]["status"], "partial");
    }

    #[test]
    fn pipeline_resource_pages_cover_all_steps_without_duplicates() {
        let runtime = fixture_runtime();
        let first = read_resource(&runtime, "jenkins://catalog/pipeline-steps?offset=0&limit=5").unwrap();
        let second = read_resource(&runtime, "jenkins://catalog/pipeline-steps?offset=5&limit=5").unwrap();
        let third = read_resource(&runtime, "jenkins://catalog/pipeline-steps?offset=10&limit=5").unwrap();
        assert_eq!(first["total"], 13);
        assert_eq!(first["returned"], 5);
        assert_eq!(first["hasMore"], true);
        assert_eq!(second["hasMore"], true);
        assert_eq!(third["hasMore"], false);
        let ids: Vec<_> = [first, second, third].into_iter()
            .flat_map(|page| page["steps"].as_array().unwrap().iter()
                .map(|step| step["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>())
            .collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), 13);
        assert_eq!(unique.len(), 13);
    }

    #[test]
    fn pipeline_resource_keeps_legacy_uri_and_reports_default_page_metadata() {
        let runtime = fixture_runtime();
        let page = read_resource(&runtime, "jenkins://catalog/pipeline-steps").unwrap();
        assert_eq!(page["offset"], 0);
        assert_eq!(page["limit"], DEFAULT_SEARCH_LIMIT);
        assert_eq!(page["total"], 13);
        assert_eq!(page["returned"], 13);
        assert_eq!(page["hasMore"], false);
        assert!(page["nextOffset"].is_null());
        assert_eq!(page["paginationLimitReached"], false);
    }

    #[test]
    fn pipeline_resource_rejects_invalid_pagination_parameters() {
        assert!(pagination_page("jenkins://catalog/pipeline-steps?offset=-1").is_err());
        assert!(pagination_page("jenkins://catalog/pipeline-steps?limit=0").is_err());
        assert!(pagination_page("jenkins://catalog/pipeline-steps?offset=1&offset=2").is_err());
        assert!(pagination_page("jenkins://catalog/pipeline-steps?cursor=bad").is_err());
        assert!(pagination_page(&format!("jenkins://catalog/pipeline-steps?offset={}", MAX_SEARCH_OFFSET + 1)).is_err());
    }

}
