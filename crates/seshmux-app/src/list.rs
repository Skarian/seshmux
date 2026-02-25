use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::App;
use crate::catalog::WorktreeCatalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListResult {
    pub repo_root: PathBuf,
    pub rows: Vec<WorktreeRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRow {
    pub name: String,
    pub path: PathBuf,
    pub created_at: String,
    pub branch: String,
    pub session_name: String,
    pub session_running: bool,
}

impl<'a> App<'a> {
    pub fn list(&self, cwd: &Path) -> Result<ListResult> {
        let catalog = WorktreeCatalog::load(self, cwd)?;
        let rows = catalog.list_rows(self)?;

        Ok(ListResult {
            repo_root: catalog.repo_root().to_path_buf(),
            rows,
        })
    }
}
