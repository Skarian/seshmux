use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use crossterm::event::{Event, KeyEvent};
use rayon::prelude::*;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;
use tui_tree_widget::{TreeItem, TreeState};

#[derive(Debug, Clone)]
pub(crate) struct ExtraNode {
    pub(crate) label: String,
    pub(crate) search_key: String,
    pub(crate) is_dir: bool,
    pub(crate) children: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtrasIndex {
    pub(crate) nodes: BTreeMap<String, ExtraNode>,
    pub(crate) roots: Vec<String>,
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
    pub(crate) fn from_candidates(candidates: &[PathBuf]) -> Result<Self> {
        let index = build_extras_index_from_paths(candidates)?;
        Ok(Self::from_index(index))
    }

    pub(crate) fn from_index(index: ExtrasIndex) -> Self {
        let collapsed = index
            .nodes
            .iter()
            .filter(|(_, node)| node.is_dir)
            .map(|(key, _)| key.clone())
            .collect();
        let mut state = Self {
            nodes: index.nodes,
            roots: index.roots,
            checked: HashSet::new(),
            collapsed,
            visible: Vec::new(),
            cursor: 0,
            filter: Input::default(),
            editing_filter: false,
        };
        state.refresh_visible();
        state
    }

    pub(crate) fn refresh_visible(&mut self) {
        self.visible.clear();
        let roots = self.roots.clone();
        let needle = filter_needle(&self.filter);
        for root in &roots {
            self.push_visible(root, needle.as_deref());
        }

        if self.visible.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len() - 1;
        }
    }

    fn push_visible(&mut self, key: &str, needle: Option<&str>) {
        if !self.subtree_matches_filter(key, needle) {
            return;
        }

        self.visible.push(VisibleRow {
            key: key.to_string(),
        });

        let Some(node) = self.nodes.get(key) else {
            return;
        };
        let children = node.children.clone();
        let filtering = needle.is_some();
        let open = filtering || !self.collapsed.contains(key);

        if open {
            for child in &children {
                self.push_visible(child, needle);
            }
        }
    }

    fn subtree_matches_filter(&self, key: &str, needle: Option<&str>) -> bool {
        let Some(needle) = needle else {
            return true;
        };

        let Some(node) = self.nodes.get(key) else {
            return false;
        };

        if node.search_key.contains(needle) {
            return true;
        }

        node.children
            .iter()
            .any(|child| self.subtree_matches_filter(child, Some(needle)))
    }

    pub(crate) fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_down(&mut self) {
        if self.cursor + 1 < self.visible.len() {
            self.cursor += 1;
        }
    }

    pub(crate) fn move_up_by(&mut self, rows: usize) {
        self.cursor = self.cursor.saturating_sub(rows);
    }

    pub(crate) fn move_down_by(&mut self, rows: usize) {
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = self
            .cursor
            .saturating_add(rows)
            .min(self.visible.len().saturating_sub(1));
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
        let needle = filter_needle(&self.filter);
        for root in &self.roots {
            if let Some(item) = self.tree_item_for(root, needle.as_deref()) {
                items.push(item);
            }
        }
        items
    }

    fn tree_item_for(&self, key: &str, needle: Option<&str>) -> Option<TreeItem<'static, String>> {
        if !self.subtree_matches_filter(key, needle) {
            return None;
        }

        let node = self.nodes.get(key)?;
        let label = format!(
            "{} {} {}",
            self.mark_for(key),
            if node.is_dir { "📁" } else { "📄" },
            node.label
        );

        let mut children = Vec::new();
        for child in &node.children {
            if let Some(item) = self.tree_item_for(child, needle) {
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
        let filtering = filter_needle(&self.filter).is_some();
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

#[derive(Debug)]
struct PreparedPath {
    normalized: String,
    components: Vec<String>,
    lowered_components: Vec<String>,
}

pub(crate) fn build_extras_index_from_paths(candidates: &[PathBuf]) -> Result<ExtrasIndex> {
    let mut prepared: Vec<PreparedPath> = candidates
        .par_iter()
        .filter_map(|candidate| prepare_candidate_path(candidate))
        .collect();

    prepared.par_sort_unstable_by(|left, right| left.normalized.cmp(&right.normalized));
    prepared.dedup_by(|left, right| left.normalized == right.normalized);

    let mut nodes = BTreeMap::<String, ExtraNode>::new();
    let mut roots = BTreeSet::<String>::new();

    for path in &prepared {
        insert_prepared_path(&mut nodes, &mut roots, path);
    }

    Ok(ExtrasIndex {
        nodes,
        roots: roots.into_iter().collect(),
    })
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

fn prepare_candidate_path(candidate: &Path) -> Option<PreparedPath> {
    let normalized = seshmux_core::extras::normalize_extra_relative_path(candidate).ok()?;
    if is_worktrees_relative_path(&normalized) {
        return None;
    }

    let mut components = Vec::<String>::new();
    for component in normalized.components() {
        let Component::Normal(value) = component else {
            return None;
        };
        components.push(value.to_str()?.to_string());
    }

    if components.is_empty() {
        return None;
    }

    let lowered_components: Vec<String> = components
        .iter()
        .map(|value| value.to_lowercase())
        .collect();
    let normalized = components.join("/");

    Some(PreparedPath {
        normalized,
        components,
        lowered_components,
    })
}

fn insert_prepared_path(
    nodes: &mut BTreeMap<String, ExtraNode>,
    roots: &mut BTreeSet<String>,
    path: &PreparedPath,
) {
    let mut key = String::new();
    let mut lowered_key = String::new();
    let mut parent: Option<String> = None;

    for (index, component) in path.components.iter().enumerate() {
        if index > 0 {
            key.push('/');
            lowered_key.push('/');
        }
        key.push_str(component);
        lowered_key.push_str(&path.lowered_components[index]);

        let is_last = index + 1 == path.components.len();
        let node_is_dir = !is_last;
        let search_key = format!("{} {}", lowered_key, path.lowered_components[index]);

        nodes
            .entry(key.clone())
            .and_modify(|node| {
                if node_is_dir {
                    node.is_dir = true;
                }
            })
            .or_insert_with(|| ExtraNode {
                label: component.clone(),
                search_key,
                is_dir: node_is_dir,
                children: Vec::new(),
            });

        if let Some(parent_key) = &parent {
            if let Some(parent_node) = nodes.get_mut(parent_key)
                && !parent_node.children.iter().any(|child| child == &key)
            {
                parent_node.children.push(key.clone());
                parent_node.children.sort();
            }
        } else {
            roots.insert(key.clone());
        }

        parent = Some(key.clone());
    }
}

fn is_worktrees_relative_path(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Normal(value)) if value == "worktrees"
    )
}

fn filter_needle(input: &Input) -> Option<String> {
    let needle = input.value().trim();
    if needle.is_empty() {
        None
    } else {
        Some(needle.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use super::{ExtrasState, build_extras_index_from_paths};

    fn open_first_directory(state: &mut ExtrasState) {
        state.cursor = 0;
        state.toggle_fold_current();
    }

    #[test]
    fn from_candidates_skips_invalid_relative_paths() {
        let state = ExtrasState::from_candidates(&[
            PathBuf::from("../outside.txt"),
            PathBuf::from("keep.txt"),
            PathBuf::from("./"),
        ])
        .expect("state");

        assert!(state.nodes.contains_key("keep.txt"));
        assert!(!state.nodes.contains_key("../outside.txt"));
    }

    #[test]
    fn from_candidates_skips_worktrees_directory() {
        let state = ExtrasState::from_candidates(&[
            PathBuf::from("worktrees/cache/state.txt"),
            PathBuf::from("keep.txt"),
        ])
        .expect("state");

        assert!(state.nodes.contains_key("keep.txt"));
        assert!(!state.nodes.contains_key("worktrees"));
    }

    #[test]
    fn build_index_contains_top_level_nodes() {
        let state = ExtrasState::from_candidates(&[
            PathBuf::from("src/main.rs"),
            PathBuf::from("src/lib.rs"),
            PathBuf::from("notes.txt"),
        ])
        .expect("state");

        assert_eq!(state.roots, vec!["notes.txt", "src"]);
    }

    #[test]
    fn toggling_fold_uses_prebuilt_index_without_discovery_io() {
        let mut state = ExtrasState::from_candidates(&[
            PathBuf::from("dir/sub/one.txt"),
            PathBuf::from("dir/sub/two.txt"),
        ])
        .expect("state");

        assert_eq!(state.visible.len(), 1);
        open_first_directory(&mut state);
        assert!(state.visible.len() > 1);
        state.toggle_fold_current();
        assert_eq!(state.visible.len(), 1);
    }

    #[test]
    fn filter_unfolds_matching_subtrees() {
        let mut state = ExtrasState::from_candidates(&[
            PathBuf::from("dir/sub/one.txt"),
            PathBuf::from("dir/sub/two.txt"),
        ])
        .expect("state");

        state.filter = tui_input::Input::new("one".to_string());
        state.refresh_visible();

        let visible: Vec<String> = state.visible.iter().map(|row| row.key.clone()).collect();
        assert_eq!(visible, vec!["dir", "dir/sub", "dir/sub/one.txt"]);
    }

    #[test]
    fn directory_selection_mark_shows_partial_when_descendants_diverge() {
        let mut state = ExtrasState::from_candidates(&[
            PathBuf::from("dir/one.txt"),
            PathBuf::from("dir/two.txt"),
        ])
        .expect("state");

        open_first_directory(&mut state);
        state.toggle_current();
        state.move_down();
        state.toggle_current();

        assert_eq!(state.mark_for("dir"), "[-]");
    }

    #[test]
    fn deterministic_ordering_is_stable_for_repeated_and_shuffled_inputs() {
        let base = vec![
            PathBuf::from("b/z.txt"),
            PathBuf::from("a/y.txt"),
            PathBuf::from("a/x.txt"),
        ];
        let shuffled = vec![
            PathBuf::from("a/x.txt"),
            PathBuf::from("b/z.txt"),
            PathBuf::from("a/y.txt"),
        ];

        let first = build_extras_index_from_paths(&base).expect("first");
        let second = build_extras_index_from_paths(&base).expect("second");
        let third = build_extras_index_from_paths(&shuffled).expect("third");

        assert_eq!(first.roots, second.roots);
        assert_eq!(first.roots, third.roots);
        assert_eq!(first.nodes["a"].children, second.nodes["a"].children);
        assert_eq!(first.nodes["a"].children, third.nodes["a"].children);
    }

    #[test]
    fn selected_for_copy_emits_only_leaves_when_directory_fully_checked() {
        let mut state = ExtrasState::from_candidates(&[
            PathBuf::from("crates/.DS_Store"),
            PathBuf::from("crates/seshmux-tui/src/ui/loading.rs"),
        ])
        .expect("state");

        state.toggle_current();
        let selected = state.selected_for_copy();

        assert_eq!(
            selected,
            vec![
                PathBuf::from("crates/.DS_Store"),
                PathBuf::from("crates/seshmux-tui/src/ui/loading.rs"),
            ]
        );
    }

    #[test]
    #[ignore]
    fn timing_receipt_large_synthetic_tree_index_and_interaction() {
        let mut candidates = Vec::<PathBuf>::new();

        for index in 0..90_000 {
            candidates.push(PathBuf::from(format!("target/debug/deps/object-{index}.o")));
        }

        for index in 0..1_500 {
            candidates.push(PathBuf::from(format!("node_modules/pkg-{index}/index.js")));
        }

        for index in 0..1_500 {
            candidates.push(PathBuf::from(format!("dist/chunk-{index}.js")));
        }

        let build_started = Instant::now();
        let index = build_extras_index_from_paths(&candidates).expect("index");
        let build_elapsed = build_started.elapsed();

        let mut state = ExtrasState::from_index(index);
        let interaction_started = Instant::now();
        for _ in 0..500 {
            state.move_down_by(2);
            state.move_up_by(1);
            state.toggle_fold_current();
            state.toggle_fold_current();
        }
        state.filter = tui_input::Input::new("object-89999".to_string());
        state.refresh_visible();
        let interaction_elapsed = interaction_started.elapsed();

        println!(
            "TIMING build_extras_index_from_paths entries={} elapsed_ms={}",
            candidates.len(),
            build_elapsed.as_millis()
        );
        println!(
            "TIMING extras_picker_interaction visible_rows={} elapsed_ms={}",
            state.visible.len(),
            interaction_elapsed.as_millis()
        );
        assert!(!state.nodes.is_empty());
    }

    #[test]
    #[ignore]
    fn timing_receipt_large_synthetic_tree_skip_ratio() {
        let mut candidates = Vec::<PathBuf>::new();

        for index in 0..90_000 {
            candidates.push(PathBuf::from(format!("target/debug/deps/object-{index}.o")));
        }

        for index in 0..1_500 {
            candidates.push(PathBuf::from(format!("node_modules/pkg-{index}/index.js")));
        }

        for index in 0..1_500 {
            candidates.push(PathBuf::from(format!("dist/chunk-{index}.js")));
        }

        let full_started = Instant::now();
        let _ = build_extras_index_from_paths(&candidates).expect("full index");
        let full_elapsed = full_started.elapsed();

        let skip_rules = std::collections::BTreeSet::from([
            "target".to_string(),
            "node_modules".to_string(),
            "dist".to_string(),
        ]);
        let flagged = seshmux_core::extras::classify_flagged_buckets(&candidates, &skip_rules);
        let skipped: std::collections::BTreeSet<String> = flagged.keys().cloned().collect();
        let filtered =
            seshmux_core::extras::filter_candidates_by_skipped_buckets(&candidates, &skipped);

        let skip_started = Instant::now();
        let _ = build_extras_index_from_paths(&filtered).expect("filtered index");
        let skip_elapsed = skip_started.elapsed();

        let ratio = full_elapsed.as_secs_f64() / skip_elapsed.as_secs_f64().max(0.001);
        println!(
            "TIMING extras_skip_ratio full_ms={} skip_ms={} ratio={:.2}",
            full_elapsed.as_millis(),
            skip_elapsed.as_millis(),
            ratio
        );
        assert!(ratio >= 10.0, "expected ratio >= 10.0, got {ratio:.2}");
    }
}
