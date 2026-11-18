#![forbid(unsafe_code)]
mod project;
mod web;
use esh_host::{parse_rights, Grant, Host, HostError};
use esh_lang::{
    compiler::{compile, Program},
    contracts::add_initial_state,
};
use esh_runtime::{
    update::{CheckLimits, UpdateHost},
    Limits, Runtime,
};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Options {
    command: String,
    file: PathBuf,
    project_root: Option<PathBuf>,
    state: Option<PathBuf>,
    grants: Vec<Grant>,
    max_bytes: usize,
    checks: CheckLimits,
    max_steps: u64,
    port: u16,
}
fn arguments() -> Result<Option<Options>, HostError> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        return Ok(None);
    };
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        return Ok(None);
    }
    if !matches!(command.as_str(), "check" | "run" | "serve") {
        return Err(HostError::new("usage", "Expected check, run, or serve."));
    }
    let mut options = Options {
        command,
        file: PathBuf::new(),
        project_root: None,
        state: None,
        grants: vec![],
        max_bytes: 8 * 1024 * 1024,
        checks: CheckLimits::default(),
        max_steps: 1_000_000,
        port: 49331,
    };
    while let Some(arg) = args.next() {
        let mut next = || {
            args.next()
                .ok_or_else(|| HostError::new("usage", "An option is missing its value."))
        };
        match arg.as_str() {
            "--port" => {
                options.port = next()?
                    .parse()
                    .map_err(|_| HostError::new("usage", "Expected a valid TCP port."))?
            }
            "--project-root" => options.project_root = Some(next()?.into()),
            "--state" => options.state = Some(next()?.into()),
            "--grant" => {
                let name = next()?;
                let rights = next()?;
                let directory = next()?.into();
                options.grants.push(Grant {
                    name,
                    rights: parse_rights(rights.split(','))?,
                    directory,
                });
            }
            "--max-bytes" => options.max_bytes = positive(&next()?)?,
            "--max-checks" => options.checks.max_observations = positive(&next()?)?,
            "--max-check-bytes" => options.checks.max_content_bytes = positive(&next()?)?,
            "--max-steps" => options.max_steps = positive(&next()?)? as u64,
            _ if !arg.starts_with('-') && options.file.as_os_str().is_empty() => {
                options.file = arg.into()
            }
            _ => {
                return Err(HostError::new(
                    "usage",
                    "Unknown option or more than one entry file.",
                ))
            }
        }
    }
    if options.file.as_os_str().is_empty() {
        return Err(HostError::new("usage", "Specify an entry file."));
    }
    Ok(Some(options))
}
fn positive(s: &str) -> Result<usize, HostError> {
    s.parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| HostError::new("usage", "Limits must be positive integers."))
}
fn failure(e: &HostError) -> Value {
    json!({"ok":false,"error":e})
}
fn execute(options: Options) -> (u8, Value) {
    let sources = match project::load(&options.file, options.project_root.as_deref(), None) {
        Ok(s) => s,
        Err(e) => return (2, failure(&e)),
    };
    let state = match options
        .state
        .as_ref()
        .map(|path| project::read_explicit(path, esh_syntax::MAX_SOURCE_BYTES))
        .transpose()
    {
        Ok(state) => state,
        Err(e) => return (2, failure(&e)),
    };
    if options.command == "serve" {
        return match web::serve(sources, state, options.port) {
            Ok(()) => (0, json!({"ok":true})),
            Err(e) => (2, failure(&e)),
        };
    }
    let mut p = match compile(sources.clone()) {
        Ok(p) => p,
        Err(e) => {
            return (
                2,
                json!({"ok":false,"diagnostic":{"code":e.code,"message":e.message,"span":e.span,"source":sources[e.source].name},"sources":sources}),
            )
        }
    };
    if let Some(state) = state {
        if let Err(e) = add_initial_state(&mut p, &state) {
            return (
                2,
                json!({"ok":false,"diagnostic":{"code":e.code,"message":e.message,"span":e.span,"source":"supplied state"}}),
            );
        }
    }
    p.report["fileImpact"] = json!(esh_lang::review::file_impacts(&p));
    if options.command == "check" {
        return (0, p.report);
    }
    run(p, &options)
}
fn run(p: Program, options: &Options) -> (u8, Value) {
    let host = match Host::new(&options.grants, options.max_bytes) {
        Ok(h) => h,
        Err(e) => return (2, failure(&e)),
    };
    let mut host = UpdateHost::new(host, vec![], options.checks);
    let mut runtime = Runtime::new(
        &mut host,
        Limits {
            max_steps: options.max_steps,
            max_value_bytes: options.max_bytes,
            ..Limits::default()
        },
    );
    let value = runtime.run(&p);
    let steps = runtime.steps;
    let trace = std::mem::take(&mut runtime.trace);
    drop(runtime);
    let (code, result) = match value {
        Ok(v) => (u8::from(v.failed()), v.to_json()),
        Err(e) => (1, json!({"error":e})),
    };
    (
        code,
        json!({"ok":code==0,"kind":"result","type":p.report["type"],"value":result,"steps":steps,"trace":trace,"update":host.report(),"assurance":p.report["assurance"]}),
    )
}
fn main() {
    if std::env::args().nth(1).as_deref() == Some("build-web") {
        if let Err(e) = web::build() {
            eprintln!("{e}");
            std::process::exit(2);
        }
        return;
    }
    let result = match arguments() {
        Ok(None) => {
            println!("esh — typed filesystem scripts\n\n  esh check FILE [--project-root DIR] [--state FILE]\n  esh run FILE --grant NAME read,write,mkdir,remove DIR\n\nEditor: esh build-web, then esh serve FILE [--state FILE] [--port N]\n\nLimits: --max-bytes N --max-steps N --max-checks N --max-check-bytes N\n\nChecks do not execute scripts. Runs validate mandatory conditions before changes.\nResults and partial-change reports are JSON. See docs/assurance.md for proof scope.");
            return;
        }
        Ok(Some(options)) => execute(options),
        Err(e) => (2, failure(&e)),
    };
    println!("{}", serde_json::to_string_pretty(&result.1).unwrap());
    std::process::exit(result.0 as i32);
}
