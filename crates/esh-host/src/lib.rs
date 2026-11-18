//! POSIX request adapter for the checked esh kernel.
//!
//! OS calls, descriptor interpretation, and the libraries implementing those
//! calls are an explicit trust boundary. This is not a verified OS sandbox.
#![forbid(unsafe_code)]
#![cfg(unix)]

use esh_policy::{allows, covers, permission_bit, valid_relative_path, valid_rights, Permission};
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags};
use rustix::io::Errno;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Grant {
    pub name: String,
    pub rights: u8,
    pub directory: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapabilityDeclaration {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Read,
    Write,
    #[serde(rename = "read-bytes")]
    ReadBytes,
    #[serde(rename = "write-bytes")]
    WriteBytes,
    List,
    Mkdir,
    Remove,
}

impl Operation {
    pub fn permission(self) -> Permission {
        match self {
            Self::Read | Self::ReadBytes | Self::List => Permission::Read,
            Self::Write | Self::WriteBytes => Permission::Write,
            Self::Mkdir => Permission::Mkdir,
            Self::Remove => Permission::Remove,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub operation: Operation,
    pub region: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HostError {
    pub code: String,
    pub message: String,
}

impl HostError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HostError {}

impl From<Errno> for HostError {
    fn from(error: Errno) -> Self {
        let code = match error {
            Errno::NOENT => "ENOENT",
            Errno::ACCESS => "EACCES",
            Errno::PERM => "EPERM",
            Errno::LOOP => "ELOOP",
            Errno::NOTDIR => "ENOTDIR",
            Errno::ISDIR => "EISDIR",
            Errno::EXIST => "EEXIST",
            Errno::NOSPC => "ENOSPC",
            Errno::INVAL => "EINVAL",
            Errno::IO => "EIO",
            _ => "io",
        };
        // Errno's message contains no configured absolute root or script path.
        Self::new(code, &error.to_string())
    }
}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        match error.raw_os_error() {
            Some(errno) => Errno::from_raw_os_error(errno).into(),
            None => Self::new("io", "Filesystem operation failed."),
        }
    }
}

pub fn parse_rights<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<u8, HostError> {
    let mut mask = 0;
    for name in names {
        let permission = match name {
            "read" => Permission::Read,
            "write" => Permission::Write,
            "mkdir" => Permission::Mkdir,
            "remove" => Permission::Remove,
            _ => return Err(HostError::new("grant", "Unknown permission in grant.")),
        };
        mask |= permission_bit(permission);
    }
    Ok(mask)
}

pub struct Host {
    roots: BTreeMap<String, (OwnedFd, u8)>,
    active: BTreeMap<String, u8>,
    max_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}
#[allow(clippy::unnecessary_cast)] // Device and inode widths vary across supported POSIX targets.
fn identity(info: &fs::Stat) -> FileIdentity {
    FileIdentity {
        device: info.st_dev as u64,
        inode: info.st_ino as u64,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    RegularFile,
    Directory,
    Absent,
    Other,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub anchor: FileIdentity,
    pub suffix: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Observation {
    pub kind: EntryKind,
    pub identity: Option<FileIdentity>,
    pub address: Address,
    pub ancestors: Vec<FileIdentity>,
    pub bytes: Option<Vec<u8>>,
}

impl Host {
    pub fn new(grants: &[Grant], max_bytes: usize) -> Result<Self, HostError> {
        if max_bytes == 0 || max_bytes.checked_add(1).is_none() {
            return Err(HostError::new(
                "limit",
                "max-bytes must be positive and leave room for a limit check.",
            ));
        }
        let mut roots = BTreeMap::new();
        for grant in grants {
            if grant.name.is_empty() || grant.name == "_" || roots.contains_key(&grant.name) {
                return Err(HostError::new(
                    "grant",
                    "Capability names must be nonempty and unique.",
                ));
            }
            if !valid_rights(grant.rights) {
                return Err(HostError::new("grant", "Unknown permission in grant."));
            }
            // Root selection is explicit trusted configuration, not a script path.
            let root = fs::open(
                &grant.directory,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            roots.insert(grant.name.clone(), (root, grant.rights));
        }
        Ok(Self {
            roots,
            active: BTreeMap::new(),
            max_bytes,
        })
    }

    /// Establish a subset of the supplied grants only after the kernel is ready.
    pub fn authorize(&mut self, declarations: &[CapabilityDeclaration]) -> Result<(), HostError> {
        let mut active = BTreeMap::new();
        for declaration in declarations {
            let required = parse_rights(declaration.permissions.iter().map(String::as_str))?;
            let (_, granted) = self
                .roots
                .get(&declaration.name)
                .ok_or_else(|| HostError::new("grant", "Missing capability grant."))?;
            if !covers(*granted, required) || active.contains_key(&declaration.name) {
                return Err(HostError::new(
                    "grant",
                    "Invalid or insufficient capability grant.",
                ));
            }
            active.insert(declaration.name.clone(), required);
        }
        self.active = active;
        Ok(())
    }

    fn directory(root: &OwnedFd, parts: &[&str]) -> Result<OwnedFd, HostError> {
        let mut fd = root.try_clone()?;
        for component in parts {
            fd = fs::openat(
                &fd,
                *component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
        }
        Ok(fd)
    }

    /// Include ancestors above the capability root: a different grant may start
    /// several levels inside a protected directory. These identifiers are host
    /// bookkeeping, never script-visible directory contents or extra authority.
    /// Resolve them afresh so an opened root moved since preflight is checked in
    /// its current namespace. Unknown ancestry prevents mutation admission.
    fn directory_ancestors(directory: &OwnedFd) -> Result<Vec<FileIdentity>, HostError> {
        let mut current = directory.try_clone()?;
        let mut current_id = identity(&fs::fstat(&current)?);
        let mut ancestors = vec![current_id];
        for _ in 0..1024 {
            let parent = fs::openat(
                &current,
                "..",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            let parent_id = identity(&fs::fstat(&parent)?);
            if parent_id == current_id {
                ancestors.reverse();
                return Ok(ancestors);
            }
            if ancestors.contains(&parent_id) {
                return Err(HostError::new(
                    "check-incomplete",
                    "Directory ancestry is cyclic or changed during observation.",
                ));
            }
            ancestors.push(parent_id);
            current = parent;
            current_id = parent_id;
        }
        Err(HostError::new(
            "check-incomplete",
            "Directory ancestry exceeds the observation limit.",
        ))
    }

    /// Bounded, descriptor-relative observation used by mandatory contracts.
    /// Metadata and contents describe this observation only, not a snapshot.
    pub fn observe(
        &self,
        region: &str,
        path: &str,
        contents: bool,
    ) -> Result<Observation, HostError> {
        self.locate(region, path, contents, Permission::Read)
    }

    pub fn mutation_target(&self, request: &Request) -> Result<Observation, HostError> {
        self.locate(
            &request.region,
            &request.path,
            false,
            request.operation.permission(),
        )
    }

    fn locate(
        &self,
        region: &str,
        path: &str,
        contents: bool,
        permission: Permission,
    ) -> Result<Observation, HostError> {
        if !allows(self.active.get(region).copied().unwrap_or(0), permission) {
            return Err(HostError::new(
                "permission",
                "Observation is outside the active grants.",
            ));
        }
        if !valid_relative_path(path.as_bytes()) {
            return Err(HostError::new("path", "Invalid relative path."));
        }
        let (root, _) = self
            .roots
            .get(region)
            .ok_or_else(|| HostError::new("permission", "Unknown root."))?;
        let mut directory = root.try_clone()?;
        let root_id = identity(&fs::fstat(&directory)?);
        let mut ancestors = Self::directory_ancestors(&directory)?;
        if path == "." {
            return Ok(Observation {
                kind: EntryKind::Directory,
                identity: Some(root_id),
                address: Address {
                    anchor: root_id,
                    suffix: vec![],
                },
                ancestors,
                bytes: None,
            });
        }
        let parts: Vec<&str> = path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let address = Address {
                anchor: *ancestors.last().unwrap(),
                suffix: parts[i..].iter().map(|s| (*s).to_owned()).collect(),
            };
            if i + 1 < parts.len() {
                match fs::openat(
                    &directory,
                    *part,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                ) {
                    Ok(child) => {
                        ancestors.push(identity(&fs::fstat(&child)?));
                        directory = child;
                    }
                    Err(Errno::NOENT) => {
                        return Ok(Observation {
                            kind: EntryKind::Absent,
                            identity: None,
                            address,
                            ancestors,
                            bytes: None,
                        })
                    }
                    Err(e) => return Err(e.into()),
                }
            } else {
                let info = match fs::statat(&directory, *part, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(info) => info,
                    Err(Errno::NOENT) => {
                        return Ok(Observation {
                            kind: EntryKind::Absent,
                            identity: None,
                            address,
                            ancestors,
                            bytes: None,
                        })
                    }
                    Err(e) => return Err(e.into()),
                };
                let kind = match FileType::from_raw_mode(info.st_mode) {
                    FileType::RegularFile => EntryKind::RegularFile,
                    FileType::Directory => EntryKind::Directory,
                    _ => EntryKind::Other,
                };
                let mut result = Observation {
                    kind,
                    identity: Some(identity(&info)),
                    address,
                    ancestors: ancestors.clone(),
                    bytes: None,
                };
                if contents && kind == EntryKind::RegularFile {
                    let fd = fs::openat(
                        &directory,
                        *part,
                        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                        Mode::empty(),
                    )?;
                    let opened = fs::fstat(&fd)?;
                    if FileType::from_raw_mode(opened.st_mode) != FileType::RegularFile
                        || identity(&opened) != identity(&info)
                    {
                        return Err(HostError::new(
                            "drift",
                            "Target changed during observation.",
                        ));
                    }
                    let mut bytes = Vec::new();
                    File::from(fd)
                        .take((self.max_bytes + 1) as u64)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() > self.max_bytes {
                        return Err(HostError::new(
                            "resource-limit",
                            "Mandatory content observation exceeds max-bytes.",
                        ));
                    }
                    result.bytes = Some(bytes);
                }
                return Ok(result);
            }
        }
        unreachable!("validated path has components")
    }

    pub fn dispatch(&self, request: &Request) -> Value {
        match self.perform(request) {
            Ok(value) => json!({"ok": value}),
            Err(error) => json!({"error": error}),
        }
    }

    pub fn perform(&self, request: &Request) -> Result<Value, HostError> {
        let rights = self.active.get(&request.region).copied().unwrap_or(0);
        if !allows(rights, request.operation.permission()) {
            return Err(HostError::new(
                "permission",
                "Request is outside the active grants.",
            ));
        }
        if !valid_relative_path(request.path.as_bytes()) {
            return Err(HostError::new("path", "Invalid relative path."));
        }
        let (root, _) = self
            .roots
            .get(&request.region)
            .ok_or_else(|| HostError::new("permission", "Unknown capability root."))?;
        let parts: Vec<&str> = if request.path == "." {
            vec![]
        } else {
            request.path.split('/').collect()
        };
        if matches!(request.operation, Operation::List) {
            let fd = Self::directory(root, &parts)?;
            let mut names = Vec::new();
            let mut size = 0usize;
            for entry in Dir::new(fd)? {
                let entry = entry?;
                let bytes = entry.file_name().to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                let name = std::str::from_utf8(bytes)
                    .map_err(|_| HostError::new("encoding", "Expected valid UTF-8."))?;
                size = size.checked_add(bytes.len()).ok_or_else(|| {
                    HostError::new("resource-limit", "Directory listing exceeds max-bytes.")
                })?;
                if size > self.max_bytes {
                    return Err(HostError::new(
                        "resource-limit",
                        "Directory listing exceeds max-bytes.",
                    ));
                }
                names.push(name.to_owned());
            }
            names.sort();
            return Ok(json!(names));
        }
        let (name, parents) = parts
            .split_last()
            .ok_or_else(|| HostError::new("path", "This operation requires a non-root path."))?;
        let directory = Self::directory(root, parents)?;
        match request.operation {
            Operation::Read | Operation::ReadBytes => {
                let fd = fs::openat(
                    &directory,
                    *name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )?;
                if FileType::from_raw_mode(fs::fstat(&fd)?.st_mode) != FileType::RegularFile {
                    return Err(HostError::new("file-kind", "Expected a regular file."));
                }
                let mut bytes = Vec::new();
                File::from(fd)
                    .take((self.max_bytes + 1) as u64)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > self.max_bytes {
                    return Err(HostError::new("resource-limit", "File exceeds max-bytes."));
                }
                if matches!(request.operation, Operation::ReadBytes) {
                    return Ok(json!(bytes));
                }
                let text = String::from_utf8(bytes)
                    .map_err(|_| HostError::new("encoding", "Expected valid UTF-8."))?;
                Ok(json!(text))
            }
            Operation::Write => {
                let text = request
                    .contents
                    .as_deref()
                    .ok_or_else(|| HostError::new("protocol", "Write requires text contents."))?;
                self.write(&directory, name, text.as_bytes())?;
                Ok(Value::Null)
            }
            Operation::WriteBytes => {
                let bytes = request
                    .bytes
                    .as_deref()
                    .ok_or_else(|| HostError::new("protocol", "Write requires byte contents."))?;
                self.write(&directory, name, bytes)?;
                Ok(Value::Null)
            }
            Operation::Mkdir => {
                fs::mkdirat(&directory, *name, Mode::from_bits_truncate(0o700))?;
                Ok(Value::Null)
            }
            Operation::Remove => {
                let info = fs::statat(&directory, *name, AtFlags::SYMLINK_NOFOLLOW)?;
                if FileType::from_raw_mode(info.st_mode) != FileType::RegularFile {
                    return Err(HostError::new(
                        "file-kind",
                        "remove accepts regular files only.",
                    ));
                }
                fs::unlinkat(&directory, *name, AtFlags::empty())?;
                Ok(Value::Null)
            }
            Operation::List => unreachable!("listing returned before file dispatch"),
        }
    }

    fn write(&self, directory: &OwnedFd, name: &str, bytes: &[u8]) -> Result<(), HostError> {
        if bytes.len() > self.max_bytes {
            return Err(HostError::new("resource-limit", "Write exceeds max-bytes."));
        }
        let mode = match fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(info) if FileType::from_raw_mode(info.st_mode) == FileType::RegularFile => {
                Mode::from_bits_truncate(info.st_mode & 0o777)
            }
            Ok(_) => {
                return Err(HostError::new(
                    "file-kind",
                    "Expected a regular file or a new path.",
                ))
            }
            Err(Errno::NOENT) => Mode::from_bits_truncate(0o600),
            Err(error) => return Err(error.into()),
        };
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| HostError::new("io", "Could not create a temporary-file name."))?;
        let temporary = format!(
            ".esh-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let fd = fs::openat(
            directory,
            temporary.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )?;
        let result = (|| -> Result<(), HostError> {
            let mut file = File::from(fd);
            file.write_all(bytes)?;
            fs::fchmod(&file, mode)?;
            file.sync_all()?;
            fs::renameat(directory, temporary.as_str(), directory, name)?;
            Ok(())
        })();
        // A successful replacement consumes the temporary path. Other cleanup
        // failures are reported; they do not imply that earlier effects rolled back.
        if result.is_ok() {
            return result;
        }
        match fs::unlinkat(directory, temporary.as_str(), AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => result,
            Err(error) => Err(error.into()),
        }
    }
}
