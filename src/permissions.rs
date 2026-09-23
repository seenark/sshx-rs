//! Shared permission-repair eligibility contract (spec: interactive CLI UX,
//! permission repair). One policy for doctor-planned repairs and runtime
//! blocking repairs; never a second ownership or privacy policy.

use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Apply one planned repair: re-check eligibility at apply time, set the exact
/// `0600`/`0700` mode, and verify. Never changes ownership or replaces paths.
pub fn apply(candidate: &RepairCandidate) -> RepairResult {
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
    let set = fs::set_permissions(
        &candidate.path,
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::Permissions::from_mode(target_mode)
        },
        #[cfg(not(unix))]
        {
            fs::permissions(&candidate.path).unwrap_or_default()
        },
    );
    if let Err(error) = set {
        return denied(
            RepairOutcome::Failed,
            format!("cannot chmod {}: {error}", candidate.path.display()),
        );
    }
    match assess(&candidate.path, candidate.target) {
        Ok(mode) if mode == target_mode => RepairResult {
            candidate: candidate.clone(),
            outcome: RepairOutcome::Fixed,
            detail: format!("{:o} -> {:o}", current, target_mode),
        },
        Ok(mode) => denied(
            RepairOutcome::Failed,
            format!(
                "{} is {:o}, expected {:o}",
                candidate.path.display(),
                mode,
                target_mode
            ),
        ),
        Err(reason) => denied(RepairOutcome::Failed, reason),
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
