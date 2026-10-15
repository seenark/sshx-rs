//! Per-HostEntry usage state (ADR 0004): last use, use count and last selected
//! service forwards, stored in a sidecar file beside each config root.

use crate::discovery::{DiscoveryRoot, HostEntry, absolute_path};
use crate::session::{self, ServiceForward};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UsedForward {
    pub remote: u16,
    pub local: u16,
    pub declared: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostUsage {
    pub last_used: u64,
    pub count: u64,
    pub forwards: Vec<UsedForward>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UsageFile {
    version: u32,
    hosts: BTreeMap<String, HostUsage>,
}

impl Default for UsageFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            hosts: BTreeMap::new(),
        }
    }
}

struct RootUsage {
    dirs: Vec<PathBuf>,
    file: UsageFile,
}

/// Usage state of every registered config root, read-only.
#[derive(Default)]
pub struct UsageIndex {
    roots: HashMap<PathBuf, RootUsage>,
}

impl UsageIndex {
    /// Missing, unreadable, corrupt, or foreign-version sidecars yield an empty
    /// state: usage only orders the TUI and must never block it.
    pub fn load(roots: &[DiscoveryRoot]) -> Self {
        let mut index = Self::default();
        for root in roots {
            let Ok(path) = absolute_path(&root.path) else {
                continue;
            };
            let file = read_sidecar(&sidecar_path(&path));
            index.roots.insert(
                path.clone(),
                RootUsage {
                    dirs: root_dirs(&path),
                    file,
                },
            );
        }
        index
    }

    pub fn get(&self, entry: &HostEntry) -> Option<&HostUsage> {
        let root = self.roots.get(&entry_root(entry)?)?;
        root.file.hosts.get(&key_in(&root.dirs, entry))
    }

    pub fn frequency(&self, entry: &HostEntry) -> u64 {
        self.get(entry).map_or(0, |usage| usage.count)
    }

    pub fn last_used(&self, entry: &HostEntry) -> Option<u64> {
        self.get(entry).map(|usage| usage.last_used)
    }
}

/// `.<root-file-name>.sshx-usage.json` beside the root file, named like the
/// mutation lock.
pub fn sidecar_path(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    root.with_file_name(format!(".{name}.sshx-usage.json"))
}

/// The `##SSHX ID` when present, otherwise `<first alias>@<source path
/// relative to the root file's directory>` (absolute when outside it).
pub fn host_key(root: &Path, entry: &HostEntry) -> String {
    key_in(&root_dirs(root), entry)
}

/// The first registered root whose Include traversal reached the entry:
/// discovery records it as the head of the first provenance chain.
pub fn entry_root(entry: &HostEntry) -> Option<PathBuf> {
    entry.provenance.first()?.paths.first().map(PathBuf::from)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Load-modify-write of the entry's root sidecar: `count += 1`,
/// `last_used = now`, forwards replaced. Never touches SSH config.
// shortcut: no cross-process lock, so two simultaneous connects can lose one
// increment; add a lock only if exact counts ever matter.
pub fn record(entry: &HostEntry, forwards: &[UsedForward], now: u64) -> Result<(), String> {
    let root = entry_root(entry).ok_or_else(|| "USAGE_NO_ROOT: entry has no root".to_string())?;
    let path = sidecar_path(&root);
    let mut file = read_sidecar(&path);
    let usage = file.hosts.entry(host_key(&root, entry)).or_default();
    usage.count += 1;
    usage.last_used = now;
    usage.forwards = forwards.to_vec();
    write_sidecar(&path, &file)
}

/// Move usage from `old_key` to `new_key` after a mutation gave the HostEntry
/// a `##SSHX ID`. An absent `old_key` is a no-op.
pub fn rekey(root: &Path, old_key: &str, new_key: &str) -> Result<(), String> {
    let path = sidecar_path(root);
    let mut file = read_sidecar(&path);
    let Some(usage) = file.hosts.remove(old_key) else {
        return Ok(());
    };
    file.hosts.insert(new_key.to_string(), usage);
    write_sidecar(&path, &file)
}

/// Service forwards of a session or tunnel (`--forward`, ports-screen rows); custom
/// `-L`/`-R`/`-D` rows are not service forwards.
pub fn used_from_service_forwards(
    entry: &HostEntry,
    forwards: &[ServiceForward],
) -> Vec<UsedForward> {
    let services = session::declared_services(entry).unwrap_or_default();
    forwards
        .iter()
        .filter(|forward| {
            !["local:", "remote:", "socks:", "custom "]
                .iter()
                .any(|prefix| forward.id.starts_with(prefix))
        })
        .map(|forward| UsedForward {
            remote: forward.remote_port,
            local: forward.local_port,
            declared: services.iter().any(|service| {
                service.remote_port == forward.remote_port
                    && service.destination_host == forward.destination_host
            }),
        })
        .collect()
}

fn root_dirs(root: &Path) -> Vec<PathBuf> {
    let Some(dir) = root.parent() else {
        return Vec::new();
    };
    // Discovery records canonical source paths; the root path may not be.
    let mut dirs = Vec::with_capacity(2);
    if let Ok(canonical) = fs::canonicalize(dir) {
        dirs.push(canonical);
    }
    dirs.push(dir.to_path_buf());
    dirs
}

fn key_in(dirs: &[PathBuf], entry: &HostEntry) -> String {
    let synthetic = format!(
        "{}#{}-{}",
        entry.source.path, entry.source.byte_start, entry.source.byte_end
    );
    if entry.id != synthetic {
        return entry.id.clone();
    }
    let source = Path::new(&entry.source.path);
    let relative = dirs
        .iter()
        .find_map(|dir| source.strip_prefix(dir).ok())
        .unwrap_or(source);
    let alias = entry.aliases.first().map_or("", String::as_str);
    format!("{alias}@{}", relative.display())
}

fn read_sidecar(path: &Path) -> UsageFile {
    let is_file = fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file());
    if !is_file {
        return UsageFile::default();
    }
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<UsageFile>(&bytes).ok())
        .filter(|file| file.version == VERSION)
        .unwrap_or_default()
}

fn write_sidecar(path: &Path, file: &UsageFile) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.file_type().is_file()
    {
        return Err(format!(
            "USAGE_WRITE_REFUSED: not a regular file: {}",
            path.display()
        ));
    }
    let mut rendered = serde_json::to_vec_pretty(file)
        .map_err(|error| format!("USAGE_WRITE_FAILED: cannot render usage: {error}"))?;
    rendered.push(b'\n');
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("usage");
    let temporary = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    // remove_file unlinks a planted symlink instead of following it.
    let _ = fs::remove_file(&temporary);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options
        .open(&temporary)
        .and_then(|mut handle| handle.write_all(&rendered).and_then(|()| handle.sync_all()))
        .and_then(|()| fs::rename(&temporary, path));
    result.map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "USAGE_WRITE_FAILED: cannot write {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{Provenance, SourceIdentity};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn directory() -> PathBuf {
        let directory = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "sshx-usage-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn entry(root: &Path, id: Option<&str>, alias: &str, source: &Path) -> HostEntry {
        let path = source.to_string_lossy().into_owned();
        HostEntry {
            id: id.map_or_else(|| format!("{path}#0-10"), str::to_owned),
            aliases: vec![alias.to_string(), "second".to_string()],
            source: SourceIdentity {
                path,
                byte_start: 0,
                byte_end: 10,
                line_start: 1,
                line_end: 2,
            },
            destination: None,
            provenance: vec![Provenance {
                paths: vec![root.to_string_lossy().into_owned()],
                scope: "personal".to_string(),
                project: None,
            }],
            scopes: vec!["personal".to_string()],
            projects: Vec::new(),
        }
    }

    fn forward(remote: u16, local: u16) -> UsedForward {
        UsedForward {
            remote,
            local,
            declared: true,
        }
    }

    #[test]
    fn sidecar_sits_beside_root_named_after_it() {
        assert_eq!(
            sidecar_path(Path::new("/a/b/config")),
            Path::new("/a/b/.config.sshx-usage.json")
        );
    }

    #[test]
    fn key_prefers_persistent_id_then_alias_at_relative_path() {
        let dir = directory();
        let root = dir.join("config");
        let with_id = entry(&root, Some("abc-123"), "web", &root);
        assert_eq!(host_key(&root, &with_id), "abc-123");

        let included = dir.join("conf.d/web.conf");
        assert_eq!(
            host_key(&root, &entry(&root, None, "web", &included)),
            "web@conf.d/web.conf"
        );
        let outside = Path::new("/elsewhere/other.conf");
        assert_eq!(
            host_key(&root, &entry(&root, None, "web", outside)),
            "web@/elsewhere/other.conf"
        );
    }

    #[test]
    fn missing_and_corrupt_sidecars_load_empty() {
        let dir = directory();
        let root = dir.join("config");
        fs::write(&root, "Host a\n").unwrap();
        let roots = [DiscoveryRoot::new(&root, "personal", None)];
        let host = entry(&root, Some("id"), "a", &root);
        assert!(UsageIndex::load(&roots).get(&host).is_none());

        fs::write(sidecar_path(&root), "{ not json").unwrap();
        assert!(UsageIndex::load(&roots).get(&host).is_none());
        fs::write(
            sidecar_path(&root),
            r#"{"version":99,"hosts":{"id":{"last_used":1,"count":1,"forwards":[]}}}"#,
        )
        .unwrap();
        assert!(UsageIndex::load(&roots).get(&host).is_none());
    }

    #[test]
    fn record_round_trips_increments_and_replaces_forwards() {
        let dir = directory();
        let root = dir.join("config");
        fs::write(&root, "Host a\n").unwrap();
        let roots = [DiscoveryRoot::new(&root, "personal", None)];
        let host = entry(&root, Some("id"), "a", &root);

        record(&host, &[forward(5432, 5432)], 100).unwrap();
        record(&host, &[forward(80, 3000)], 200).unwrap();

        let index = UsageIndex::load(&roots);
        let usage = index.get(&host).unwrap();
        assert_eq!(usage.count, 2);
        assert_eq!(usage.last_used, 200);
        assert_eq!(usage.forwards, vec![forward(80, 3000)]);
        assert_eq!(index.frequency(&host), 2);
        assert_eq!(index.last_used(&host), Some(200));

        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(sidecar_path(&root)).unwrap()).unwrap();
        assert_eq!(document["version"], 1);
        assert_eq!(document["hosts"]["id"]["forwards"][0]["declared"], true);
    }

    #[test]
    fn rekey_moves_usage_and_ignores_unknown_keys() {
        let dir = directory();
        let root = dir.join("config");
        fs::write(&root, "Host a\n").unwrap();
        let old = entry(&root, None, "a", &root);
        record(&old, &[], 5).unwrap();

        rekey(&root, "nope", "new").unwrap();
        rekey(&root, &host_key(&root, &old), "new-id").unwrap();

        let renamed = entry(&root, Some("new-id"), "a", &root);
        let index = UsageIndex::load(&[DiscoveryRoot::new(&root, "personal", None)]);
        assert_eq!(index.get(&renamed).unwrap().count, 1);
        assert!(index.get(&old).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn sidecar_is_private_and_symlinks_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = directory();
        let root = dir.join("config");
        fs::write(&root, "Host a\n").unwrap();
        let host = entry(&root, Some("id"), "a", &root);
        record(&host, &[], 1).unwrap();
        let mode = fs::metadata(sidecar_path(&root))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        let other = dir.join("other");
        let target = dir.join("target");
        fs::write(&target, "keep").unwrap();
        let other_root = dir.join("other-config");
        fs::write(&other_root, "Host b\n").unwrap();
        std::os::unix::fs::symlink(&target, sidecar_path(&other_root)).unwrap();
        let linked = entry(&other_root, Some("id"), "b", &other);
        assert!(record(&linked, &[], 1).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn unwritable_root_directory_reports_error_without_sidecar() {
        use std::os::unix::fs::PermissionsExt;
        let dir = directory();
        let root = dir.join("config");
        fs::write(&root, "Host a\n").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let result = record(&entry(&root, Some("id"), "a", &root), &[], 1);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        // Running as root bypasses directory modes; only assert the invariant.
        if result.is_err() {
            assert!(!sidecar_path(&root).exists());
        }
    }
}
