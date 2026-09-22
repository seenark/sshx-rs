use crate::discovery::{DiscoveryRoot, HostEntry, path_reachable};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::fd::AsRawFd;
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

#[derive(Clone, Debug)]
pub struct UpdateRequest {
    pub path: PathBuf,
    pub expected_id: String,
    pub selected_alias: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub alias: Option<String>,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub password: Option<String>,
    pub clear_user: bool,
    pub clear_port: bool,
    pub clear_password: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MutationKind {
    Update,
    Rename,
    Delete,
}

#[derive(Debug)]
pub struct EditPlan {
    pub id: String,
    pub operation: MutationKind,
    pub files: Vec<FileChange>,
    lock_path: PathBuf,
    writes: Vec<WriteFile>,
}

#[derive(Debug)]
pub struct CreatePlan {
    pub id: String,
    pub files: Vec<FileChange>,
    lock_path: PathBuf,
    writes: Vec<WriteFile>,
}

#[derive(Clone, Debug)]
pub struct PairMutationRequest {
    pub gateway_path: PathBuf,
    pub gateway_expected_id: String,
    pub gateway_id: String,
    pub gateway_alias: String,
    pub gateway_byte_start: usize,
    pub gateway_byte_end: usize,
    pub vm_path: PathBuf,
    pub vm_expected_id: String,
    pub vm_id: String,
    pub vm_alias: String,
    pub vm_byte_start: usize,
    pub vm_byte_end: usize,
    pub transit_host: String,
    pub transit_port: u16,
}

#[derive(Debug)]
pub struct PairPlan {
    pub gateway_id: String,
    pub vm_id: String,
    pub transit_host: String,
    pub transit_port: u16,
    pub files: Vec<FileChange>,
    lock_path: PathBuf,
    writes: Vec<WriteFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PairJournal {
    writes: Vec<PairJournalWrite>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PairJournalWrite {
    path: String,
    before: String,
    after: String,
    before_digest: u64,
    after_digest: u64,
    existed: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOperation {
    Create,
    Modify,
    Delete,
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
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
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
    apply_writes(&plan.lock_path, &plan.writes)
}

pub fn apply_edit(plan: &EditPlan) -> Result<(), String> {
    apply_writes(&plan.lock_path, &plan.writes)
}

pub fn plan_pair(request: &PairMutationRequest) -> Result<PairPlan, String> {
    validate_value("gateway_id", &request.gateway_id)?;
    validate_value("vm_id", &request.vm_id)?;
    validate_value("transit_host", &request.transit_host)?;
    if request.transit_port == 0 {
        return Err("TRANSIT_PORT_INVALID: transit port must be non-zero".to_string());
    }
    let gateway_request = UpdateRequest {
        path: request.gateway_path.clone(),
        expected_id: request.gateway_expected_id.clone(),
        selected_alias: request.gateway_alias.clone(),
        byte_start: request.gateway_byte_start,
        byte_end: request.gateway_byte_end,
        alias: None,
        hostname: None,
        user: None,
        port: None,
        password: None,
        clear_user: false,
        clear_port: false,
        clear_password: false,
    };
    let vm_request = UpdateRequest {
        path: request.vm_path.clone(),
        expected_id: request.vm_expected_id.clone(),
        selected_alias: request.vm_alias.clone(),
        byte_start: request.vm_byte_start,
        byte_end: request.vm_byte_end,
        alias: None,
        hostname: None,
        user: None,
        port: None,
        password: None,
        clear_user: false,
        clear_port: false,
        clear_password: false,
    };
    let gateway = load_block_with_span(
        &gateway_request,
        request.gateway_byte_start,
        request.gateway_byte_end,
    )?;
    let vm = load_block_with_span(&vm_request, request.vm_byte_start, request.vm_byte_end)?;
    let mut gateway_changes = pair_identity_changes(&gateway, &request.gateway_id, false, "", 0)?;
    gateway_changes.push(insert_metadata(
        &gateway,
        &format!("##SSHX VM={}", request.vm_id),
    ));
    let vm_changes = pair_identity_changes(
        &vm,
        &request.vm_id,
        true,
        &request.gateway_id,
        request.transit_port,
    )
    .map(|mut changes| {
        changes.push(insert_metadata(
            &vm,
            &format!(
                "##SSHX TRANSIT={}:{}",
                request.transit_host, request.transit_port
            ),
        ));
        changes
    })?;

    let same_file = gateway.path == vm.path;
    let mut files = Vec::new();
    let mut writes = Vec::new();
    if same_file {
        if gateway.before != vm.before {
            return Err("CONFIG_CHANGED: pair entries changed while reading".to_string());
        }
        let mut changes = gateway_changes;
        changes.extend(vm_changes);
        let after = apply_changes(&gateway.before, changes);
        if gateway.before != after {
            files.push(FileChange {
                path: display_path(&gateway.path),
                operation: FileOperation::Modify,
                patch: edit_patch(&gateway.path, &gateway.before, &after),
            });
            writes.push(WriteFile {
                path: gateway.path.clone(),
                before: gateway.before.clone(),
                after,
                mode: gateway.mode,
                existed: true,
                snapshot: gateway.snapshot.clone(),
            });
        }
    } else {
        let gateway_after = apply_changes(&gateway.before, gateway_changes);
        if gateway.before != gateway_after {
            files.push(FileChange {
                path: display_path(&gateway.path),
                operation: FileOperation::Modify,
                patch: edit_patch(&gateway.path, &gateway.before, &gateway_after),
            });
            writes.push(WriteFile {
                path: gateway.path.clone(),
                before: gateway.before.clone(),
                after: gateway_after,
                mode: gateway.mode,
                existed: true,
                snapshot: gateway.snapshot.clone(),
            });
        }
        let vm_after = apply_changes(&vm.before, vm_changes);
        if vm.before != vm_after {
            files.push(FileChange {
                path: display_path(&vm.path),
                operation: FileOperation::Modify,
                patch: edit_patch(&vm.path, &vm.before, &vm_after),
            });
            writes.push(WriteFile {
                path: vm.path.clone(),
                before: vm.before,
                after: vm_after,
                mode: vm.mode,
                existed: true,
                snapshot: vm.snapshot,
            });
        }
    }
    Ok(PairPlan {
        gateway_id: request.gateway_id.clone(),
        vm_id: request.vm_id.clone(),
        transit_host: request.transit_host.clone(),
        transit_port: request.transit_port,
        files,
        lock_path: mutation_lock_path(&gateway.path),
        writes,
    })
}

pub fn apply_pair(plan: &PairPlan) -> Result<(), String> {
    recover_pair_journal(&plan.lock_path)?;
    let _lock = WriterLock::acquire(&plan.lock_path)?;
    for write in &plan.writes {
        verify_snapshot(write)?;
    }
    if plan.writes.is_empty() {
        return Ok(());
    }
    let journal_path = pair_journal_path(&plan.lock_path);
    let mut journal = PairJournal { writes: Vec::new() };
    for write in &plan.writes {
        let before = pair_temp(&write.path, "before", &write.before, write.mode)?;
        let after = pair_temp(&write.path, "after", &write.after, write.mode)?;
        journal.writes.push(PairJournalWrite {
            path: display_path(&write.path),
            before: display_path(&before),
            after: display_path(&after),
            before_digest: digest(&write.before),
            after_digest: digest(&write.after),
            existed: write.existed,
        });
    }
    write_pair_journal(&journal_path, &journal)?;
    let result = (|| {
        for write in &journal.writes {
            let path = PathBuf::from(&write.path);
            let current = fingerprint(&path)?;
            if !current.exists || current.digest != write.before_digest {
                return Err(format!(
                    "CONCURRENT_EDIT: file changed during pair commit: {}",
                    path.display()
                ));
            }
            fs::rename(&write.after, &path).map_err(|error| {
                format!(
                    "MUTATION_WRITE_FAILED: cannot replace {}: {error}",
                    path.display()
                )
            })?;
            sync_parent(&path)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let recovery = rollback_pair_journal(&journal_path, &journal);
        return match recovery {
            Ok(()) => Err(error),
            Err(recovery_error) => Err(format!(
                "MUTATION_PARTIAL: {error}; recovery failed: {recovery_error}"
            )),
        };
    }
    cleanup_pair_journal(&journal_path, &journal);
    Ok(())
}

pub fn recover_pair_journals(paths: &[PathBuf]) -> Result<(), String> {
    for path in paths {
        recover_pair_journal(&mutation_lock_path(path))?;
    }
    Ok(())
}

fn pair_journal_path(lock_path: &Path) -> PathBuf {
    lock_path.with_file_name(format!(
        "{}.journal",
        lock_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sshx")
    ))
}

fn pair_temp(path: &Path, kind: &str, bytes: &[u8], mode: u32) -> Result<PathBuf, String> {
    let temporary = path.with_file_name(format!(
        ".{}.sshx-pair-{kind}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).map_err(|error| {
        format!(
            "MUTATION_WRITE_FAILED: cannot create {}: {error}",
            temporary.display()
        )
    })?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(temporary)
}

fn write_pair_journal(path: &Path, journal: &PairJournal) -> Result<(), String> {
    let temporary = pair_temp(
        path,
        "journal",
        &serde_json::to_vec(journal).map_err(|error| error.to_string())?,
        0o600,
    )?;
    fs::rename(&temporary, path).map_err(|error| {
        format!(
            "MUTATION_WRITE_FAILED: cannot publish {}: {error}",
            path.display()
        )
    })?;
    sync_parent(path)
}

fn recover_pair_journal(lock_path: &Path) -> Result<(), String> {
    let journal_path = pair_journal_path(lock_path);
    if !journal_path.is_file() {
        return Ok(());
    }
    let bytes = fs::read(&journal_path)
        .map_err(|error| format!("MUTATION_RECOVERY_FAILED: cannot read journal: {error}"))?;
    let journal: PairJournal = serde_json::from_slice(&bytes)
        .map_err(|error| format!("MUTATION_RECOVERY_FAILED: invalid journal: {error}"))?;
    let result = rollback_pair_journal(&journal_path, &journal);
    let _ = fs::remove_file(lock_path);
    result
}

fn rollback_pair_journal(path: &Path, journal: &PairJournal) -> Result<(), String> {
    let mut failure = None;
    for write in journal.writes.iter().rev() {
        let target = PathBuf::from(&write.path);
        let current = fingerprint(&target)?;
        if current.exists && current.digest == write.after_digest {
            let before = PathBuf::from(&write.before);
            if write.existed {
                fs::rename(&before, &target).map_err(|error| error.to_string())?;
                sync_parent(&target)?;
            } else {
                fs::remove_file(&target).map_err(|error| error.to_string())?;
            }
        } else if current.exists && current.digest == write.before_digest {
            let _ = fs::remove_file(&write.before);
        } else {
            failure = Some(format!(
                "MUTATION_PARTIAL: refusing rollback after external change: {}",
                target.display()
            ));
        }
        let _ = fs::remove_file(&write.after);
    }
    let _ = fs::remove_file(path);
    if let Some(error) = failure {
        Err(error)
    } else {
        Ok(())
    }
}

fn cleanup_pair_journal(path: &Path, journal: &PairJournal) {
    for write in &journal.writes {
        let _ = fs::remove_file(&write.before);
        let _ = fs::remove_file(&write.after);
    }
    let _ = fs::remove_file(path);
}

fn sync_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let directory = File::open(parent)
            .map_err(|error| format!("MUTATION_WRITE_FAILED: cannot open parent: {error}"))?;
        directory
            .sync_all()
            .map_err(|error| format!("MUTATION_WRITE_FAILED: cannot sync parent: {error}"))?;
    }
    Ok(())
}

fn pair_identity_changes(
    loaded: &LoadedBlock,
    id: &str,
    vm: bool,
    gateway_id: &str,
    transit_port: u16,
) -> Result<Vec<Change>, String> {
    let metadata = metadata_lines(loaded);
    if vm
        && (metadata.iter().any(|line| line.0 == "GATEWAY")
            || metadata.iter().any(|line| line.0 == "TRANSIT"))
    {
        return Err("PAIR_EXISTS: VM entry already has pair metadata".to_string());
    }
    let mut changes = Vec::new();
    if loaded.marker_range.is_none() {
        changes.push(insert_metadata(loaded, &format!("##SSHX ID={id}")));
    }
    if vm {
        changes.push(insert_metadata(
            loaded,
            &format!("##SSHX GATEWAY={gateway_id}"),
        ));
        if transit_port == 0 {
            return Err("TRANSIT_PORT_INVALID: transit port must be non-zero".to_string());
        }
    }
    Ok(changes)
}

fn metadata_lines(loaded: &LoadedBlock) -> Vec<(String, String)> {
    (loaded.block_start_line..loaded.host_line)
        .chain(loaded.host_line + 1..loaded.block_end_line)
        .filter_map(|index| {
            let line = loaded.lines[index];
            let text = String::from_utf8_lossy(&loaded.before[line.start..line.content_end]);
            let rest = text.trim().strip_prefix("##SSHX")?.trim();
            let (name, value) = rest
                .split_once('=')
                .or_else(|| rest.split_once(char::is_whitespace))?;
            Some((name.trim().to_ascii_uppercase(), value.trim().to_string()))
        })
        .collect()
}

fn insert_metadata(loaded: &LoadedBlock, metadata: &str) -> Change {
    let line = loaded.lines[loaded.host_line];
    let eol = if line.end > line.content_end {
        loaded.before[line.content_end..line.end].to_vec()
    } else {
        detect_line_ending(&loaded.before)
            .unwrap_or_else(|| "\n".to_string())
            .into_bytes()
    };
    let mut replacement = metadata.as_bytes().to_vec();
    replacement.extend_from_slice(&eol);
    Change {
        start: line.start,
        end: line.start,
        replacement,
    }
}

pub fn validate_mutation_roots(roots: &[DiscoveryRoot]) -> Result<(), String> {
    for root in roots {
        validate_path_components(&root.path, "CONFIG_ROOT")?;
    }
    Ok(())
}

pub fn validate_entry_paths(entry: &HostEntry) -> Result<(), String> {
    for provenance in &entry.provenance {
        for path in &provenance.paths {
            let path = Path::new(path);
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "CONFIG_CHANGED: cannot inspect selected path {}: {error}",
                    display_path(path)
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "MUTATION_SYMLINK: refusing symlink in selected include path: {}",
                    display_path(path)
                ));
            }
        }
    }
    validate_path_components(Path::new(&entry.source.path), "SOURCE_FILE")
}

pub fn plan_update(request: &UpdateRequest) -> Result<EditPlan, String> {
    validate_update_request(request)?;
    let loaded = load_block(request)?;
    let mut changes = Vec::new();
    if let Some(alias) = &request.alias {
        changes.push(replace_alias(&loaded, &request.selected_alias, alias)?);
    }
    let port_value = request.port.map(|port| port.to_string());
    for (keyword, value, clear) in [
        ("HostName", request.hostname.as_deref(), false),
        ("User", request.user.as_deref(), request.clear_user),
        ("Port", port_value.as_deref(), request.clear_port),
    ] {
        if (value.is_some() || clear)
            && let Some(change) = replace_directive(&loaded, keyword, value, clear)?
        {
            changes.push(change);
        }
    }
    if request.password.is_some() || request.clear_password {
        changes.push(replace_password(
            &loaded,
            request.password.as_deref(),
            request.clear_password,
        )?);
    }
    let after = apply_changes(&loaded.before, changes);
    Ok(make_edit_plan(
        request.expected_id.clone(),
        MutationKind::Update,
        loaded,
        after,
    ))
}

pub fn plan_delete(
    path: &Path,
    expected_id: &str,
    selected_alias: &str,
    byte_start: usize,
    byte_end: usize,
) -> Result<EditPlan, String> {
    let request = UpdateRequest {
        path: path.to_path_buf(),
        expected_id: expected_id.to_string(),
        selected_alias: selected_alias.to_string(),
        byte_start,
        byte_end,
        alias: None,
        hostname: None,
        user: None,
        port: None,
        password: None,
        clear_user: false,
        clear_port: false,
        clear_password: false,
    };
    let loaded = load_block_with_span(&request, byte_start, byte_end)?;
    let after = delete_block(&loaded);
    Ok(make_edit_plan(
        expected_id.to_string(),
        MutationKind::Delete,
        loaded,
        after,
    ))
}

fn apply_writes(lock_path: &Path, writes: &[WriteFile]) -> Result<(), String> {
    let _lock = WriterLock::acquire(lock_path)?;
    for write in writes {
        verify_snapshot(write)?;
    }
    let mut applied = Vec::new();
    for write in writes {
        if let Err(error) = write_atomic(write) {
            let mut recovery_errors = Vec::new();
            for previous in applied.into_iter().rev() {
                if let Err(recovery_error) = restore(previous) {
                    recovery_errors.push(recovery_error);
                }
            }
            if recovery_errors.is_empty() {
                return Err(error);
            }
            return Err(format!(
                "MUTATION_PARTIAL: {error}; recovery failed: {}",
                recovery_errors.join("; ")
            ));
        }
        applied.push(write);
    }

    Ok(())
}

#[derive(Clone, Copy)]

struct ByteLine {
    start: usize,
    content_end: usize,
    end: usize,
}

#[derive(Clone)]
struct LoadedBlock {
    path: PathBuf,
    before: Vec<u8>,
    snapshot: Fingerprint,
    mode: u32,
    lines: Vec<ByteLine>,
    host_line: usize,
    block_start_line: usize,
    block_end_line: usize,
    block_end: usize,
    marker_range: Option<(usize, usize)>,
}

#[derive(Clone)]
struct Change {
    start: usize,
    end: usize,
    replacement: Vec<u8>,
}

fn validate_update_request(request: &UpdateRequest) -> Result<(), String> {
    if request.alias.is_none()
        && request.hostname.is_none()
        && request.user.is_none()
        && request.port.is_none()
        && request.password.is_none()
        && !request.clear_user
        && !request.clear_port
        && !request.clear_password
    {
        return Err("MUTATION_EMPTY: provide at least one host field".to_string());
    }
    if let Some(alias) = &request.alias {
        validate_alias(alias)?;
    }
    for (name, value) in [
        ("hostname", request.hostname.as_deref()),
        ("user", request.user.as_deref()),
        ("password", request.password.as_deref()),
    ] {
        if let Some(value) = value {
            validate_value(name, value)?;
        }
    }
    if request.user.is_some() && request.clear_user
        || request.port.is_some() && request.clear_port
        || request.password.is_some() && request.clear_password
    {
        return Err("MUTATION_CONFLICT: cannot set and clear one field together".to_string());
    }
    Ok(())
}

fn validate_alias(alias: &str) -> Result<(), String> {
    validate_value("alias", alias)?;
    if alias.chars().any(|character| {
        character.is_whitespace() || matches!(character, '*' | '?' | '[' | ']' | '!')
    }) {
        return Err("ALIAS_INVALID: alias must be one exact Host token".to_string());
    }
    Ok(())
}

fn load_block(request: &UpdateRequest) -> Result<LoadedBlock, String> {
    load_block_with_span(request, request.byte_start, request.byte_end)
}

fn load_block_with_span(
    request: &UpdateRequest,
    byte_start: usize,
    byte_end: usize,
) -> Result<LoadedBlock, String> {
    let path = absolute_path(&request.path)?;
    validate_path_components(&path, "SOURCE_FILE")?;
    let before = read_regular_file(&path, "SOURCE_FILE")?;
    let snapshot = fingerprint(&path)?;
    ensure_snapshot_bytes(&snapshot, &before, &path)?;
    let mode = snapshot.mode;
    let lines = split_byte_lines(&before);
    let host_line = lines
        .iter()
        .position(|line| line.start == byte_start)
        .ok_or_else(|| "CONFIG_CHANGED: selected Host line no longer exists".to_string())?;
    if !is_host_boundary(&before, lines[host_line]) {
        return Err("CONFIG_CHANGED: selected span is no longer a Host block".to_string());
    }
    let block_end_line = lines
        .iter()
        .enumerate()
        .skip(host_line + 1)
        .find(|(_, line)| is_boundary(&before, **line))
        .map_or(lines.len(), |(index, _)| index);
    let block_end = lines
        .get(block_end_line.saturating_sub(1))
        .map_or(before.len(), |line| line.end);
    if block_end != byte_end {
        return Err("CONFIG_CHANGED: selected Host block span is stale".to_string());
    }
    let spans = token_spans(&before[lines[host_line].start..lines[host_line].content_end]);
    let aliases = spans
        .iter()
        .skip(1)
        .map(|(start, end)| {
            decode_token(&before[lines[host_line].start + *start..lines[host_line].start + *end])
        })
        .collect::<Vec<_>>();
    if !aliases.iter().any(|alias| alias == &request.selected_alias) {
        return Err("CONFIG_CHANGED: selected alias no longer exists".to_string());
    }

    let block_start_line = (0..host_line)
        .rev()
        .find(|index| is_boundary(&before, lines[*index]))
        .map_or(0, |index| index + 1);
    let marker_range = (block_start_line..host_line).find_map(|index| {
        let line = lines[index];
        let text = String::from_utf8_lossy(&before[line.start..line.content_end]);
        (existing_id(&text) == Some(request.expected_id.clone())).then_some((line.start, line.end))
    });
    let actual_id = marker_range
        .as_ref()
        .map(|_| request.expected_id.clone())
        .unwrap_or_else(|| synthetic_entry_id(&path, byte_start, byte_end));
    if actual_id != request.expected_id {
        return Err("CONFIG_CHANGED: selected Host identity is stale".to_string());
    }
    Ok(LoadedBlock {
        path,
        before,
        snapshot,
        mode,
        lines,
        host_line,
        block_start_line,
        block_end_line,
        block_end,
        marker_range,
    })
}

fn make_edit_plan(
    id: String,
    operation: MutationKind,
    loaded: LoadedBlock,
    after: Vec<u8>,
) -> EditPlan {
    let LoadedBlock {
        path,
        before,
        snapshot,
        mode,
        ..
    } = loaded;
    let files = if before == after {
        Vec::new()
    } else {
        vec![FileChange {
            path: display_path(&path),
            operation: match operation {
                MutationKind::Delete => FileOperation::Delete,
                MutationKind::Update | MutationKind::Rename => FileOperation::Modify,
            },
            patch: edit_patch(&path, &before, &after),
        }]
    };
    let writes = if before == after {
        Vec::new()
    } else {
        vec![WriteFile {
            path: path.clone(),
            before,
            after,
            mode,
            existed: true,
            snapshot,
        }]
    };
    EditPlan {
        id,
        operation,
        files,
        lock_path: mutation_lock_path(&path),
        writes,
    }
}

fn replace_alias(
    loaded: &LoadedBlock,
    selected_alias: &str,
    alias: &str,
) -> Result<Change, String> {
    let line = loaded.lines[loaded.host_line];
    let raw = &loaded.before[line.start..line.content_end];
    let spans = token_spans(raw);
    let (start, end) = spans
        .iter()
        .skip(1)
        .find(|(start, end)| decode_token(&raw[*start..*end]) == selected_alias)
        .copied()
        .ok_or_else(|| "CONFIG_CHANGED: selected alias no longer exists".to_string())?;
    Ok(Change {
        start: line.start + start,
        end: line.start + end,
        replacement: alias.as_bytes().to_vec(),
    })
}

fn replace_directive(
    loaded: &LoadedBlock,
    keyword: &str,
    value: Option<&str>,
    clear: bool,
) -> Result<Option<Change>, String> {
    for index in loaded.host_line + 1..loaded.block_end_line {
        let line = loaded.lines[index];
        let raw = &loaded.before[line.start..line.content_end];
        if !directive_matches(raw, keyword) {
            continue;
        }
        if clear {
            return Ok(Some(Change {
                start: line.start,
                end: line.end,
                replacement: Vec::new(),
            }));
        }
        let Some((start, end)) = argument_span(raw, keyword) else {
            return Err(format!("CONFIG_CHANGED: {keyword} directive has no value"));
        };
        return Ok(Some(Change {
            start: line.start + start,
            end: line.start + end,
            replacement: value.unwrap_or_default().as_bytes().to_vec(),
        }));
    }
    if clear {
        return Ok(None);
    }
    Ok(Some(insert_directive(
        loaded,
        keyword,
        value.unwrap_or_default(),
    )))
}

fn replace_password(
    loaded: &LoadedBlock,
    value: Option<&str>,
    clear: bool,
) -> Result<Change, String> {
    for index in loaded.host_line + 1..loaded.block_end_line {
        let line = loaded.lines[index];
        let raw = &loaded.before[line.start..line.content_end];
        if !password_line(raw) {
            continue;
        }
        if clear {
            return Ok(Change {
                start: line.start,
                end: line.end,
                replacement: Vec::new(),
            });
        }
        let start = password_value_start(raw);
        return Ok(Change {
            start: line.start + start,
            end: line.content_end,
            replacement: value.unwrap_or_default().as_bytes().to_vec(),
        });
    }
    if clear {
        return Err("CONFIG_CHANGED: password metadata is already absent".to_string());
    }
    Ok(insert_directive(
        loaded,
        "##PASSWORD",
        value.unwrap_or_default(),
    ))
}

fn insert_directive(loaded: &LoadedBlock, keyword: &str, value: &str) -> Change {
    let line = loaded.lines[loaded.host_line];
    let eol = if line.end > line.content_end {
        loaded.before[line.content_end..line.end].to_vec()
    } else if let Some(eol) = detect_line_ending(&loaded.before) {
        eol.into_bytes()
    } else {
        b"\n".to_vec()
    };
    let mut replacement = Vec::new();
    if line.end == line.content_end {
        replacement.extend_from_slice(&eol);
    }
    replacement.extend_from_slice(b"  ");
    replacement.extend_from_slice(keyword.as_bytes());
    replacement.push(b' ');
    replacement.extend_from_slice(value.as_bytes());
    replacement.extend_from_slice(&eol);
    Change {
        start: line.end,
        end: line.end,
        replacement,
    }
}

fn apply_changes(before: &[u8], mut changes: Vec<Change>) -> Vec<u8> {
    changes.sort_by_key(|change| std::cmp::Reverse(change.start));
    let mut after = before.to_vec();
    for change in changes {
        after.splice(change.start..change.end, change.replacement);
    }
    after
}

fn delete_block(loaded: &LoadedBlock) -> Vec<u8> {
    let host = loaded.lines[loaded.host_line];
    let mut after = Vec::with_capacity(
        loaded
            .before
            .len()
            .saturating_sub(loaded.block_end.saturating_sub(host.start)),
    );
    if let Some((marker_start, marker_end)) = loaded.marker_range {
        after.extend_from_slice(&loaded.before[..marker_start]);
        after.extend_from_slice(&loaded.before[marker_end..host.start]);
    } else {
        after.extend_from_slice(&loaded.before[..host.start]);
    }
    after.extend_from_slice(&loaded.before[loaded.block_end..]);
    after
}

fn split_byte_lines(bytes: &[u8]) -> Vec<ByteLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let content_end = if index > start && bytes[index - 1] == b'\r' {
            index - 1
        } else {
            index
        };
        lines.push(ByteLine {
            start,
            content_end,
            end: index + 1,
        });
        start = index + 1;
    }
    if start < bytes.len() {
        lines.push(ByteLine {
            start,
            content_end: bytes.len(),
            end: bytes.len(),
        });
    }
    lines
}

fn is_host_boundary(bytes: &[u8], line: ByteLine) -> bool {
    let raw = &bytes[line.start..line.content_end];
    directive_matches(raw, "Host") && token_spans(raw).len() > 1
}

fn is_boundary(bytes: &[u8], line: ByteLine) -> bool {
    let raw = &bytes[line.start..line.content_end];
    directive_matches(raw, "Match") || is_host_boundary(bytes, line)
}

fn directive_matches(raw: &[u8], keyword: &str) -> bool {
    let spans = token_spans(raw);
    let Some((start, end)) = spans.first().copied() else {
        return false;
    };
    let token = &raw[start..end];
    let name_end = token
        .iter()
        .position(|byte| *byte == b'=')
        .unwrap_or(token.len());
    token[..name_end].eq_ignore_ascii_case(keyword.as_bytes())
}

fn argument_span(raw: &[u8], keyword: &str) -> Option<(usize, usize)> {
    let spans = token_spans(raw);
    let (first_start, first_end) = spans.first().copied()?;
    let first = &raw[first_start..first_end];
    if let Some(equal) = first.iter().position(|byte| *byte == b'=')
        && first[..equal].eq_ignore_ascii_case(keyword.as_bytes())
    {
        return Some((first_start + equal + 1, first_end));
    }
    if directive_matches(raw, keyword) {
        spans.get(1).copied()
    } else {
        None
    }
}

fn token_spans(raw: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        while index < raw.len() && raw[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= raw.len() || raw[index] == b'#' {
            break;
        }
        let start = index;
        let mut quote = None;
        let mut escaped = false;
        while index < raw.len() {
            let byte = raw[index];
            if escaped {
                escaped = false;
                index += 1;
                continue;
            }
            if byte == b'\\' && quote != Some(b'\'') {
                escaped = true;
                index += 1;
                continue;
            }
            if let Some(active) = quote {
                if byte == active {
                    quote = None;
                }
                index += 1;
                continue;
            }
            if byte == b'\'' || byte == b'"' {
                quote = Some(byte);
                index += 1;
                continue;
            }
            if byte.is_ascii_whitespace() || byte == b'#' {
                break;
            }
            index += 1;
        }
        if start < index {
            spans.push((start, index));
        }
        while index < raw.len() && raw[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < raw.len() && raw[index] == b'#' {
            break;
        }
    }
    spans
}

fn decode_token(raw: &[u8]) -> String {
    let raw = if raw.len() >= 2
        && matches!(
            (raw.first(), raw.last()),
            (Some(b'\''), Some(b'\'')) | (Some(b'"'), Some(b'"'))
        ) {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    let mut value = Vec::with_capacity(raw.len());
    let mut escaped = false;
    for byte in raw {
        if escaped {
            value.push(*byte);
            escaped = false;
        } else if *byte == b'\\' {
            escaped = true;
        } else {
            value.push(*byte);
        }
    }
    if escaped {
        value.push(b'\\');
    }
    String::from_utf8_lossy(&value).into_owned()
}

fn password_line(raw: &[u8]) -> bool {
    let trimmed = raw
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |start| &raw[start..]);
    trimmed
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"##PASSWORD"))
        && trimmed.get(10).is_some_and(u8::is_ascii_whitespace)
}

fn password_value_start(raw: &[u8]) -> usize {
    let marker_start = raw
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(0);
    let mut index = marker_start + 10;
    while index < raw.len() && raw[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn synthetic_entry_id(path: &Path, byte_start: usize, byte_end: usize) -> String {
    format!("{}#{}-{}", display_path(path), byte_start, byte_end)
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

fn validate_path_components(path: &Path, code: &str) -> Result<(), String> {
    let path = absolute_path(path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{code}_SYMLINK: refusing symlink: {}",
            display_path(&path)
        )),
        Ok(metadata) if !metadata.file_type().is_file() => Err(format!(
            "{code}_INVALID: not a regular file: {}",
            display_path(&path)
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "{code}_STAT_FAILED: cannot inspect {}: {error}",
            display_path(&path)
        )),
    }
}

fn edit_patch(path: &Path, before: &[u8], after: &[u8]) -> String {
    let old_lines = split_byte_lines(before);
    let new_lines = split_byte_lines(after);
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && before[old_lines[prefix].start..old_lines[prefix].end]
            == after[new_lines[prefix].start..new_lines[prefix].end]
    {
        prefix += 1;
    }
    let mut old_end = old_lines.len();
    let mut new_end = new_lines.len();
    while old_end > prefix
        && new_end > prefix
        && before[old_lines[old_end - 1].start..old_lines[old_end - 1].end]
            == after[new_lines[new_end - 1].start..new_lines[new_end - 1].end]
    {
        old_end -= 1;
        new_end -= 1;
    }
    let old_changed = &old_lines[prefix..old_end];
    let new_changed = &new_lines[prefix..new_end];
    let secret = old_changed
        .iter()
        .any(|line| password_line(&before[line.start..line.content_end]))
        || new_changed
            .iter()
            .any(|line| password_line(&after[line.start..line.content_end]));
    let mut patch = format!(
        "--- {}\n+++ {}\n@@\n",
        display_path(path),
        display_path(path)
    );
    if secret {
        if !old_changed.is_empty() {
            patch.push_str("-<redacted secret-bearing block>\n");
        }
        if !new_changed.is_empty() {
            patch.push_str("+<redacted secret-bearing block>\n");
        }
        return patch;
    }
    for line in old_changed {
        patch.push('-');
        patch.push_str(&display_line(before, *line));
        patch.push('\n');
    }
    for line in new_changed {
        patch.push('+');
        patch.push_str(&display_line(after, *line));
        patch.push('\n');
    }
    patch
}

fn display_line(bytes: &[u8], line: ByteLine) -> String {
    String::from_utf8_lossy(&bytes[line.start..line.content_end]).into_owned()
}

pub fn generate_id(existing: &HashSet<String>) -> String {
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

fn new_id(target: &[u8], root: &[u8]) -> String {
    let mut existing = HashSet::new();
    for bytes in [target, root] {
        for line in String::from_utf8_lossy(bytes).lines() {
            if let Some(id) = existing_id(line) {
                existing.insert(id);
            }
        }
    }
    generate_id(&existing)
}

fn existing_id(line: &str) -> Option<String> {
    let marker = line.trim().strip_prefix("##SSHX")?.trim_start();
    let token = marker.split_whitespace().next()?;
    let (name, value) = token.split_once('=')?;
    (name.eq_ignore_ascii_case("ID") && !value.is_empty()).then(|| value.to_string())
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
                #[cfg(unix)]
                uid: 0,
                #[cfg(unix)]
                gid: 0,
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
        uid: metadata.uid(),
        #[cfg(unix)]
        gid: metadata.gid(),
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
    validate_path_components(&write.path, "MUTATION")?;
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
        #[cfg(unix)]
        if write.existed
            && unsafe { libc::fchown(file.as_raw_fd(), write.snapshot.uid, write.snapshot.gid) }
                != 0
        {
            return Err(format!(
                "MUTATION_WRITE_FAILED: cannot preserve ownership {}",
                write.path.display()
            ));
        }
        file.write_all(&write.after).map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot write {}: {error}",
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
        file.sync_all().map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot sync {}: {error}",
                temporary.display()
            )
        })?;
        verify_snapshot(write)?;
        validate_path_components(&write.path, "MUTATION")?;
        fs::rename(&temporary, &write.path).map_err(|error| {
            format!(
                "MUTATION_WRITE_FAILED: cannot replace {}: {error}",
                write.path.display()
            )
        })?;
        if let Some(parent) = write.path.parent() {
            let directory = File::open(parent).map_err(|error| {
                format!(
                    "MUTATION_WRITE_FAILED: cannot open parent {}: {error}",
                    parent.display()
                )
            })?;
            directory.sync_all().map_err(|error| {
                format!(
                    "MUTATION_WRITE_FAILED: cannot sync parent {}: {error}",
                    parent.display()
                )
            })?;
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
