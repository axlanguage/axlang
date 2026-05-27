use ax_codegen_llvm::{build_program_with_backend, BackendKind};
use ax_core::SourceFile;
use ax_diag::Diagnostic;
use ax_parser::parse_source;
use ax_semantic::{check_program_with_packs, explain_program, semantic_graph, PackSpec};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

fn run() -> Result<(), u8> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        print_usage();
        return Err(2);
    }
    let command = args.remove(0);
    match command.as_str() {
        "init" => {
            let Some(name) = args.first() else {
                eprintln!("usage: ax init <name>");
                return Err(2);
            };
            ax_pkg::init_project(name).map_err(|err| {
                eprintln!("init failed: {}", err);
                1
            })?;
            println!("created {}", name);
            Ok(())
        }
        "check" => {
            let (path, json) = parse_file_and_json(&args)?;
            let registry = parse_registry_arg(&args)?;
            let packs = load_semantic_packs(&registry)?;
            let source = load_source(&path)?;
            match parse_source(&source)
                .and_then(|program| check_program_with_packs(&program, packs).map(|_| program))
            {
                Ok(_) => {
                    if !json {
                        println!("check ok: {}", path.display());
                    }
                    Ok(())
                }
                Err(diag) => fail_diag(&source, diag, json),
            }
        }
        "fmt" => {
            let Some(path) = args.first().map(PathBuf::from) else {
                eprintln!("usage: ax fmt <file.ax> [--write]");
                return Err(2);
            };
            let write = args.iter().any(|arg| arg == "--write");
            let source = load_source(&path)?;
            match parse_source(&source) {
                Ok(program) => {
                    let formatted = ax_fmt::format_program(&program);
                    if write {
                        std::fs::write(&path, formatted).map_err(|err| {
                            eprintln!("fmt failed: {}", err);
                            1
                        })?;
                    } else {
                        print!("{}", formatted);
                    }
                    Ok(())
                }
                Err(diag) => fail_diag(&source, diag, false),
            }
        }
        "build" => {
            let (input, output) = parse_build_args(&args)?;
            let backend = parse_backend_arg(&args)?;
            let registry = parse_registry_arg(&args)?;
            let packs = load_semantic_packs(&registry)?;
            let source = load_source(&input)?;
            match parse_source(&source).and_then(|program| {
                let semantic = check_program_with_packs(&program, packs)?;
                build_program_with_backend(&input, &program, &semantic, &output, backend)
            }) {
                Ok(artifact) => {
                    println!("built {}", artifact.output_path.display());
                    println!("backend {}", artifact.backend);
                    let ir_label = if artifact.backend == "custom" {
                        "asm"
                    } else {
                        "llvm"
                    };
                    println!("{} {}", ir_label, artifact.ll_path.display());
                    println!("object {}", artifact.object_path.display());
                    Ok(())
                }
                Err(diag) => fail_diag(&source, diag, false),
            }
        }
        "run" => {
            let (tool_args, program_args) = split_run_args(&args);
            let Some(input) = first_positional(tool_args).map(PathBuf::from) else {
                eprintln!("usage: ax run <file.ax> [--registry <source>] [--backend llvm|custom] [-- <args>...]");
                return Err(2);
            };
            let stem = input
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("ax-run");
            let output = PathBuf::from(".ax-out").join(format!("run-{}", stem));
            let backend = parse_backend_arg(tool_args)?;
            let registry = parse_registry_arg(tool_args)?;
            let packs = load_semantic_packs(&registry)?;
            let source = load_source(&input)?;
            let artifact = match parse_source(&source).and_then(|program| {
                let semantic = check_program_with_packs(&program, packs)?;
                build_program_with_backend(&input, &program, &semantic, &output, backend)
            }) {
                Ok(artifact) => artifact,
                Err(diag) => return fail_diag(&source, diag, false),
            };
            let status = Command::new(&artifact.output_path)
                .args(program_args)
                .status()
                .map_err(|err| {
                    eprintln!("run failed: {}", err);
                    1
                })?;
            if status.success() {
                Ok(())
            } else {
                Err(status.code().unwrap_or(1) as u8)
            }
        }
        "test" => {
            let cwd = env::current_dir().map_err(|err| {
                eprintln!("test failed: {}", err);
                1
            })?;
            let report = ax_test::run_tests(&cwd).map_err(|err| {
                eprintln!("{}", err);
                1
            })?;
            println!(
                "test result: {} passed; {} failed; {} files",
                report.passed, report.failed, report.files
            );
            if report.failed == 0 {
                Ok(())
            } else {
                Err(1)
            }
        }
        "add" => {
            let Some(pack) = first_positional(&args) else {
                eprintln!("usage: ax add <pack> [--registry <source>]");
                return Err(2);
            };
            let registry = parse_registry_arg(&args)?;
            let manifest = ax_pkg::add_pack_from_registry(pack, Path::new("ax.toml"), &registry)
                .map_err(|err| {
                    eprintln!("add failed: {}", err);
                    1
                })?;
            println!("added {} {}", manifest.name, manifest.version);
            Ok(())
        }
        "pack" => run_pack_command(&args),
        "packs" => {
            let registry = parse_registry_arg(&args)?;
            let packs = ax_pkg::list_packs(&registry).map_err(|err| {
                eprintln!("packs failed: {}", err);
                1
            })?;
            for pack in packs {
                println!("{} {}", pack.name, pack.version);
            }
            Ok(())
        }
        "graph" => {
            let Some(path) = first_positional(&args).map(PathBuf::from) else {
                eprintln!("usage: ax graph <file.ax>");
                return Err(2);
            };
            let registry = parse_registry_arg(&args)?;
            let packs = load_semantic_packs(&registry)?;
            let source = load_source(&path)?;
            match parse_source(&source).and_then(|program| {
                let semantic = check_program_with_packs(&program, packs)?;
                Ok((program, semantic))
            }) {
                Ok((program, semantic)) => {
                    print!(
                        "{}",
                        semantic_graph(&path.display().to_string(), &program, &semantic)
                    );
                    Ok(())
                }
                Err(diag) => fail_diag(&source, diag, false),
            }
        }
        "explain" => {
            let Some(path) = first_positional(&args).map(PathBuf::from) else {
                eprintln!("usage: ax explain <file.ax>");
                return Err(2);
            };
            let registry = parse_registry_arg(&args)?;
            let packs = load_semantic_packs(&registry)?;
            let source = load_source(&path)?;
            match parse_source(&source).and_then(|program| {
                let semantic = check_program_with_packs(&program, packs)?;
                Ok((program, semantic))
            }) {
                Ok((program, semantic)) => {
                    print!("{}", explain_program(&program, &semantic));
                    Ok(())
                }
                Err(diag) => fail_diag(&source, diag, false),
            }
        }
        "version" | "--version" | "-V" => {
            println!("ax 1.0.0");
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => {
            eprintln!("unknown command `{}`", command);
            print_usage();
            Err(2)
        }
    }
}

fn run_pack_command(args: &[String]) -> Result<(), u8> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_pack_usage();
        return Err(2);
    };
    match subcommand {
        "list" => {
            let registry = parse_registry_arg(args)?;
            let packs = ax_pkg::list_packs(&registry).map_err(|err| {
                eprintln!("pack list failed: {}", err);
                1
            })?;
            print_pack_list(packs);
            Ok(())
        }
        "find" | "search" => {
            let registry = parse_registry_arg(args)?;
            let query = first_positional(&args[1..]).unwrap_or_default();
            let packs = ax_pkg::list_packs(&registry).map_err(|err| {
                eprintln!("pack find failed: {}", err);
                1
            })?;
            let matches = packs
                .into_iter()
                .filter(|pack| pack_matches_query(pack, query))
                .collect::<Vec<_>>();
            if matches.is_empty() {
                eprintln!("no packs matched `{}`", query);
                return Err(1);
            }
            print_pack_list(matches);
            Ok(())
        }
        "info" => {
            let Some(pack) = first_positional(&args[1..]) else {
                eprintln!("usage: ax pack info <pack> [--registry <source>]");
                return Err(2);
            };
            let registry = parse_registry_arg(args)?;
            let manifest = ax_pkg::resolve_pack(pack, &registry).map_err(|err| {
                eprintln!("pack info failed: {}", err);
                1
            })?;
            print_pack_info(&manifest);
            Ok(())
        }
        "install" | "add" => {
            let Some(pack) = first_positional(&args[1..]) else {
                eprintln!("usage: ax pack install <pack> [--registry <source>]");
                return Err(2);
            };
            let registry = parse_registry_arg(args)?;
            let manifest = ax_pkg::add_pack_from_registry(pack, Path::new("ax.toml"), &registry)
                .map_err(|err| {
                    eprintln!("pack install failed: {}", err);
                    1
                })?;
            println!("added {} {}", manifest.name, manifest.version);
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_pack_usage();
            Ok(())
        }
        _ => {
            eprintln!("unknown pack command `{}`", subcommand);
            print_pack_usage();
            Err(2)
        }
    }
}

fn print_pack_list(packs: Vec<ax_pkg::PackManifest>) {
    for pack in packs {
        println!("{} {}", pack.name, pack.version);
    }
}

fn print_pack_info(pack: &ax_pkg::PackManifest) {
    println!("name {}", pack.name);
    println!("version {}", pack.version);
    println!("source {}", pack.source);
    println!("syntax {}", join_or_dash(&pack.syntax));
    println!("operations {}", join_or_dash(&pack.operations));
    println!("effects {}", join_or_dash(&pack.effects));
    println!("native {}", join_or_dash(&pack.native));
}

fn pack_matches_query(pack: &ax_pkg::PackManifest, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_ascii_lowercase();
    pack.name.to_ascii_lowercase().contains(&query)
        || pack
            .syntax
            .iter()
            .any(|item| item.to_ascii_lowercase().contains(&query))
        || pack
            .operations
            .iter()
            .any(|item| item.to_ascii_lowercase().contains(&query))
        || pack
            .effects
            .iter()
            .any(|item| item.to_ascii_lowercase().contains(&query))
        || pack
            .native
            .iter()
            .any(|item| item.to_ascii_lowercase().contains(&query))
}

fn join_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

fn parse_file_and_json(args: &[String]) -> Result<(PathBuf, bool), u8> {
    let json = args.iter().any(|arg| arg == "--json");
    let path = first_positional(args).map(PathBuf::from);
    let Some(path) = path else {
        eprintln!("usage: ax check <file.ax> [--json] [--registry <source>]");
        return Err(2);
    };
    Ok((path, json))
}

fn parse_build_args(args: &[String]) -> Result<(PathBuf, PathBuf), u8> {
    if args.is_empty() {
        eprintln!("usage: ax build <file.ax> -o <output> [--backend llvm|custom]");
        return Err(2);
    }
    let input = PathBuf::from(&args[0]);
    let mut output = None;
    let mut idx = 1;
    while idx < args.len() {
        if args[idx] == "-o" || args[idx] == "--output" {
            output = args.get(idx + 1).map(PathBuf::from);
            idx += 2;
        } else {
            idx += 1;
        }
    }
    let Some(output) = output else {
        eprintln!("usage: ax build <file.ax> -o <output> [--backend llvm|custom]");
        return Err(2);
    };
    Ok((input, output))
}

fn parse_registry_arg(args: &[String]) -> Result<ax_pkg::RegistrySource, u8> {
    let explicit = args.windows(2).find_map(|window| {
        if window[0] == "--registry" {
            Some(window[1].as_str())
        } else {
            None
        }
    });
    if args.iter().any(|arg| arg == "--registry") && explicit.is_none() {
        eprintln!("--registry requires a value");
        return Err(2);
    }
    match explicit {
        Some(value) => ax_pkg::registry_from_value(Some(value)),
        None => ax_pkg::registry_from_env_or_default(),
    }
    .map_err(|err| {
        eprintln!("registry failed: {}", err);
        1
    })
}

fn parse_backend_arg(args: &[String]) -> Result<BackendKind, u8> {
    let explicit = args.windows(2).find_map(|window| {
        if window[0] == "--backend" {
            Some(window[1].as_str())
        } else {
            None
        }
    });
    if args.iter().any(|arg| arg == "--backend") && explicit.is_none() {
        eprintln!("--backend requires llvm or custom");
        return Err(2);
    }
    match explicit.unwrap_or("llvm") {
        "llvm" => Ok(BackendKind::Llvm),
        "custom" => Ok(BackendKind::Custom),
        value => {
            eprintln!("unknown backend `{}`; expected llvm or custom", value);
            Err(2)
        }
    }
}

fn load_semantic_packs(registry: &ax_pkg::RegistrySource) -> Result<Vec<PackSpec>, u8> {
    let dependencies = ax_pkg::read_project_dependencies(Path::new("ax.toml")).map_err(|err| {
        eprintln!("dependency load failed: {}", err);
        1
    })?;
    let mut packs = Vec::new();
    for dependency in dependencies {
        let source = if dependency.starts_with("std.") {
            ax_pkg::RegistrySource::Builtin
        } else {
            registry.clone()
        };
        let manifest = ax_pkg::resolve_pack(&dependency, &source).map_err(|err| {
            eprintln!("dependency resolve failed: {}", err);
            1
        })?;
        let native_sources = manifest
            .resolve_native_sources()
            .map_err(|err| {
                eprintln!("dependency native source load failed: {}", err);
                1
            })?
            .into_iter()
            .map(|path| path.display().to_string())
            .collect();
        packs.push(PackSpec {
            name: manifest.name,
            syntax: manifest.syntax,
            operations: manifest.operations,
            effects: manifest.effects,
            native_sources,
        });
    }
    Ok(packs)
}

fn first_positional(args: &[String]) -> Option<&str> {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--registry" || arg == "--backend" {
            skip_next = true;
            continue;
        }
        if !arg.starts_with('-') {
            return Some(arg);
        }
    }
    None
}

fn split_run_args(args: &[String]) -> (&[String], &[String]) {
    if let Some(idx) = args.iter().position(|arg| arg == "--") {
        (&args[..idx], &args[idx + 1..])
    } else {
        (args, &[])
    }
}

fn load_source(path: &Path) -> Result<SourceFile, u8> {
    SourceFile::from_path(path).map_err(|err| {
        eprintln!("failed to read {}: {}", path.display(), err);
        1
    })
}

fn fail_diag(source: &SourceFile, diag: Diagnostic, json: bool) -> Result<(), u8> {
    if json {
        eprintln!("{}", diag.to_json(source));
    } else {
        eprintln!("{}", diag.render(source));
    }
    Err(1)
}

fn print_usage() {
    eprintln!(
        "Ax v1.0\n\ncommands:\n  ax init <name>\n  ax check <file.ax> [--json] [--registry <source>]\n  ax fmt <file.ax> [--write]\n  ax build <file.ax> -o <output> [--registry <source>] [--backend llvm|custom]\n  ax run <file.ax> [--registry <source>] [--backend llvm|custom] [-- <args>...]\n  ax test\n  ax pack list [--registry <source>]\n  ax pack find [query] [--registry <source>]\n  ax pack info <pack> [--registry <source>]\n  ax pack install <pack> [--registry <source>]\n  ax add <pack> [--registry <source>]\n  ax packs [--registry <source>]\n  ax graph <file.ax> [--registry <source>]\n  ax explain <file.ax> [--registry <source>]\n  ax version"
    );
}

fn print_pack_usage() {
    eprintln!(
        "usage:\n  ax pack list [--registry <source>]\n  ax pack find [query] [--registry <source>]\n  ax pack info <pack> [--registry <source>]\n  ax pack install <pack> [--registry <source>]"
    );
}
