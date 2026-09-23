//! Shared permission-repair eligibility contract (spec: interactive CLI UX,
//! permission repair). One policy for doctor-planned repairs and runtime
//! blocking repairs; never a second ownership or privacy policy.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub const PRIVATE_FILE_MODE: u32 = 0o600;
pub const PRIVATE_DIR_MODE: u32 = 0o700;

#[derive(Clone, Debug, Serialize)]
pub struct RepairCandidate {
    /// Doctor location kind, e.g. `app_settings` or `password_file`.
    pub kind: String,
    pub path: PathBuf,
    /// True repairs to a file (`0600`); false repairs to a directory (`0700`).
    pub file: bool,
    pub current_mode: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RepairResult {
    pub candidate: RepairCandidate,
    /// `fixed`, `skipped`, or `failed`.
    pub outcome: String,
    pub detail: String,
}

/// Assess the shared eligibility contract for `path`.
///
/// Eligible requires: path exists, is the expected regular-file or directory
/// type, is not a symlink, and is owned by the current effective user.
/// Returns the current mode when eligible; otherwise the disqualifying reason.
pub fn assess(path: &Path, file: bool) -> Result<u32, String> {
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
    if file && !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if !file && !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32 {
            return Err(format!("{} is owned by another user", path.display()));
        }
        if file && metadata.nlink() > 1 {
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
    let target_mode = if candidate.file {
        PRIVATE_FILE_MODE
    } else {
        PRIVATE_DIR_MODE
    };
    let denied = |outcome: &str, detail: String| RepairResult {
        candidate: candidate.clone(),
        outcome: outcome.to_string(),
        detail,
    };
    let current = match assess(&candidate.path, candidate.file) {
        Ok(mode) => mode,
        Err(reason) => return denied("skipped", reason),
    };
    if current == target_mode {
        return RepairResult {
            candidate: candidate.clone(),
            outcome: "fixed".to_string(),
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
            "failed",
            format!("cannot chmod {}: {error}", candidate.path.display()),
        );
    }
    match assess(&candidate.path, candidate.file) {
        Ok(mode) if mode == target_mode => RepairResult {
            candidate: candidate.clone(),
            outcome: "fixed".to_string(),
            detail: format!("{:o} -> {:o}", current, target_mode),
        },
        Ok(mode) => denied(
            "failed",
            format!(
                "{} is {:o}, expected {:o}",
                candidate.path.display(),
                mode,
                target_mode
            ),
        ),
        Err(reason) => denied("failed", reason),
    }
}

/// Apply every planned repair even when individual repairs fail, reporting
/// `fixed`, `skipped`, or `failed` for each planned path.
pub fn apply_all(candidates: &[RepairCandidate]) -> Vec<RepairResult> {
    candidates.iter().map(apply).collect()
}

/// Shell-quoted manual repair command for error messages.
pub fn manual_chmod_command(path: &Path, file: bool) -> String {
    let mode = if file { 600 } else { 700 };
    let quoted = path.display().to_string().replace('\'', "'\\''");
    format!("chmod {mode} '{quoted}'")
}
