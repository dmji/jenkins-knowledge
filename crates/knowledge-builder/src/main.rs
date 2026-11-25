use knowledge_builder::build_package;
use std::{env, path::Path, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 3 || args[0] != "build" {
        eprintln!("usage: knowledge-builder build <profile.yml> <output-package-directory>");
        return ExitCode::from(2);
    }
    match build_package(Path::new(&args[1]), Path::new(&args[2])) {
        Ok(()) => { println!("package created: {}", args[2]); ExitCode::SUCCESS }
        Err(error) => { eprintln!("build failed: {error}"); ExitCode::FAILURE }
    }
}
