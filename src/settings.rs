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
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let settings = SettingsFile {
        version: SETTINGS_VERSION,
        roots: roots.to_vec(),
    };
    let mut rendered = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("cannot render sshx settings: {error}"))?;
    rendered.push(b'\n');
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, rendered)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
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
