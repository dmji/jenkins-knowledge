use jenkins_runtime::{Runtime, SearchOptions, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_OFFSET, MAX_SEARCH_RESULTS};
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
      {"name":"search_api","description":"Search catalog symbols with shared filters and a bounded result limit. Results include profile, confidence, and source provenance.","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"kind":{"type":"string"},"source":{"type":"string"},"plugin":{"type":"string"},"owner":{"type":"string"},"type":{"type":"string"},"confidence":{"enum":["verified","inferred","unknown"]},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"results":{"type":"array","items":{"type":"object"}},"limit":{"type":"integer"}},"required":["profile","results","limit"]}},
      {"name":"get_api_details","description":"Get one symbol by exact name or ID, including signature, parameters, provenance, profile, and confidence.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"}},"required":["id","name","profile","source","confidence"],"additionalProperties":true}},
      {"name":"list_api","description":"List catalog symbols with shared filters and a bounded result limit.","inputSchema":{"type":"object","properties":{"kind":{"type":"string"},"source":{"type":"string"},"plugin":{"type":"string"},"owner":{"type":"string"},"type":{"type":"string"},"confidence":{"enum":["verified","inferred","unknown"]},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false},"outputSchema":{"type":"object","properties":{"profile":{"type":"string"},"results":{"type":"array","items":{"type":"object"}},"limit":{"type":"integer"}},"required":["profile","results","limit"]}},
      {"name":"get_usage_examples","description":"Get catalog-backed usage examples for one exact symbol name or ID.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"},"examples":{"type":"array","items":{"type":"string"}}},"required":["id","name","profile","source","confidence","examples"],"additionalProperties":true}},
      {"name":"get_pipeline_step","description":"Get Jenkins Pipeline step metadata by exact name or symbol ID.","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false},"outputSchema":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"},"profile":{"type":"string"},"source":{"type":"object"},"confidence":{"type":"string"}},"required":["id","name","profile","source","confidence"],"additionalProperties":true}}
    ])
}
fn query_options(args: &Value, include_query: bool) -> Result<SearchOptions, String> {
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_SEARCH_LIMIT as u64) as usize;
    if limit == 0 || limit > MAX_SEARCH_RESULTS {
        return Err(format!("'limit' must be between 1 and {MAX_SEARCH_RESULTS}"));
    }
    Ok(SearchOptions {
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
        limit,
    })
}
fn call_tool(rt: &Runtime, name: &str, args: &Value) -> Result<Value,String> {
    match name {
        "search_api" => {
            let options = query_options(args, true)?;
            Ok(content(json!({"profile":rt.profile(),"limit":options.limit,"results":rt.search_with_options(&options)})))
        },
        "list_api" => {
            let options = query_options(args, false)?;
            Ok(content(json!({"profile":rt.profile(),"limit":options.limit,"results":rt.search_with_options(&options)})))
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
            let returned = page.results.len();
            Ok(json!({
                "profile": rt.profile(),
                "steps": page.results,
                "offset": page.offset,
                "limit": page.limit,
                "total": page.total,
                "returned": returned,
                "hasMore": page.has_more,
                "nextOffset": if page.has_more { Some(page.offset + returned) } else { None }
            }))
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
        for filter in ["plugin", "owner", "type", "confidence", "limit"] {
            assert!(search["inputSchema"]["properties"].get(filter).is_some());
        }
        for tool in definitions {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["properties"].is_object());
            assert_eq!(tool["outputSchema"]["type"], "object");
            assert!(tool["outputSchema"]["properties"].is_object());
        }
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
}
