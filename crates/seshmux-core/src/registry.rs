use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(rename = "worktree", default)]
    entries: Vec<RegistryEntry>,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("failed to read registry at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse registry at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to write registry at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize registry: {0}")]
    Serialize(toml::ser::Error),
    #[error("worktree registry already contains name '{name}'")]
    DuplicateName { name: String },
    #[error("worktree registry already contains path '{path}'")]
    DuplicatePath { path: String },
}

pub fn registry_path(repo_root: &Path) -> PathBuf {
    repo_root.join("worktrees").join("worktree.toml")
}

pub fn load_registry(repo_root: &Path) -> Result<Vec<RegistryEntry>, RegistryError> {
    let path = registry_path(repo_root);

    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(&path).map_err(|source| RegistryError::Read {
        path: path.clone(),
        source,
    })?;

    let parsed: RegistryFile = toml::from_str(&raw).map_err(|source| RegistryError::Parse {
        path: path.clone(),
        source,
    })?;

    Ok(parsed.entries)
}

pub fn ensure_entry_available(
    repo_root: &Path,
    name: &str,
    path: &Path,
) -> Result<(), RegistryError> {
    let entries = load_registry(repo_root)?;
    let path_value = path.to_string_lossy().to_string();

    for entry in entries {
        if entry.name == name {
            return Err(RegistryError::DuplicateName {
                name: name.to_string(),
            });
        }

        if entry.path == path_value {
            return Err(RegistryError::DuplicatePath {
                path: path_value.clone(),
            });
        }
    }

    Ok(())
}

pub fn insert_unique_entry(repo_root: &Path, entry: RegistryEntry) -> Result<(), RegistryError> {
    let mut entries = load_registry(repo_root)?;

    for existing in &entries {
        if existing.name == entry.name {
            return Err(RegistryError::DuplicateName {
                name: entry.name.clone(),
            });
        }

        if existing.path == entry.path {
            return Err(RegistryError::DuplicatePath {
                path: entry.path.clone(),
            });
        }
    }

    entries.push(entry);
    write_registry(repo_root, entries)
}

fn write_registry(repo_root: &Path, entries: Vec<RegistryEntry>) -> Result<(), RegistryError> {
    let path = registry_path(repo_root);
    let parent = path.parent().expect("worktree registry path has parent");
    fs::create_dir_all(parent).map_err(|source| RegistryError::Write {
        path: parent.to_path_buf(),
        source,
    })?;

    let payload = RegistryFile { entries };
    let serialized = toml::to_string(&payload).map_err(RegistryError::Serialize)?;

    let temp_path = path.with_extension("toml.tmp");

    fs::write(&temp_path, serialized).map_err(|source| RegistryError::Write {
        path: temp_path.clone(),
        source,
    })?;

    fs::rename(&temp_path, &path).map_err(|source| RegistryError::Write {
        path: path.clone(),
        source,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_unique_entry_rejects_duplicate_name() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");

        insert_unique_entry(
            repo_root,
            RegistryEntry {
                name: "w1".to_string(),
                path: repo_root
                    .join("worktrees")
                    .join("w1")
                    .to_string_lossy()
                    .to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
        )
        .expect("first insert");

        let error = insert_unique_entry(
            repo_root,
            RegistryEntry {
                name: "w1".to_string(),
                path: repo_root
                    .join("worktrees")
                    .join("w2")
                    .to_string_lossy()
                    .to_string(),
                created_at: "2026-01-01T00:00:01Z".to_string(),
            },
        )
        .expect_err("duplicate should fail");

        assert!(matches!(error, RegistryError::DuplicateName { .. }));
    }
}
