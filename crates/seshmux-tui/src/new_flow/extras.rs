use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{Event, KeyEvent};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;
use tui_tree_widget::{TreeItem, TreeState};

#[derive(Debug, Clone)]
pub(crate) struct ExtraNode {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) is_dir: bool,
    pub(crate) children: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VisibleRow {
    pub(crate) key: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtrasState {
    pub(crate) nodes: BTreeMap<String, ExtraNode>,
    pub(crate) roots: Vec<String>,
    pub(crate) checked: HashSet<String>,
    pub(crate) collapsed: HashSet<String>,
    pub(crate) visible: Vec<VisibleRow>,
    pub(crate) cursor: usize,
    pub(crate) filter: Input,
    pub(crate) editing_filter: bool,
}

impl ExtrasState {
    pub(crate) fn from_candidates(repo_root: &Path, candidates: &[PathBuf]) -> Result<Self> {
        let mut normalized = BTreeSet::new();

        for candidate in candidates {
            let relative = seshmux_core::extras::normalize_extra_relative_path(candidate)
                .map_err(|error| anyhow!(error.to_string()))?;
            let absolute = repo_root.join(&relative);

            if absolute.is_dir() {
                normalized.insert(relative.clone());
                expand_directory(repo_root, &absolute, &mut normalized)?;
            } else {
                normalized.insert(relative);
            }
        }

        let mut nodes = BTreeMap::<String, ExtraNode>::new();
        let mut roots = BTreeSet::<String>::new();

        for path in normalized {
            let is_dir = repo_root.join(&path).is_dir();
            insert_path(&mut nodes, &mut roots, &path, is_dir)?;
        }

        let mut state = Self {
            nodes,
            roots: roots.into_iter().collect(),
            checked: HashSet::new(),
            collapsed: HashSet::new(),
            visible: Vec::new(),
            cursor: 0,
            filter: Input::default(),
            editing_filter: false,
        };
        state.refresh_visible();
        Ok(state)
    }

    pub(crate) fn refresh_visible(&mut self) {
        self.visible.clear();
        let roots = self.roots.clone();
        for root in &roots {
            self.push_visible(root);
        }

        if self.visible.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len() - 1;
        }
    }

    fn push_visible(&mut self, key: &str) {
        if !self.subtree_matches_filter(key) {
            return;
        }

        self.visible.push(VisibleRow {
            key: key.to_string(),
        });

        let Some(node) = self.nodes.get(key) else {
            return;
        };
        let children = node.children.clone();
        let filtering = !self.filter.value().trim().is_empty();
        let open = filtering || !self.collapsed.contains(key);

        if open {
            for child in &children {
                self.push_visible(child);
            }
        }
    }

    fn subtree_matches_filter(&self, key: &str) -> bool {
        if self.filter.value().trim().is_empty() {
            return true;
        }

        let Some(node) = self.nodes.get(key) else {
            return false;
        };

        let needle = self.filter.value().trim().to_lowercase();
        if node.key.to_lowercase().contains(&needle) || node.label.to_lowercase().contains(&needle)
        {
            return true;
        }

        node.children
            .iter()
            .any(|child| self.subtree_matches_filter(child))
    }

    pub(crate) fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_down(&mut self) {
        if self.cursor + 1 < self.visible.len() {
            self.cursor += 1;
        }
    }

    pub(crate) fn toggle_current(&mut self) {
        let Some(row) = self.visible.get(self.cursor) else {
            return;
        };
        let key = row.key.clone();
        let should_select = !self.checked.contains(&key);
        self.set_recursive_checked(&key, should_select);
    }

    pub(crate) fn toggle_fold_current(&mut self) {
        let Some(row) = self.visible.get(self.cursor) else {
            return;
        };
        let key = row.key.clone();
        let is_dir = self
            .nodes
            .get(&key)
            .map(|node| node.is_dir)
            .unwrap_or(false);
        if !is_dir {
            return;
        }

        if self.collapsed.contains(&key) {
            self.collapsed.remove(&key);
        } else {
            self.collapsed.insert(key);
        }
        self.refresh_visible();
    }

    fn set_recursive_checked(&mut self, key: &str, value: bool) {
        if value {
            self.checked.insert(key.to_string());
        } else {
            self.checked.remove(key);
        }

        let Some(node) = self.nodes.get(key) else {
            return;
        };
        let children = node.children.clone();

        for child in &children {
            self.set_recursive_checked(child, value);
        }
    }

    pub(crate) fn select_all(&mut self) {
        self.checked = self.nodes.keys().cloned().collect();
    }

    pub(crate) fn select_none(&mut self) {
        self.checked.clear();
    }

    pub(crate) fn toggle_filter_editing(&mut self) {
        self.editing_filter = !self.editing_filter;
    }

    pub(crate) fn edit_filter(&mut self, key: KeyEvent) {
        if self.filter.handle_event(&Event::Key(key)).is_some() {
            self.refresh_visible();
        }
    }

    pub(crate) fn selected_for_copy(&self) -> Vec<PathBuf> {
        let mut selected = Vec::<String>::new();
        for root in &self.roots {
            self.collect_selected(root, &mut selected);
        }
        selected.sort();
        selected.dedup();
        selected.into_iter().map(PathBuf::from).collect()
    }

    fn collect_selected(&self, key: &str, selected: &mut Vec<String>) {
        let Some(node) = self.nodes.get(key) else {
            return;
        };

        if node.is_dir {
            if self.checked.contains(key) && self.descendants_fully_checked(key) {
                selected.push(key.to_string());
                return;
            }

            if node.children.is_empty() {
                if self.checked.contains(key) {
                    selected.push(key.to_string());
                }
                return;
            }

            for child in &node.children {
                self.collect_selected(child, selected);
            }
            return;
        }

        if self.checked.contains(key) {
            selected.push(key.to_string());
        }
    }

    fn descendants_fully_checked(&self, key: &str) -> bool {
        let Some(node) = self.nodes.get(key) else {
            return false;
        };

        if node.children.is_empty() {
            return self.checked.contains(key);
        }

        for child in &node.children {
            if !self.checked.contains(child) {
                return false;
            }
            if !self.descendants_fully_checked(child) {
                return false;
            }
        }

        true
    }

    fn mark_for(&self, key: &str) -> &'static str {
        let Some(node) = self.nodes.get(key) else {
            return "[ ]";
        };

        if !node.is_dir {
            return if self.checked.contains(key) {
                "[x]"
            } else {
                "[ ]"
            };
        }

        let has_checked_descendant = self.has_checked_descendant(key);
        let full = self.checked.contains(key) && self.descendants_fully_checked(key);

        if full {
            "[x]"
        } else if self.checked.contains(key) || has_checked_descendant {
            "[-]"
        } else {
            "[ ]"
        }
    }

    fn has_checked_descendant(&self, key: &str) -> bool {
        let Some(node) = self.nodes.get(key) else {
            return false;
        };

        for child in &node.children {
            if self.checked.contains(child) || self.has_checked_descendant(child) {
                return true;
            }
        }
        false
    }

    pub(crate) fn tree_items(&self) -> Vec<TreeItem<'static, String>> {
        let mut items = Vec::new();
        for root in &self.roots {
            if let Some(item) = self.tree_item_for(root) {
                items.push(item);
            }
        }
        items
    }

    fn tree_item_for(&self, key: &str) -> Option<TreeItem<'static, String>> {
        if !self.subtree_matches_filter(key) {
            return None;
        }

        let node = self.nodes.get(key)?;
        let label = format!(
            "{} {} {}",
            self.mark_for(key),
            if node.is_dir { "[D]" } else { "[F]" },
            node.label
        );

        let mut children = Vec::new();
        for child in &node.children {
            if let Some(item) = self.tree_item_for(child) {
                children.push(item);
            }
        }

        if children.is_empty() {
            Some(TreeItem::new_leaf(key.to_string(), label))
        } else {
            Some(
                TreeItem::new(key.to_string(), label, children)
                    .expect("all extra tree identifiers are unique"),
            )
        }
    }

    pub(crate) fn tree_state(&self) -> TreeState<String> {
        let mut state = TreeState::default();
        let filtering = !self.filter.value().trim().is_empty();
        for row in &self.visible {
            let is_dir = self
                .nodes
                .get(&row.key)
                .map(|node| node.is_dir)
                .unwrap_or(false);
            if is_dir && (filtering || !self.collapsed.contains(&row.key)) {
                state.open(identifier_path_for_key(&row.key));
            }
        }

        if let Some(row) = self.visible.get(self.cursor) {
            state.select(identifier_path_for_key(&row.key));
        }
        state
    }
}

pub(crate) fn identifier_path_for_key(key: &str) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut current = PathBuf::new();
    for component in Path::new(key).components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        identifiers.push(current.to_string_lossy().to_string());
    }
    identifiers
}

pub(crate) fn expand_directory(
    repo_root: &Path,
    directory: &Path,
    output: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("failed to read extra directory {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", directory.display()))?;
        let absolute = entry.path();
        let relative = absolute
            .strip_prefix(repo_root)
            .with_context(|| format!("failed to relativize path {}", absolute.display()))?
            .to_path_buf();

        output.insert(relative.clone());

        if absolute.is_dir() {
            expand_directory(repo_root, &absolute, output)?;
        }
    }

    Ok(())
}

pub(crate) fn insert_path(
    nodes: &mut BTreeMap<String, ExtraNode>,
    roots: &mut BTreeSet<String>,
    path: &Path,
    is_dir: bool,
) -> Result<()> {
    let mut current = PathBuf::new();
    let components: Vec<_> = path.components().collect();

    for (index, component) in components.iter().enumerate() {
        let Component::Normal(part) = component else {
            return Err(anyhow!(
                "invalid extra path component in {}",
                path.display()
            ));
        };

        current.push(part);
        let key = current
            .to_str()
            .ok_or_else(|| anyhow!("extra path is not valid UTF-8: {}", path.display()))?
            .to_string();

        let parent = current
            .parent()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);

        let is_last = index + 1 == components.len();
        let node_is_dir = if is_last { is_dir } else { true };
        let label = part
            .to_str()
            .ok_or_else(|| anyhow!("extra path is not valid UTF-8: {}", path.display()))?
            .to_string();

        nodes
            .entry(key.clone())
            .and_modify(|node| {
                if node_is_dir {
                    node.is_dir = true;
                }
            })
            .or_insert_with(|| ExtraNode {
                key: key.clone(),
                label,
                is_dir: node_is_dir,
                children: Vec::new(),
            });

        if let Some(parent_key) = &parent {
            if let Some(parent_node) = nodes.get_mut(parent_key) {
                if !parent_node.children.iter().any(|child| child == &key) {
                    parent_node.children.push(key.clone());
                    parent_node.children.sort();
                }
            }
        } else {
            roots.insert(key);
        }
    }

    Ok(())
}
