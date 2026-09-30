//! Shared permission-repair eligibility contract (spec: interactive CLI UX,
//! permission repair). One policy for doctor-planned repairs and runtime
//! blocking repairs; never a second ownership or privacy policy.

use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::ffi::CString;
use std::io;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(target_os = "linux")]
const DIRECTORY_ACCESS: i32 = libc::O_PATH;
#[cfg(target_vendor = "apple")]
const DIRECTORY_ACCESS: i32 = libc::O_SEARCH;
#[cfg(all(unix, not(any(target_os = "linux", target_vendor = "apple"))))]
const DIRECTORY_ACCESS: i32 = libc::O_RDONLY;

#[cfg(unix)]
fn open_directory_at(dir: i32, name: &std::ffi::CStr) -> io::Result<OwnedFd> {
    let fd = unsafe {
        libc::openat(
            dir,
            name.as_ptr(),
            DIRECTORY_ACCESS | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_repair_target(path: &Path, target: PermissionTarget, mode: u32) -> io::Result<u32> {
    let start = CString::new(if path.is_absolute() { "/" } else { "." }).unwrap();
    let fd = unsafe {
        libc::open(
            start.as_ptr(),
            DIRECTORY_ACCESS | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let is_final = components.peek().is_none();
        match component {
            Component::RootDir | Component::CurDir if !is_final => {}
            Component::RootDir | Component::CurDir => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "path has no filename"));
            }
            Component::ParentDir if !is_final => {
                let parent = CString::new("..").unwrap();
                dir = open_directory_at(dir.as_raw_fd(), &parent)?;
            }
            Component::Normal(part) if !is_final => {
                let part = CString::new(part.as_bytes())?;
                dir = open_directory_at(dir.as_raw_fd(), &part)?;
            }
            Component::ParentDir | Component::Normal(_) => {
                let name = CString::new(component.as_os_str().as_bytes())?;
                return repair_opened_target(&dir, &name, path, target, mode);
            }
            Component::Prefix(_) => unreachable!("prefix components are not Unix paths"),
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidInput, "path has no filename"))
}

#[cfg(unix)]
fn repair_opened_target(
    dir: &OwnedFd,
    name: &CString,
    path: &Path,
    target: PermissionTarget,
    mode: u32,
) -> io::Result<u32> {
    use std::os::unix::fs::PermissionsExt;

    #[cfg(target_os = "linux")]
    let target_file = open_target_fd(
        dir,
        name,
        libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
    )?;
    #[cfg(target_os = "linux")]
    let metadata = target_file.metadata()?;
    #[cfg(not(target_os = "linux"))]
    let metadata = fs::symlink_metadata(path)?;
    check_metadata(&metadata, target)?;
    let current = metadata.permissions().mode() & 0o7777;
    if current == mode {
        return Ok(current);
    }

    #[cfg(target_os = "linux")]
    let mut changed = unsafe {
        libc::syscall(
            libc::SYS_fchmodat2,
            target_file.as_raw_fd(),
            b"\0".as_ptr(),
            mode,
            libc::AT_EMPTY_PATH,
        )
    };
    #[cfg(target_os = "linux")]
    if changed < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ENOSYS) {
        // Older kernels lack fchmodat2; procfs still names this held inode,
        // even if the original path is replaced before chmod.
        let held_path = CString::new(format!("/proc/self/fd/{}", target_file.as_raw_fd())).unwrap();
        changed = unsafe { libc::fchmodat(libc::AT_FDCWD, held_path.as_ptr(), mode, 0) }.into();
    }
    // Darwin cannot open mode-000 targets for fchmod; no-follow fchmodat avoids
    // changing a symlink referent if the final entry races after inspection.
    #[cfg(not(target_os = "linux"))]
    let changed = unsafe {
        libc::fchmodat(
            dir.as_raw_fd(),
            name.as_ptr(),
            mode as libc::mode_t,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if changed < 0 {
        return Err(io::Error::last_os_error());
    }

    #[cfg(target_os = "linux")]
    let verified = target_file.metadata()?;
    #[cfg(not(target_os = "linux"))]
    let verified = fs::symlink_metadata(path)?;
    check_metadata(&verified, target)?;
    #[cfg(not(target_os = "linux"))]
    if metadata.dev() != verified.dev() || metadata.ino() != verified.ino() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "target changed during repair"));
    }
    let verified_mode = verified.permissions().mode() & 0o7777;
    if verified_mode != mode {
        return Err(io::Error::other(format!(
            "mode is {:o}, expected {:o}",
            verified_mode, mode
        )));
    }
    Ok(current)
}

#[cfg(target_os = "linux")]
fn open_target_fd(dir: &OwnedFd, name: &CString, flags: i32) -> io::Result<std::fs::File> {
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
    }
}

#[cfg(unix)]
fn check_metadata(metadata: &fs::Metadata, target: PermissionTarget) -> io::Result<()> {
    if metadata.file_type().is_symlink()
        || (target.is_file() && !metadata.is_file())
        || (!target.is_file() && !metadata.is_dir())
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "target type changed"));
    }
    if metadata.uid() != unsafe { libc::geteuid() } as u32 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "target is owned by another user",
        ));
    }
    if target.is_file() && metadata.nlink() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target is a shared file",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn open_repair_target(_path: &Path, _target: PermissionTarget, _mode: u32) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "race-resistant permission repair is unavailable on this platform",
    ))
}


pub const PRIVATE_FILE_MODE: u32 = 0o600;
pub const PRIVATE_DIR_MODE: u32 = 0o700;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionTarget {
    File,
    Directory,
}

impl PermissionTarget {
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File)
    }

    pub const fn private_mode(self) -> u32 {
        match self {
            Self::File => PRIVATE_FILE_MODE,
            Self::Directory => PRIVATE_DIR_MODE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RepairOutcome {
    Fixed,
    Skipped,
    Failed,
}

impl fmt::Display for RepairOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Fixed => "fixed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RepairCandidate {
    /// Doctor location kind, e.g. `app_settings` or `password_file`.
    pub kind: String,
    pub path: PathBuf,
    pub target: PermissionTarget,
    pub current_mode: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RepairResult {
    pub candidate: RepairCandidate,
    pub outcome: RepairOutcome,
    pub detail: String,
}

/// Assess the shared eligibility contract for `path`.
///
/// Eligible requires: path exists, is the expected regular-file or directory
/// type, is not a symlink, and is owned by the current effective user.
/// Returns the current mode when eligible; otherwise the disqualifying reason.
pub fn assess(path: &Path, target: PermissionTarget) -> Result<u32, String> {
    if let Some(component) = symlink_component(path) {
        return Err(format!(
            "{} contains symlink component {}",
            path.display(),
            component.display()
        ));
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("{} is a symlink", path.display()));
    }
    if target.is_file() && !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if !target.is_file() && !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32 {
            return Err(format!("{} is owned by another user", path.display()));
        }
        if target.is_file() && metadata.nlink() > 1 {
            return Err(format!("{} is a shared file", path.display()));
        }
        Ok(metadata.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Ok(0)
    }
}

fn symlink_component(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        fs::symlink_metadata(ancestor)
            .ok()
            .filter(|metadata| metadata.file_type().is_symlink())
            .map(|_| ancestor.to_path_buf())
    })
}

/// Re-check eligibility, then repair through no-follow directory descriptors.
/// Never changes ownership or replaces paths.
pub fn apply(candidate: &RepairCandidate) -> RepairResult {
    apply_with(candidate, || {})
}

fn apply_with(candidate: &RepairCandidate, before_repair: impl FnOnce()) -> RepairResult {
    let target_mode = candidate.target.private_mode();
    let denied = |outcome: RepairOutcome, detail: String| RepairResult {
        candidate: candidate.clone(),
        outcome,
        detail,
    };
    let current = match assess(&candidate.path, candidate.target) {
        Ok(mode) => mode,
        Err(reason) => return denied(RepairOutcome::Skipped, reason),
    };
    if current == target_mode {
        return RepairResult {
            candidate: candidate.clone(),
            outcome: RepairOutcome::Fixed,
            detail: format!("already {:o}", target_mode),
        };
    }
    before_repair();
    let repaired_from = match open_repair_target(&candidate.path, candidate.target, target_mode) {
        Ok(mode) => mode,
        Err(error) => {
            let outcome = if assess(&candidate.path, candidate.target).is_err() {
                RepairOutcome::Skipped
            } else {
                RepairOutcome::Failed
            };
            return denied(
                outcome,
                format!("cannot safely chmod {}: {error}", candidate.path.display()),
            );
        }
    };
    RepairResult {
        candidate: candidate.clone(),
        outcome: RepairOutcome::Fixed,
        detail: format!("{:o} -> {:o}", repaired_from, target_mode),
    }
}

/// Apply every planned repair even when individual repairs fail, reporting
/// `fixed`, `skipped`, or `failed` for each planned path.
pub fn apply_all(candidates: &[RepairCandidate]) -> Vec<RepairResult> {
    candidates.iter().map(apply).collect()
}

/// Shell-quoted manual repair command for error messages.
pub fn manual_chmod_command(path: &Path, target: PermissionTarget) -> String {
    let mode = target.private_mode();
    let quoted = path.display().to_string().replace('\'', "'\\''");
    format!("chmod {:o} '{quoted}'", mode)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn apply_rejects_symlink_replacement_between_check_and_repair() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sshx-permission-race-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let path = root.join("secret");
        let outside = root.join("outside");
        fs::write(&path, "private").unwrap();
        fs::write(&outside, "outside secret").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
        let candidate = RepairCandidate {
            kind: "password_file".to_string(),
            path: path.clone(),
            target: PermissionTarget::File,
            current_mode: 0o644,
            reason: "test".to_string(),
        };

        let result = apply_with(&candidate, || {
            fs::remove_file(&path).unwrap();
            symlink(&outside, &path).unwrap();
        });

        assert_eq!(result.outcome, RepairOutcome::Skipped);
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        assert_eq!(fs::metadata(&outside).unwrap().permissions().mode() & 0o7777, 0o644);
        assert_eq!(fs::read_to_string(&outside).unwrap(), "outside secret");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_repairs_private_mode_and_skips_shared_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sshx-permission-apply-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let path = root.join("secret");
        fs::write(&path, "private").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let candidate = RepairCandidate {
            kind: "password_file".to_string(),
            path: path.clone(),
            target: PermissionTarget::File,
            current_mode: 0o644,
            reason: "test".to_string(),
        };
        let result = apply(&candidate);
        assert_eq!(result.outcome, RepairOutcome::Fixed, "{}", result.detail);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o7777, 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let result = apply(&candidate);
        assert_eq!(result.outcome, RepairOutcome::Fixed, "{}", result.detail);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o7777, 0o600);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::hard_link(&path, root.join("shared")).unwrap();
        let result = apply(&candidate);
        assert_eq!(result.outcome, RepairOutcome::Skipped);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o7777, 0o644);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_repairs_file_below_search_only_directory() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sshx-search-only-repair-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let path = root.join("secret");
        fs::write(&path, "private").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let candidate = RepairCandidate {
            kind: "password_file".to_string(),
            path: path.clone(),
            target: PermissionTarget::File,
            current_mode: 0o644,
            reason: "test".to_string(),
        };
        fs::set_permissions(&root, fs::Permissions::from_mode(0o100)).unwrap();
        let result = apply(&candidate);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result.outcome, RepairOutcome::Fixed, "{}", result.detail);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o7777, 0o600);
        fs::remove_dir_all(root).unwrap();
    }
}
