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
      {"name":"search_api","description":"Search catalog symbols with shared filters and bounded pagination. Responses include total, returned, hasMore, and nextOffset.","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"kind":{"type":"string"},"source":{"type":"string"},"plugin":{"type":"string"},"owner":{"type":"string"},"type":{"type":"string"},"confidence":{"enum":["verified","inferred","unknown"]},"offset":{"type":"integer","minimum":0,"maximum":10000},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"results":{"type":"array","items":{"type":"object"}},"offset":{"type":"integer"},"limit":{"type":"integer"},"total":{"type":"integer"},"returned":{"type":"integer"},"hasMore":{"type":"boolean"},"nextOffset":{"type":["integer","null"]},"paginationLimitReached":{"type":"boolean"}},"required":["profile","results","offset","limit","total","returned","hasMore","nextOffset","paginationLimitReached"]}},
      {"name":"get_api_details","description":"Get one symbol by exact name or ID, including signature, parameters, provenance, profile, and confidence.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"}},"required":["id","name","profile","source","confidence"],"additionalProperties":true}},
      {"name":"list_api","description":"List catalog symbols with shared filters and bounded pagination. Responses include total, returned, hasMore, and nextOffset.","inputSchema":{"type":"object","properties":{"kind":{"type":"string"},"source":{"type":"string"},"plugin":{"type":"string"},"owner":{"type":"string"},"type":{"type":"string"},"confidence":{"enum":["verified","inferred","unknown"]},"offset":{"type":"integer","minimum":0,"maximum":10000},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"results":{"type":"array","items":{"type":"object"}},"offset":{"type":"integer"},"limit":{"type":"integer"},"total":{"type":"integer"},"returned":{"type":"integer"},"hasMore":{"type":"boolean"},"nextOffset":{"type":["integer","null"]},"paginationLimitReached":{"type":"boolean"}},"required":["profile","results","offset","limit","total","returned","hasMore","nextOffset","paginationLimitReached"]}},
      {"name":"get_usage_examples","description":"Get catalog-backed usage examples for one exact symbol name or ID.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"},"examples":{"type":"array","items":{"type":"string"}}},"required":["id","name","profile","source","confidence","examples"],"additionalProperties":true}},
      {"name":"get_pipeline_step","description":"Get Jenkins Pipeline step metadata by exact name or symbol ID.","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"}},"required":["id","name","profile","source","confidence"],"additionalProperties":true}}
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
        plugin: optional(args,"plugin").map(str::to_owned),
        owner: optional(args,"owner").map(str::to_owned),
        type_name: optional(args,"type").map(str::to_owned),
        confidence: optional(args,"confidence").map(str::to_owned),
        prefix: None,
        contexts: None,
        kinds: None,
        source_kind: None,
        limit,
    }, offset)
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
        _ => Err(format!("unknown tool '{name}'")),
    }
}

fn pipeline_page(uri: &str) -> Result<(usize, usize), String> {
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

fn read_resource(rt: &Runtime, uri: &str) -> Result<Value,String> {
    match uri {
        "jenkins://catalog/manifest" => Ok(rt.manifest().clone()),
        "jenkins://catalog/coverage" => Ok(json!({"profile":rt.profile(),"coverage":rt.coverage()})),
        "jenkins://catalog/libraries" => Ok(json!({"profile":rt.profile(),"sources":rt.sources().iter().filter(|s|s["kind"]=="shared-library").collect::<Vec<_>>()})),
        _ if uri == "jenkins://catalog/pipeline-steps" || uri.starts_with("jenkins://catalog/pipeline-steps?") => {
            let (offset, limit) = pipeline_page(uri)?;
            let options = SearchOptions {
                kind: Some("step".into()),
                limit,
                ..SearchOptions::default()
            };
            let page = rt.search_page(&options, offset).map_err(|error| error.to_string())?;
            let mut response = page_metadata(&page);
            response["profile"] = json!(rt.profile());
            response["steps"] = json!(page.results);
            Ok(response)
        },
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
            {"uri":"jenkins://catalog/coverage","name":"Catalog collection coverage","mimeType":"application/json"},
            {"uri":"jenkins://catalog/libraries","name":"Shared Library sources","mimeType":"application/json"},
            {"uri":"jenkins://catalog/pipeline-steps","name":"Jenkins Pipeline steps","description":"Paginated with ?offset=0&limit=50; responses include total, returned, and hasMore.","mimeType":"application/json"},
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
fn main() {
    let mut args=std::env::args().skip(1); let mut package=None;
    while let Some(arg)=args.next() { if arg=="--package" || arg=="-p" { package=args.next(); } }
    let package=package.or_else(||std::env::var("JENKINS_KNOWLEDGE_PACKAGE").ok()).unwrap_or_else(||{
        eprintln!("Usage: jenkins-mcp --package <knowledge-package-dir>"); std::process::exit(2)
    });
    let runtime=Runtime::open(package).unwrap_or_else(|e|{eprintln!("Cannot load Knowledge Package: {e}");std::process::exit(2)});
    let stdin=io::stdin(); let mut stdout=io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line)=line else { break }; if line.trim().is_empty(){continue;}
        let response=match serde_json::from_str::<Value>(&line) {
            Ok(req)=>handle(&runtime,req),
            Err(e)=>Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}}))
        };
        if let Some(response)=response { if writeln!(stdout,"{response}").and_then(|_|stdout.flush()).is_err(){break;} }
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
        assert_eq!(names, ["search_api", "get_api_details", "list_api", "get_usage_examples", "get_pipeline_step"]);
        let search = definitions.iter().find(|tool| tool["name"] == "search_api").unwrap();
        for filter in ["plugin", "owner", "type", "confidence", "offset", "limit"] {
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
        assert_eq!(default["total"], 13);
        assert_eq!(default["returned"], 13);
        assert_eq!(default["hasMore"], false);
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
            "toolVersions": {}
        }).to_string()).unwrap();
        let runtime = Runtime::open(&package).unwrap();
        fs::remove_dir_all(package).unwrap();
        runtime
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
        assert!(pipeline_page("jenkins://catalog/pipeline-steps?offset=-1").is_err());
        assert!(pipeline_page("jenkins://catalog/pipeline-steps?limit=0").is_err());
        assert!(pipeline_page("jenkins://catalog/pipeline-steps?offset=1&offset=2").is_err());
        assert!(pipeline_page("jenkins://catalog/pipeline-steps?cursor=bad").is_err());
        assert!(pipeline_page(&format!("jenkins://catalog/pipeline-steps?offset={}", MAX_SEARCH_OFFSET + 1)).is_err());
    }

}
