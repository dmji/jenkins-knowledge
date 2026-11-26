use jenkins_runtime::{Runtime, SearchOptions, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_RESULTS};
use serde_json::json;
use std::{env, process};

fn usage() {
    eprintln!("Usage: jenkins-runtime --package <dir|manifest.json> [--profile <name>] <search|list|get|examples|step|steps|globals|declarative|types|library-symbols|plugin-symbols|controller|plugins|sources|coverage> [arguments] [--kind <kind>] [--source <source-id>] [--controller <id|name|url>] [--source-kind <kind>] [--plugin <id>] [--owner <name>] [--type <type>] [--context <context>] [--confidence <level>] [--limit <1..100>]");
    eprintln!("Commands: search <query>, list, get <name-or-id>, examples <name-or-id>, step <step-name>, steps, globals, declarative, types, library-symbols, plugin-symbols, controller, plugins, sources, coverage");
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1).peekable();
    let mut package = None;
    let mut profile = None;
    let mut positional = Vec::new();
    let mut kind = None;
    let mut source = None;
    let mut controller = None;
    let mut source_kind = None;
    let mut context = None;
    let mut plugin = None;
    let mut owner = None;
    let mut type_name = None;
    let mut confidence = None;
    let mut limit = DEFAULT_SEARCH_LIMIT;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--package" | "-p" => package = args.next(),
            "--profile" => profile = args.next(),
            "--kind" => kind = args.next(),
            "--source" => source = args.next(),
            "--controller" => controller = args.next(),
            "--source-kind" => source_kind = args.next(),
            "--context" => context = args.next(),
            "--plugin" => plugin = args.next(),
            "--owner" => owner = args.next(),
            "--type" => type_name = args.next(),
            "--confidence" => confidence = args.next(),
            "--limit" => limit = args.next().ok_or("missing value for --limit")?.parse()?,
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            _ => positional.push(arg),
        }
    }
    let package = package.ok_or("missing required --package")?;
    let command = positional.first().ok_or("missing command")?;
    let query = positional.get(1).map(String::as_str).unwrap_or("");
    let runtime = Runtime::open(package)?;
    if let Some(requested) = profile {
        if requested != runtime.profile() {
            return Err(format!(
                "requested profile '{requested}' does not match package profile '{}'; select the matching package explicitly",
                runtime.profile()
            )
            .into());
        }
    }
    if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
        return Err(format!("--limit must be between 1 and {MAX_SEARCH_RESULTS}").into());
    }
    let search_options = SearchOptions {
        query: if command == "search" { query.to_owned() } else { String::new() },
        kind, source, controller, plugin, owner, type_name, confidence,
        prefix: None, contexts: context.map(|value| vec![value]), kinds: None, source_kind,
        limit,
    };
    let output = match command.as_str() {
        "search" | "list" => json!({
            "profile": runtime.profile(),
            "limit": search_options.limit,
            "results": runtime.search_with_options(&search_options)
        }),
        "coverage" => json!({"profile": runtime.profile(), "coverage": runtime.coverage()}),
        "compatibility" => runtime.compatibility_report(None),
        "controller" => json!({"profile": runtime.profile(), "controller": runtime.controller_details()}),
        "plugins" => json!({"profile": runtime.profile(), "status": runtime.plugin_inventory_status(), "plugins": runtime.plugins()}),
        "sources" => json!({"profile": runtime.profile(), "sources": runtime.source_details()}),
        "steps" | "globals" | "declarative" | "types" | "library-symbols" | "plugin-symbols" => {
            let mut options = search_options.clone();
            let result_key = match command.as_str() {
                "steps" => { options.kind = Some("step".into()); "steps" }
                "globals" => { options.kind = Some("global-variable".into()); "globals" }
                "declarative" => { options.kind = Some("declarative-directive".into()); "constructs" }
                "types" => { options.kind = Some("class".into()); "types" }
                "library-symbols" => { options.source_kind = Some("shared-library".into()); "symbols" }
                _ => { options.source_kind = Some("plugin".into()); "symbols" }
            };
            let page = runtime.search_page(&options, 0)?;
            let results = if matches!(command.as_str(), "steps" | "types") {
                page.results.iter().filter_map(|symbol| symbol["id"].as_str().and_then(|id| {
                    if command == "steps" { runtime.pipeline_step(id).ok() } else { runtime.get(id).ok() }
                })).collect::<Vec<_>>()
            } else { page.results };
            let mut response = json!({"profile": runtime.profile(), "total": page.total, "limit": page.limit, "hasMore": page.has_more});
            response[result_key] = json!(results);
            response
        },
        "get" => runtime.get(query)?,
        "examples" => runtime.examples(query)?,
        "step" => runtime.pipeline_step(query)?,
        _ => {
            usage();
            return Err(format!("unknown command '{command}'").into());
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
