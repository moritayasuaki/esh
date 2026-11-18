use esh_host::{CapabilityDeclaration, Grant, Host, HostError, Operation, Request};
use esh_lang::compiler::{resolve_import, Source};
use esh_syntax::{parse, MAX_SOURCE_BYTES};
use std::{collections::BTreeSet, io::Read, path::Path};

pub fn read_explicit(path: &Path, limit: usize) -> Result<String, HostError> {
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| HostError::new("source", "Cannot open the source as a regular file."))?;
    let file = std::fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(HostError::new("source", "Expected a regular source file."));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(HostError::new(
            "source-limit",
            "Source exceeds its size limit.",
        ));
    }
    String::from_utf8(bytes).map_err(|_| HostError::new("encoding", "Source must be UTF-8."))
}
pub fn load(
    file: &Path,
    project_root: Option<&Path>,
    override_text: Option<&str>,
) -> Result<Vec<Source>, HostError> {
    let file = file
        .canonicalize()
        .map_err(|_| HostError::new("source", "Cannot resolve the entry file."))?;
    let root = project_root
        .unwrap_or_else(|| file.parent().unwrap())
        .canonicalize()
        .map_err(|_| HostError::new("project-root", "Cannot resolve the project root."))?;
    let name = file
        .strip_prefix(&root)
        .map_err(|_| {
            HostError::new(
                "project-root",
                "Entry is outside the designated project root.",
            )
        })?
        .to_str()
        .ok_or_else(|| HostError::new("encoding", "Source names must be UTF-8."))?
        .to_owned();
    let mut host = Host::new(
        &[Grant {
            name: "source".into(),
            rights: 1,
            directory: root,
        }],
        MAX_SOURCE_BYTES,
    )?;
    host.authorize(&[CapabilityDeclaration {
        name: "source".into(),
        permissions: vec!["read".into()],
    }])?;
    let text = if let Some(t) = override_text {
        t.into()
    } else {
        read(&host, &name)?
    };
    let mut sources = vec![Source {
        name: name.clone(),
        text,
    }];
    let mut seen: BTreeSet<String> = [name].into();
    let mut cursor = 0;
    while cursor < sources.len() {
        if sources.len() > 128
            || sources.iter().map(|s| s.text.len()).sum::<usize>() > 4 * 1024 * 1024
        {
            return Err(HostError::new(
                "source-limit",
                "Module graph exceeds 128 files or 4 MiB.",
            ));
        }
        // Preserve parser diagnostics for the compiler; malformed files have no usable imports.
        if let Ok(f) = parse(&sources[cursor].text, cursor) {
            if let Some(xs) = f.list() {
                for declaration in xs {
                    if !declaration.has_head("import") {
                        continue;
                    }
                    let xs = declaration.list().unwrap();
                    if xs.len() != 3 {
                        continue;
                    }
                    let Some(path) = xs[2].text() else { continue };
                    let target = resolve_import(&sources[cursor].name, path).ok_or_else(|| {
                        HostError::new(
                            "project-root",
                            "Import escapes the designated project root.",
                        )
                    })?;
                    if seen.insert(target.clone()) {
                        let text = read(&host, &target)?;
                        sources.push(Source { name: target, text });
                    }
                }
            }
        }
        cursor += 1;
    }
    Ok(sources)
}
fn read(host: &Host, path: &str) -> Result<String, HostError> {
    host.perform(&Request {
        operation: Operation::Read,
        region: "source".into(),
        path: path.into(),
        contents: None,
        bytes: None,
    })?
    .as_str()
    .map(str::to_owned)
    .ok_or_else(|| HostError::new("source", "Expected UTF-8 source text."))
}
