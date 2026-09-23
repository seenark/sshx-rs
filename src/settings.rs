use crate::discovery::DiscoveryRoot;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const SETTINGS_VERSION: u8 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegisteredRoot {
    pub scope: String,
    pub path: PathBuf,
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SettingsFile {
    version: u8,
    roots: Vec<RegisteredRoot>,
}

pub fn settings_path(home: &Path) -> PathBuf {
    home.join(".config/sshx/config.json")
}

pub fn load(home: &Path) -> Result<Vec<RegisteredRoot>, String> {
    let path = settings_path(home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let settings: SettingsFile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if settings.version != SETTINGS_VERSION {
        return Err(format!(
            "unsupported sshx settings version {} in {}",
            settings.version,
            path.display()
        ));
    }
    Ok(settings.roots)
}

pub fn save(home: &Path, roots: &[RegisteredRoot]) -> Result<(), String> {
    let path = settings_path(home);
    let parent = path
        .parent()
        .ok_or_else(|| "sshx settings path has no parent".to_string())?;
    private_directory(parent)?;
    let settings = SettingsFile {
        version: SETTINGS_VERSION,
        roots: roots.to_vec(),
    };
    let mut rendered = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("cannot render sshx settings: {error}"))?;
    rendered.push(b'\n');
    let temporary = path.with_extension("json.tmp");
    private_file(&temporary, &rendered)?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

/// Create the sshx-owned settings directory with mode `0700` regardless of
/// the process umask. Shared parents such as `~/.config` keep their mode.
fn private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot set private mode on {}: {error}", path.display()))?;
    }
    Ok(())
}

/// Write a new file with mode `0600` regardless of the process umask.
fn private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        use std::io::Write;
        file.write_all(contents)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot set private mode on {}: {error}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(())
}

pub fn auto_detect(home: &Path) -> Vec<RegisteredRoot> {
    [
        ("personal", home.join(".ssh/config")),
        ("work", home.join(".private-key/private-key/config")),
    ]
    .into_iter()
    .filter_map(|(scope, path)| {
        path.is_file().then_some(RegisteredRoot {
            scope: scope.to_string(),
            path,
            project: None,
        })
    })
    .collect()
}

pub fn normalize_path(path: &Path, home: &Path) -> PathBuf {
    let path = path.to_string_lossy();
    let path = if path == "~" {
        home.to_path_buf()
    } else if let Some(suffix) = path.strip_prefix("~/") {
        home.join(suffix)
    } else {
        PathBuf::from(path.as_ref())
    };
    if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub fn merge(roots: &mut Vec<RegisteredRoot>, additions: impl IntoIterator<Item = RegisteredRoot>) {
    for addition in additions {
        if !roots.iter().any(|root| root == &addition) {
            roots.push(addition);
        }
    }
}

pub fn discovery_roots(roots: &[RegisteredRoot]) -> Vec<DiscoveryRoot> {
    roots
        .iter()
        .map(|root| DiscoveryRoot::new(root.path.clone(), root.scope.clone(), root.project.clone()))
        .collect()
}
