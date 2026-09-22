use crate::discovery::path_reachable;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct CreateRequest {
    pub root: PathBuf,
    pub target: PathBuf,
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    password: Option<String>,
}

impl CreateRequest {
    pub fn new(
        root: PathBuf,
        target: PathBuf,
        alias: String,
        hostname: String,
        user: Option<String>,
        port: Option<u16>,
        password: Option<String>,
    ) -> Self {
        Self {
            root,
            target,
            alias,
            hostname,
            user,
            port,
            password,
        }
    }
}

#[derive(Debug)]
pub struct CreatePlan {
    pub id: String,
    pub files: Vec<FileChange>,
    lock_path: PathBuf,
    writes: Vec<WriteFile>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOperation {
    Create,
    Modify,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub operation: FileOperation,
    pub patch: String,
}

#[derive(Clone, Debug)]
struct WriteFile {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
    mode: u32,
    existed: bool,
    snapshot: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Fingerprint {
    exists: bool,
    length: u64,
    digest: u64,
    mode: u32,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    modified: Option<(u64, u32)>,
}

struct WriterLock {
    path: PathBuf,
    _file: File,
}

impl WriterLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "MUTATION_BUSY: another host mutation owns {}",
                    path.display()
                )
            } else {
                format!(
                    "MUTATION_LOCK_FAILED: cannot lock {}: {error}",
                    path.display()
                )
            }
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            _file: file,
        })
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn plan_create(request: &CreateRequest) -> Result<CreatePlan, String> {
    validate_request(request)?;
    let root = absolute_path(&request.root)?;
    let target = absolute_path(&request.target)?;
    let root_bytes = read_regular_file(&root, "CONFIG_ROOT")?;
    let target_bytes = if target == root {
        root_bytes.clone()
    } else {
        read_optional_regular_file(&target, "TARGET_FILE")?.unwrap_or_default()
    };
    let root_snapshot = fingerprint(&root)?;
    let target_snapshot = if target == root {
        root_snapshot.clone()
    } else {
        fingerprint(&target)?
    };
    ensure_snapshot_bytes(&root_snapshot, &root_bytes, &root)?;
    if target != root {
        ensure_snapshot_bytes(&target_snapshot, &target_bytes, &target)?;
    }
    let target_exists = target_snapshot.exists;
    let target_mode = if target_exists {
        target_snapshot.mode
    } else {
        0o600
    };
    let target_eol = detect_line_ending(&target_bytes)
        .or_else(|| detect_line_ending(&root_bytes))
        .unwrap_or_else(|| "\n".to_string());
    let root_eol = detect_line_ending(&root_bytes).unwrap_or_else(|| target_eol.clone());
    let id = new_id(&target_bytes, &root_bytes);
    let block = host_block(request, &id, &target_eol);
    let target_after = append_bytes(&target_bytes, block.as_bytes(), target_eol.as_bytes());
    let mut writes = vec![WriteFile {
        path: target.clone(),
        before: target_bytes.clone(),
        after: target_after.clone(),
        mode: target_mode,
        existed: target_exists,
        snapshot: target_snapshot,
    }];
    let mut files = vec![FileChange {
        path: display_path(&target),
        operation: if target_exists {
            FileOperation::Modify
        } else {
            FileOperation::Create
        },
        patch: append_patch(
            &target,
            &target_bytes,
            &target_after,
            request.password.as_deref(),
        ),
    }];

    let same_file = target == root
        || (target.exists()
            && root.exists()
            && fs::canonicalize(&target).ok() == fs::canonicalize(&root).ok());
    if !same_file && !path_reachable(&root, &target).map_err(|error| error.to_string())? {
        let include = include_path(&target);
        let include_line = format!("Include {include}");
        let root_after = append_bytes(
            &root_bytes,
            format!("{include_line}{root_eol}").as_bytes(),
            root_eol.as_bytes(),
        );
        writes.push(WriteFile {
            path: root.clone(),
            before: root_bytes.clone(),
            after: root_after.clone(),
            mode: root_snapshot.mode,
            existed: true,
            snapshot: root_snapshot,
        });
        files.push(FileChange {
            path: display_path(&root),
            operation: FileOperation::Modify,
            patch: append_patch(&root, &root_bytes, &root_after, request.password.as_deref()),
        });
    }

    Ok(CreatePlan {
        id,
        files,
        lock_path: mutation_lock_path(&root),
        writes,
    })
}

pub fn apply(plan: &CreatePlan) -> Result<(), String> {
    let _lock = WriterLock::acquire(&plan.lock_path)?;
    for write in &plan.writes {
        verify_snapshot(write)?;
    }
    let mut applied = Vec::new();
    for write in &plan.writes {
        if let Err(error) = write_atomic(write) {
            for previous in applied.into_iter().rev() {
                let _ = restore(previous);
            }
            return Err(error);
        }
        applied.push(write);
    }
    Ok(())
}

fn validate_request(request: &CreateRequest) -> Result<(), String> {
    validate_value("alias", &request.alias)?;
    if request.alias.chars().any(|character| {
        character.is_whitespace() || matches!(character, '*' | '?' | '[' | ']' | '!')
    }) {
        return Err("ALIAS_INVALID: alias must be one exact Host token".to_string());
    }
    validate_value("hostname", &request.hostname)?;
    if let Some(user) = &request.user {
        validate_value("user", user)?;
    }
    if let Some(password) = &request.password {
        validate_value("password", password)?;
    }
    Ok(())
}

fn ensure_snapshot_bytes(snapshot: &Fingerprint, bytes: &[u8], path: &Path) -> Result<(), String> {
    if (!snapshot.exists && bytes.is_empty())
        || (snapshot.exists
            && snapshot.length == bytes.len() as u64
            && snapshot.digest == digest(bytes))
    {
        return Ok(());
    }
    Err(format!(
        "CONCURRENT_EDIT: file changed while reading: {}",
        display_path(path)
    ))
}

fn validate_value(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.contains('\0') || value.contains('\r') || value.contains('\n') {
        return Err(format!(
            "{name}_INVALID: value must not be empty or contain control lines"
        ));
    }
    Ok(())
}

fn host_block(request: &CreateRequest, id: &str, eol: &str) -> String {
    let mut lines = vec![
        format!("##SSHX ID={id}"),
        format!("Host {}", request.alias),
        format!("  HostName {}", request.hostname),
    ];
    if let Some(user) = &request.user {
        lines.push(format!("  User {user}"));
    }
    if let Some(port) = request.port {
        lines.push(format!("  Port {port}"));
    }
    if let Some(password) = &request.password {
        lines.push(format!("  ##PASSWORD {password}"));
    }
    format!("{}{}", lines.join(eol), eol)
}

fn append_bytes(before: &[u8], addition: &[u8], eol: &[u8]) -> Vec<u8> {
    let mut after = Vec::with_capacity(before.len() + addition.len() + eol.len());
    after.extend_from_slice(before);
    if !before.is_empty() && !before.ends_with(eol) {
        after.extend_from_slice(eol);
    }
    after.extend_from_slice(addition);
    after
}

fn append_patch(path: &Path, before: &[u8], after: &[u8], password: Option<&str>) -> String {
    let added = &after[before.len()..];
    let mut patch = format!(
        "--- {}\n+++ {}\n@@\n",
        display_path(path),
        display_path(path)
    );
    for raw in String::from_utf8_lossy(added).split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let line = password.map_or_else(
            || line.to_string(),
            |secret| {
                if secret.is_empty() {
                    line.to_string()
                } else {
                    line.replace(secret, "<redacted>")
                }
            },
        );
        if !line.is_empty() {
            patch.push('+');
            patch.push_str(&line);
            patch.push('\n');
        }
    }
    patch
}

fn new_id(target: &[u8], root: &[u8]) -> String {
    let mut existing = HashSet::new();
    for bytes in [target, root] {
        for line in String::from_utf8_lossy(bytes).lines() {
            if let Some(id) = existing_id(line) {
                existing.insert(id);
            }
        }
    }
    loop {
        let sequence = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let value = now ^ (u128::from(std::process::id()) << 64) ^ u128::from(sequence);
        let candidate = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (value >> 96) as u32,
            (value >> 80) as u16,
            (((value >> 64) as u16) & 0x0fff) | 0x4000,
            (((value >> 48) as u16) & 0x3fff) | 0x8000,
            value & 0xffff_ffff_ffff,
        );
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
}

fn existing_id(line: &str) -> Option<String> {
    let marker = line.trim().strip_prefix("##SSHX")?.trim_start();
    let token = marker.split_whitespace().next()?;
    let (_, value) = token.split_once('=')?;
    (!value.is_empty() && token.starts_with("ID=")).then(|| value.to_string())
}

fn include_path(target: &Path) -> String {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".ssh"));
    if let Some(base) = base
        && let Ok(relative) = target.strip_prefix(&base)
    {
        return relative.to_string_lossy().into_owned();
    }
    target.to_string_lossy().into_owned()
}

fn mutation_lock_path(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    root.with_file_name(format!(".{name}.sshx.lock"))
}

fn read_regular_file(path: &Path, code: &str) -> Result<Vec<u8>, String> {
    read_optional_regular_file(path, code)?.ok_or_else(|| {
        format!(
            "{code}_NOT_FOUND: config file does not exist: {}",
            display_path(path)
        )
    })
}

fn read_optional_regular_file(path: &Path, code: &str) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{code}_SYMLINK: refusing symlink: {}",
            display_path(path)
        )),
        Ok(metadata) if !metadata.file_type().is_file() => Err(format!(
            "{code}_INVALID: not a regular file: {}",
            display_path(path)
        )),
        Ok(_) => fs::read(path).map(Some).map_err(|error| {
            format!(
                "{code}_READ_FAILED: cannot read {}: {error}",
                display_path(path)
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "{code}_STAT_FAILED: cannot inspect {}: {error}",
            display_path(path)
        )),
    }
}

fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Fingerprint {
                exists: false,
                length: 0,
                digest: 0,
                mode: 0o600,
                #[cfg(unix)]
                device: 0,
                #[cfg(unix)]
                inode: 0,
                modified: None,
            });
        }
        Err(error) => {
            return Err(format!(
                "CONCURRENT_EDIT: cannot inspect {}: {error}",
                display_path(path)
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(format!(
            "CONCURRENT_EDIT: {} is not a regular file",
            display_path(path)
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "CONCURRENT_EDIT: cannot read {}: {error}",
            display_path(path)
        )
    })?;
    let modified = metadata.modified().ok().and_then(|time| {
        time.duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| (duration.as_secs(), duration.subsec_nanos()))
    });
    Ok(Fingerprint {
        exists: true,
        length: bytes.len() as u64,
        digest: digest(&bytes),
        mode: file_mode(&metadata),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        modified,
    })
}

fn verify_snapshot(write: &WriteFile) -> Result<(), String> {
    let current = fingerprint(&write.path)?;
    if current != write.snapshot {
        return Err(format!(
            "CONCURRENT_EDIT: file changed since preview: {}",
            display_path(&write.path)
        ));
    }
    Ok(())
}

fn write_atomic(write: &WriteFile) -> Result<(), String> {
    verify_snapshot(write)?;
    if let Some(parent) = write.path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "TARGET_DIRECTORY_FAILED: cannot create {}: {error}",
                parent.display()
            )
        })?;
    }
    let temporary = write.path.with_file_name(format!(
        ".{}.sshx-{}",
        write
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(if write.existed { write.mode } else { 0o600 });
        let mut file = options.open(&temporary).map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot create {}: {error}",
                temporary.display()
            )
        })?;
        file.write_all(&write.after).map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot write {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot sync {}: {error}",
                temporary.display()
            )
        })?;
        #[cfg(unix)]
        if write.existed {
            fs::set_permissions(&temporary, fs::Permissions::from_mode(write.mode)).map_err(
                |error| {
                    format!(
                        "MUTATION_WRITE_FAILED: cannot preserve mode {}: {error}",
                        write.path.display()
                    )
                },
            )?;
        }
        verify_snapshot(write)?;
        fs::rename(&temporary, &write.path).map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot replace {}: {error}",
                write.path.display()
            )
        })?;
        if let Some(parent) = write.path.parent()
            && let Ok(directory) = File::open(parent)
        {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn restore(write: &WriteFile) -> Result<(), String> {
    let current = fingerprint(&write.path)?;
    if !current.exists
        || current.length != write.after.len() as u64
        || current.digest != digest(&write.after)
    {
        return Err(format!(
            "MUTATION_PARTIAL: refusing rollback after external change: {}",
            display_path(&write.path)
        ));
    }
    if write.existed {
        let restored = WriteFile {
            path: write.path.clone(),
            before: write.after.clone(),
            after: write.before.clone(),
            mode: write.mode,
            existed: true,
            snapshot: current,
        };
        write_atomic(&restored)
    } else {
        fs::remove_file(&write.path).map_err(|error| error.to_string())
    }
}

fn detect_line_ending(bytes: &[u8]) -> Option<String> {
    if bytes.windows(2).any(|window| window == b"\r\n") {
        Some("\r\n".to_string())
    } else if bytes.contains(&b'\n') {
        Some("\n".to_string())
    } else {
        None
    }
}

fn file_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        metadata.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o600
    }
}

fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("cannot resolve target path: {error}"))
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
