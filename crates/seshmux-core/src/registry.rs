use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const REGISTRY_VERSION: i64 = 1;
const DEFAULT_ALWAYS_SKIP_BUCKETS: &[&str] = &[
    "target",
    "node_modules",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "dist",
    "build",
    "out",
    "coverage",
    ".cache",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".venv",
    "venv",
    "vendor",
    "vendor/bundle",
    ".gradle",
    "DerivedData",
    "Pods",
    "Carthage",
    ".terraform",
    ".serverless",
    "cdk.out",
    ".dart_tool",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RegistryFile {
    version: i64,
    settings: RegistrySettings,
    #[serde(rename = "worktree")]
    entries: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RegistrySettings {
    extras: RegistryExtrasSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RegistryExtrasSettings {
    #[serde(default)]
    always_skip_buckets: Option<Vec<String>>,
}

impl Default for RegistryFile {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            settings: RegistrySettings {
                extras: RegistryExtrasSettings {
                    always_skip_buckets: None,
                },
            },
            entries: Vec::new(),
        }
    }
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
    #[error("{message}")]
    InvalidSchema { message: String },
    #[error("worktree registry already contains name '{name}'")]
    DuplicateName { name: String },
    #[error("worktree registry already contains path '{path}'")]
    DuplicatePath { path: String },
}

pub fn registry_path(repo_root: &Path) -> PathBuf {
    repo_root.join("worktrees").join("worktree.toml")
}

pub fn default_always_skip_buckets() -> BTreeSet<String> {
    DEFAULT_ALWAYS_SKIP_BUCKETS
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlwaysSkipBucketsLoad {
    pub buckets: BTreeSet<String>,
    pub configured_buckets: BTreeSet<String>,
    pub registry_missing: bool,
}

pub fn load_registry(repo_root: &Path) -> Result<Vec<RegistryEntry>, RegistryError> {
    Ok(load_registry_file(repo_root)?.entries)
}

pub fn load_always_skip_buckets_for_indexing(
    repo_root: &Path,
) -> Result<AlwaysSkipBucketsLoad, RegistryError> {
    let path = registry_path(repo_root);
    if !path.exists() {
        return Ok(AlwaysSkipBucketsLoad {
            buckets: default_always_skip_buckets(),
            configured_buckets: BTreeSet::new(),
            registry_missing: true,
        });
    }

    let registry = load_registry_file(repo_root)?;
    let configured_buckets = registry
        .settings
        .extras
        .always_skip_buckets
        .clone()
        .map(normalize_bucket_values);
    let buckets = configured_buckets
        .clone()
        .unwrap_or_else(default_always_skip_buckets);

    Ok(AlwaysSkipBucketsLoad {
        buckets,
        configured_buckets: configured_buckets.unwrap_or_default(),
        registry_missing: false,
    })
}

pub fn load_always_skip_buckets(repo_root: &Path) -> Result<BTreeSet<String>, RegistryError> {
    let mut registry = load_registry_file(repo_root)?;

    if let Some(values) = registry.settings.extras.always_skip_buckets.clone() {
        return Ok(normalize_bucket_values(values));
    }

    let seeded = default_always_skip_buckets();
    registry.settings.extras.always_skip_buckets = Some(seeded.iter().cloned().collect());
    write_registry_file(repo_root, &registry)?;
    Ok(seeded)
}

pub fn save_always_skip_buckets(
    repo_root: &Path,
    buckets: &BTreeSet<String>,
) -> Result<(), RegistryError> {
    let mut registry = load_registry_file(repo_root)?;
    registry.settings.extras.always_skip_buckets = Some(buckets.iter().cloned().collect());
    write_registry_file(repo_root, &registry)
}

pub fn ensure_entry_available(
    repo_root: &Path,
    name: &str,
    path: &Path,
) -> Result<(), RegistryError> {
    let entries = load_registry(repo_root)?;
    let path_value = path.to_string_lossy();
    ensure_unique_entry(&entries, name, path_value.as_ref())
}

pub fn find_entry_by_name(
    repo_root: &Path,
    name: &str,
) -> Result<Option<RegistryEntry>, RegistryError> {
    let entries = load_registry(repo_root)?;
    Ok(entries.into_iter().find(|entry| entry.name == name))
}

pub fn remove_entry_by_name(
    repo_root: &Path,
    name: &str,
) -> Result<Option<RegistryEntry>, RegistryError> {
    let mut registry = load_registry_file(repo_root)?;
    let index = registry.entries.iter().position(|entry| entry.name == name);

    let Some(index) = index else {
        return Ok(None);
    };

    let removed = registry.entries.remove(index);
    write_registry_file(repo_root, &registry)?;
    Ok(Some(removed))
}

pub fn insert_unique_entry(repo_root: &Path, entry: RegistryEntry) -> Result<(), RegistryError> {
    let mut registry = load_registry_file(repo_root)?;
    ensure_unique_entry(&registry.entries, &entry.name, &entry.path)?;

    registry.entries.push(entry);
    write_registry_file(repo_root, &registry)
}

fn ensure_unique_entry(
    entries: &[RegistryEntry],
    name: &str,
    path: &str,
) -> Result<(), RegistryError> {
    if entries.iter().any(|entry| entry.name == name) {
        return Err(RegistryError::DuplicateName {
            name: name.to_string(),
        });
    }

    if entries.iter().any(|entry| entry.path == path) {
        return Err(RegistryError::DuplicatePath {
            path: path.to_string(),
        });
    }

    Ok(())
}

fn load_registry_file(repo_root: &Path) -> Result<RegistryFile, RegistryError> {
    let path = registry_path(repo_root);
    if !path.exists() {
        return Ok(RegistryFile::default());
    }

    let raw = fs::read_to_string(&path).map_err(|source| RegistryError::Read {
        path: path.clone(),
        source,
    })?;

    let parsed_value: toml::Value =
        toml::from_str(&raw).map_err(|source| RegistryError::Parse {
            path: path.clone(),
            source,
        })?;

    validate_registry_schema(&parsed_value)?;

    let parsed: RegistryFile = parsed_value
        .try_into()
        .map_err(|source| RegistryError::Parse {
            path: path.clone(),
            source,
        })?;

    if parsed.version != REGISTRY_VERSION {
        return Err(schema_error(format!(
            "invalid worktree registry schema: unsupported version (expected {REGISTRY_VERSION}, found {})",
            parsed.version
        )));
    }

    Ok(parsed)
}

fn validate_registry_schema(value: &toml::Value) -> Result<(), RegistryError> {
    let Some(root) = value.as_table() else {
        return Err(schema_error(
            "invalid worktree registry schema: missing required top-level field 'version'"
                .to_string(),
        ));
    };

    let Some(version) = root.get("version") else {
        return Err(schema_error(
            "invalid worktree registry schema: missing required top-level field 'version'"
                .to_string(),
        ));
    };

    match version.as_integer() {
        Some(current) if current == REGISTRY_VERSION => {}
        Some(current) => {
            return Err(schema_error(format!(
                "invalid worktree registry schema: unsupported version (expected {REGISTRY_VERSION}, found {current})"
            )));
        }
        None => {
            return Err(schema_error(
                "invalid worktree registry schema: unsupported version (expected integer)"
                    .to_string(),
            ));
        }
    }

    let has_settings_extras = root
        .get("settings")
        .and_then(toml::Value::as_table)
        .and_then(|settings| settings.get("extras"))
        .and_then(toml::Value::as_table)
        .is_some();
    if !has_settings_extras {
        return Err(schema_error(
            "invalid worktree registry schema: missing required section [settings.extras]"
                .to_string(),
        ));
    }

    let has_worktree_entries = root
        .get("worktree")
        .and_then(toml::Value::as_array)
        .is_some();
    if !has_worktree_entries {
        return Err(schema_error(
            "invalid worktree registry schema: missing required [[worktree]] entries section"
                .to_string(),
        ));
    }

    Ok(())
}

fn write_registry_file(repo_root: &Path, registry: &RegistryFile) -> Result<(), RegistryError> {
    let path = registry_path(repo_root);
    let parent = path.parent().expect("worktree registry path has parent");
    fs::create_dir_all(parent).map_err(|source| RegistryError::Write {
        path: parent.to_path_buf(),
        source,
    })?;

    let mut root = toml::map::Map::<String, toml::Value>::new();
    root.insert(
        "version".to_string(),
        toml::Value::Integer(registry.version),
    );

    let mut extras = toml::map::Map::<String, toml::Value>::new();
    if let Some(values) = &registry.settings.extras.always_skip_buckets {
        extras.insert(
            "always_skip_buckets".to_string(),
            toml::Value::Array(
                values
                    .iter()
                    .map(|value| toml::Value::String(value.clone()))
                    .collect(),
            ),
        );
    }

    let mut settings = toml::map::Map::<String, toml::Value>::new();
    settings.insert("extras".to_string(), toml::Value::Table(extras));
    root.insert("settings".to_string(), toml::Value::Table(settings));

    let entries = registry
        .entries
        .iter()
        .map(|entry| {
            let mut table = toml::map::Map::<String, toml::Value>::new();
            table.insert("name".to_string(), toml::Value::String(entry.name.clone()));
            table.insert("path".to_string(), toml::Value::String(entry.path.clone()));
            table.insert(
                "created_at".to_string(),
                toml::Value::String(entry.created_at.clone()),
            );
            toml::Value::Table(table)
        })
        .collect();
    root.insert("worktree".to_string(), toml::Value::Array(entries));

    let serialized =
        toml::to_string(&toml::Value::Table(root)).map_err(RegistryError::Serialize)?;
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

fn normalize_bucket_values(values: Vec<String>) -> BTreeSet<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn schema_error(message: String) -> RegistryError {
    RegistryError::InvalidSchema { message }
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

    #[test]
    fn registry_settings_round_trip_preserves_worktree_entries() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();

        let entry = RegistryEntry {
            name: "w1".to_string(),
            path: repo_root
                .join("worktrees")
                .join("w1")
                .to_string_lossy()
                .to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        insert_unique_entry(repo_root, entry.clone()).expect("insert");

        let buckets = BTreeSet::from(["target".to_string(), "node_modules".to_string()]);
        save_always_skip_buckets(repo_root, &buckets).expect("save buckets");

        let loaded_entries = load_registry(repo_root).expect("load entries");
        assert_eq!(loaded_entries, vec![entry]);
        assert_eq!(
            load_always_skip_buckets(repo_root).expect("load buckets"),
            buckets
        );
    }

    #[test]
    fn load_always_skip_buckets_for_indexing_uses_defaults_without_creating_registry_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();

        let loaded = load_always_skip_buckets_for_indexing(repo_root).expect("load defaults");

        assert!(loaded.registry_missing);
        assert_eq!(loaded.buckets, default_always_skip_buckets());
        assert!(loaded.configured_buckets.is_empty());
        assert!(!registry_path(repo_root).exists());
    }

    #[test]
    fn load_always_skip_buckets_for_indexing_reads_existing_registry_without_writing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 1\nworktree=[]\n[settings.extras]\n",
        )
        .expect("write registry");

        let loaded = load_always_skip_buckets_for_indexing(repo_root).expect("load existing");
        let raw = fs::read_to_string(registry_path(repo_root)).expect("read registry");

        assert!(!loaded.registry_missing);
        assert_eq!(loaded.buckets, default_always_skip_buckets());
        assert!(loaded.configured_buckets.is_empty());
        assert!(!raw.contains("always_skip_buckets"));
    }

    #[test]
    fn load_always_skip_buckets_for_indexing_tracks_explicit_config_values() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 1\nworktree=[]\n[settings.extras]\nalways_skip_buckets=['target','vendor/bundle']\n",
        )
        .expect("write registry");

        let loaded = load_always_skip_buckets_for_indexing(repo_root).expect("load configured");

        assert!(!loaded.registry_missing);
        assert_eq!(
            loaded.buckets,
            BTreeSet::from(["target".to_string(), "vendor/bundle".to_string()])
        );
        assert_eq!(loaded.buckets, loaded.configured_buckets);
    }

    #[test]
    fn registry_schema_replacement_round_trip_includes_extras_settings_and_worktree_entries() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();

        let entry = RegistryEntry {
            name: "example".to_string(),
            path: repo_root
                .join("worktrees")
                .join("example")
                .to_string_lossy()
                .to_string(),
            created_at: "2026-02-28T00:00:00Z".to_string(),
        };

        save_always_skip_buckets(
            repo_root,
            &BTreeSet::from(["target".to_string(), "node_modules".to_string()]),
        )
        .expect("save buckets");
        insert_unique_entry(repo_root, entry.clone()).expect("insert entry");

        let raw = fs::read_to_string(registry_path(repo_root)).expect("read registry");
        assert!(raw.contains("version = 1"));
        assert!(raw.contains("[settings.extras]"));
        assert!(raw.contains("always_skip_buckets"));
        assert!(raw.contains("[[worktree]]"));

        let entries = load_registry(repo_root).expect("load entries");
        assert_eq!(entries, vec![entry]);
        let buckets = load_always_skip_buckets(repo_root).expect("load buckets");
        assert!(buckets.contains("target"));
        assert!(buckets.contains("node_modules"));
    }

    #[test]
    fn registry_schema_replacement_rejects_legacy_v1_shape() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "[[worktree]]\nname='w1'\npath='/tmp/w1'\ncreated_at='2026-01-01T00:00:00Z'\n",
        )
        .expect("write legacy registry");

        let error = load_registry(repo_root).expect_err("legacy should fail");
        assert!(error.to_string().starts_with(
            "invalid worktree registry schema: missing required top-level field 'version'"
        ));
    }

    #[test]
    fn registry_schema_replacement_reports_actionable_error_for_missing_version() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "[settings.extras]\nalways_skip_buckets=[\"target\"]\n[[worktree]]\nname='w1'\npath='/tmp/w1'\ncreated_at='2026-01-01T00:00:00Z'\n",
        )
        .expect("write registry");

        let error = load_registry(repo_root).expect_err("should fail");
        assert!(error.to_string().starts_with(
            "invalid worktree registry schema: missing required top-level field 'version'"
        ));
    }

    #[test]
    fn registry_schema_replacement_reports_actionable_error_for_unsupported_version() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 2\n[settings.extras]\nalways_skip_buckets=[\"target\"]\nworktree=[]\n",
        )
        .expect("write registry");

        let error = load_registry(repo_root).expect_err("should fail");
        assert!(
            error
                .to_string()
                .starts_with("invalid worktree registry schema: unsupported version")
        );
    }

    #[test]
    fn registry_schema_replacement_reports_actionable_error_for_missing_settings_extras() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(registry_path(repo_root), "version = 1\nworktree=[]\n").expect("write registry");

        let error = load_registry(repo_root).expect_err("should fail");
        assert!(error.to_string().starts_with(
            "invalid worktree registry schema: missing required section [settings.extras]"
        ));
    }

    #[test]
    fn registry_schema_replacement_reports_actionable_error_for_missing_worktree_section() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 1\n[settings.extras]\nalways_skip_buckets=[\"target\"]\n",
        )
        .expect("write registry");

        let error = load_registry(repo_root).expect_err("should fail");
        assert!(error.to_string().starts_with(
            "invalid worktree registry schema: missing required [[worktree]] entries section"
        ));
    }

    #[test]
    fn registry_schema_replacement_seeds_defaults_when_always_skip_buckets_unset() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 1\nworktree=[]\n[settings.extras]\n",
        )
        .expect("write registry");

        let buckets = load_always_skip_buckets(repo_root).expect("load buckets");
        assert!(buckets.contains("target"));
        assert!(buckets.contains("node_modules"));

        let raw = fs::read_to_string(registry_path(repo_root)).expect("read registry");
        assert!(raw.contains("always_skip_buckets"));
    }

    #[test]
    fn registry_schema_replacement_preserves_explicit_empty_always_skip_buckets_without_reseed() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 1\nworktree=[]\n[settings.extras]\nalways_skip_buckets=[]\n",
        )
        .expect("write registry");

        let buckets = load_always_skip_buckets(repo_root).expect("load buckets");
        assert!(buckets.is_empty());

        let raw = fs::read_to_string(registry_path(repo_root)).expect("read registry");
        assert!(raw.contains("always_skip_buckets"));
        assert!(!raw.contains("\"target\""));
    }

    #[test]
    fn registry_schema_replacement_loads_demo_setup_output() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join("worktrees")).expect("worktrees dir");
        fs::write(
            registry_path(repo_root),
            "version = 1\n[settings.extras]\nalways_skip_buckets=[\"target\",\"node_modules\"]\n[[worktree]]\nname='demo'\npath='/tmp/demo/worktrees/demo'\ncreated_at='2026-02-26T00:00:00Z'\n",
        )
        .expect("write registry");

        let entries = load_registry(repo_root).expect("load registry");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "demo");
    }
}
