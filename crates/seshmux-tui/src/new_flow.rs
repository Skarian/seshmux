use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use seshmux_app::{App, NewPrepare, NewRequest, NewResult, NewStartPoint};
use seshmux_core::git::{BranchRef, CommitRef};

use crate::{TerminalSession, UiExit, centered_rect, is_ctrl_c};

pub(crate) trait NewFlowOps {
    fn prepare(&self, cwd: &Path) -> Result<NewPrepare>;
    fn query_branches(&self, repo_root: &Path, query: &str) -> Result<Vec<BranchRef>>;
    fn query_commits(&self, repo_root: &Path, query: &str, limit: usize) -> Result<Vec<CommitRef>>;
    fn list_extras(&self, repo_root: &Path) -> Result<Vec<PathBuf>>;
    fn execute_new(&self, request: NewRequest) -> Result<NewResult>;
}

impl<'a> NewFlowOps for App<'a> {
    fn prepare(&self, cwd: &Path) -> Result<NewPrepare> {
        self.new_prepare(cwd)
    }

    fn query_branches(&self, repo_root: &Path, query: &str) -> Result<Vec<BranchRef>> {
        self.new_query_branches(repo_root, query)
    }

    fn query_commits(&self, repo_root: &Path, query: &str, limit: usize) -> Result<Vec<CommitRef>> {
        self.new_query_commits(repo_root, query, limit)
    }

    fn list_extras(&self, repo_root: &Path) -> Result<Vec<PathBuf>> {
        self.new_list_extras(repo_root)
    }

    fn execute_new(&self, request: NewRequest) -> Result<NewResult> {
        self.new_execute(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    GitignoreDecision,
    NameInput,
    StartPointMode,
    BranchPicker,
    BranchSearchInput,
    CommitPicker,
    CommitSearchInput,
    ExtrasPicker,
    ConnectNow,
    Review,
    Success,
    ErrorScreen,
}

#[derive(Debug, Clone)]
struct BranchPickerState {
    query: Option<String>,
    items: Vec<BranchRef>,
    selected: usize,
}

#[derive(Debug, Clone)]
struct CommitPickerState {
    query: Option<String>,
    items: Vec<CommitRef>,
    selected: usize,
}

#[derive(Debug, Clone)]
struct ExtraNode {
    key: String,
    label: String,
    is_dir: bool,
    children: Vec<String>,
}

#[derive(Debug, Clone)]
struct VisibleRow {
    key: String,
    depth: usize,
}

#[derive(Debug, Clone)]
struct ExtrasState {
    nodes: BTreeMap<String, ExtraNode>,
    roots: Vec<String>,
    checked: HashSet<String>,
    visible: Vec<VisibleRow>,
    cursor: usize,
    filter: String,
    editing_filter: bool,
}

impl ExtrasState {
    fn from_candidates(repo_root: &Path, candidates: &[PathBuf]) -> Result<Self> {
        let mut normalized = BTreeSet::new();

        for candidate in candidates {
            let relative = normalize_relative(candidate)?;
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
            visible: Vec::new(),
            cursor: 0,
            filter: String::new(),
            editing_filter: false,
        };
        state.refresh_visible();
        Ok(state)
    }

    fn refresh_visible(&mut self) {
        self.visible.clear();
        let roots = self.roots.clone();
        for root in &roots {
            self.push_visible(root, 0);
        }

        if self.visible.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.visible.len() {
            self.cursor = self.visible.len() - 1;
        }
    }

    fn push_visible(&mut self, key: &str, depth: usize) {
        if !self.subtree_matches_filter(key) {
            return;
        }

        self.visible.push(VisibleRow {
            key: key.to_string(),
            depth,
        });

        let Some(node) = self.nodes.get(key) else {
            return;
        };
        let children = node.children.clone();

        for child in &children {
            self.push_visible(child, depth + 1);
        }
    }

    fn subtree_matches_filter(&self, key: &str) -> bool {
        if self.filter.trim().is_empty() {
            return true;
        }

        let Some(node) = self.nodes.get(key) else {
            return false;
        };

        let needle = self.filter.trim().to_lowercase();
        if node.key.to_lowercase().contains(&needle) || node.label.to_lowercase().contains(&needle)
        {
            return true;
        }

        node.children
            .iter()
            .any(|child| self.subtree_matches_filter(child))
    }

    fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if self.cursor + 1 < self.visible.len() {
            self.cursor += 1;
        }
    }

    fn toggle_current(&mut self) {
        let Some(row) = self.visible.get(self.cursor) else {
            return;
        };
        let key = row.key.clone();
        let should_select = !self.checked.contains(&key);
        self.set_recursive_checked(&key, should_select);
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

    fn select_all(&mut self) {
        self.checked = self.nodes.keys().cloned().collect();
    }

    fn select_none(&mut self) {
        self.checked.clear();
    }

    fn toggle_filter_editing(&mut self) {
        self.editing_filter = !self.editing_filter;
    }

    fn edit_filter(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.filter.push(character);
            }
            _ => {}
        }

        self.refresh_visible();
    }

    fn selected_for_copy(&self) -> Vec<PathBuf> {
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
}

fn normalize_relative(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Err(anyhow!("extra path must be relative: {}", path.display()));
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => clean.push(value),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!(
                    "extra path must stay inside repository: {}",
                    path.display()
                ));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(anyhow!("extra path cannot be empty"));
    }

    Ok(clean)
}

fn expand_directory(
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

fn insert_path(
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

#[derive(Debug)]
struct NewFlow {
    cwd: PathBuf,
    prepare: NewPrepare,
    step: Step,
    gitignore_yes_selected: bool,
    name_input: String,
    name_error: Option<String>,
    start_mode_selected: usize,
    start_point: Option<NewStartPoint>,
    branch_picker: Option<BranchPickerState>,
    branch_search_input: String,
    commit_picker: Option<CommitPickerState>,
    commit_search_input: String,
    extras: ExtrasState,
    connect_yes_selected: bool,
    success: Option<NewResult>,
    error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSignal {
    Continue,
    Exit(UiExit),
}

impl NewFlow {
    fn new(ops: &dyn NewFlowOps, cwd: &Path) -> Result<Self> {
        let prepare = ops.prepare(cwd)?;
        let commits = ops.query_commits(&prepare.repo_root, "", 1)?;
        if commits.is_empty() {
            return Err(anyhow!(
                "repository has no commits yet; create an initial commit before starting seshmux"
            ));
        }
        let candidates = ops.list_extras(&prepare.repo_root)?;
        let extras = ExtrasState::from_candidates(&prepare.repo_root, &candidates)?;

        let first_step = if prepare.gitignore_has_worktrees_entry {
            Step::NameInput
        } else {
            Step::GitignoreDecision
        };

        Ok(Self {
            cwd: cwd.to_path_buf(),
            prepare,
            step: first_step,
            gitignore_yes_selected: true,
            name_input: String::new(),
            name_error: None,
            start_mode_selected: 0,
            start_point: None,
            branch_picker: None,
            branch_search_input: String::new(),
            commit_picker: None,
            commit_search_input: String::new(),
            extras,
            connect_yes_selected: true,
            success: None,
            error_message: None,
        })
    }

    fn render(&self, frame: &mut ratatui::Frame<'_>) {
        match self.step {
            Step::GitignoreDecision => self.render_gitignore_decision(frame),
            Step::NameInput => self.render_name_input(frame),
            Step::StartPointMode => self.render_start_mode(frame),
            Step::BranchPicker => self.render_branch_picker(frame),
            Step::BranchSearchInput => self.render_branch_search_input(frame),
            Step::CommitPicker => self.render_commit_picker(frame),
            Step::CommitSearchInput => self.render_commit_search_input(frame),
            Step::ExtrasPicker => self.render_extras_picker(frame),
            Step::ConnectNow => self.render_connect_now(frame),
            Step::Review => self.render_review(frame),
            Step::Success => self.render_success(frame),
            Step::ErrorScreen => self.render_error(frame),
        }
    }

    fn on_key(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        match self.step {
            Step::GitignoreDecision => self.on_key_gitignore(key),
            Step::NameInput => self.on_key_name(key),
            Step::StartPointMode => self.on_key_start_mode(key, ops),
            Step::BranchPicker => self.on_key_branch_picker(key, ops),
            Step::BranchSearchInput => self.on_key_branch_search_input(key, ops),
            Step::CommitPicker => self.on_key_commit_picker(key, ops),
            Step::CommitSearchInput => self.on_key_commit_search_input(key, ops),
            Step::ExtrasPicker => self.on_key_extras(key),
            Step::ConnectNow => self.on_key_connect_now(key),
            Step::Review => self.on_key_review(key, ops),
            Step::Success => self.on_key_success(key),
            Step::ErrorScreen => self.on_key_error(key),
        }
    }

    fn on_key_gitignore(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => Ok(FlowSignal::Exit(UiExit::BackAtRoot)),
            KeyCode::Left | KeyCode::Up => {
                self.gitignore_yes_selected = true;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Right | KeyCode::Down => {
                self.gitignore_yes_selected = false;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                self.step = Step::NameInput;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_name(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                if self.prepare.gitignore_has_worktrees_entry {
                    Ok(FlowSignal::Exit(UiExit::BackAtRoot))
                } else {
                    self.step = Step::GitignoreDecision;
                    Ok(FlowSignal::Continue)
                }
            }
            KeyCode::Backspace => {
                self.name_input.pop();
                self.name_error = None;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.name_input.push(character);
                self.name_error = None;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                let candidate = self.name_input.trim().to_string();
                match seshmux_core::names::validate_worktree_name(&candidate) {
                    Ok(()) => {
                        self.name_input = candidate;
                        self.name_error = None;
                        self.step = Step::StartPointMode;
                    }
                    Err(error) => {
                        self.name_error = Some(error.to_string());
                    }
                }
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_start_mode(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::NameInput;
                Ok(FlowSignal::Continue)
            }
            KeyCode::Up => {
                self.start_mode_selected = self.start_mode_selected.saturating_sub(1);
                Ok(FlowSignal::Continue)
            }
            KeyCode::Down => {
                if self.start_mode_selected < 2 {
                    self.start_mode_selected += 1;
                }
                Ok(FlowSignal::Continue)
            }
            KeyCode::Enter => {
                match self.start_mode_selected {
                    0 => {
                        self.start_point = Some(NewStartPoint::CurrentBranch);
                        self.step = Step::ExtrasPicker;
                    }
                    1 => {
                        self.load_branches(ops, "")?;
                        self.step = Step::BranchPicker;
                    }
                    _ => {
                        self.load_commits(ops, "")?;
                        self.step = Step::CommitPicker;
                    }
                }
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_branch_picker(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        let Some(mut picker) = self.branch_picker.clone() else {
            self.load_branches(ops, "")?;
            return Ok(FlowSignal::Continue);
        };

        let action_rows = if picker.query.is_some() { 2 } else { 1 };
        let total_rows = action_rows + picker.items.len();

        match key.code {
            KeyCode::Esc => {
                self.step = Step::StartPointMode;
            }
            KeyCode::Up => {
                picker.selected = picker.selected.saturating_sub(1);
                self.branch_picker = Some(picker);
            }
            KeyCode::Down => {
                if picker.selected + 1 < total_rows {
                    picker.selected += 1;
                }
                self.branch_picker = Some(picker);
            }
            KeyCode::Enter => {
                if picker.selected == 0 {
                    self.branch_search_input = picker.query.clone().unwrap_or_default();
                    self.step = Step::BranchSearchInput;
                    self.branch_picker = Some(picker);
                    return Ok(FlowSignal::Continue);
                }

                if picker.query.is_some() && picker.selected == 1 {
                    self.load_branches(ops, "")?;
                    return Ok(FlowSignal::Continue);
                }

                let index = picker.selected.saturating_sub(action_rows);
                if let Some(branch) = picker.items.get(index) {
                    self.start_point = Some(NewStartPoint::Branch(branch.name.clone()));
                    self.step = Step::ExtrasPicker;
                }
                self.branch_picker = Some(picker);
            }
            _ => {
                self.branch_picker = Some(picker);
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_branch_search_input(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
    ) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::BranchPicker;
            }
            KeyCode::Backspace => {
                self.branch_search_input.pop();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.branch_search_input.push(character);
            }
            KeyCode::Enter => {
                let query = self.branch_search_input.trim().to_string();
                self.load_branches(ops, &query)?;
                self.step = Step::BranchPicker;
            }
            _ => {}
        }
        Ok(FlowSignal::Continue)
    }

    fn on_key_commit_picker(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        let Some(mut picker) = self.commit_picker.clone() else {
            self.load_commits(ops, "")?;
            return Ok(FlowSignal::Continue);
        };

        let action_rows = if picker.query.is_some() { 2 } else { 1 };
        let total_rows = action_rows + picker.items.len();

        match key.code {
            KeyCode::Esc => {
                self.step = Step::StartPointMode;
            }
            KeyCode::Up => {
                picker.selected = picker.selected.saturating_sub(1);
                self.commit_picker = Some(picker);
            }
            KeyCode::Down => {
                if picker.selected + 1 < total_rows {
                    picker.selected += 1;
                }
                self.commit_picker = Some(picker);
            }
            KeyCode::Enter => {
                if picker.selected == 0 {
                    self.commit_search_input = picker.query.clone().unwrap_or_default();
                    self.step = Step::CommitSearchInput;
                    self.commit_picker = Some(picker);
                    return Ok(FlowSignal::Continue);
                }

                if picker.query.is_some() && picker.selected == 1 {
                    self.load_commits(ops, "")?;
                    return Ok(FlowSignal::Continue);
                }

                let index = picker.selected.saturating_sub(action_rows);
                if let Some(commit) = picker.items.get(index) {
                    self.start_point = Some(NewStartPoint::Commit(commit.hash.clone()));
                    self.step = Step::ExtrasPicker;
                }
                self.commit_picker = Some(picker);
            }
            _ => {
                self.commit_picker = Some(picker);
            }
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_commit_search_input(
        &mut self,
        key: KeyEvent,
        ops: &dyn NewFlowOps,
    ) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::CommitPicker;
            }
            KeyCode::Backspace => {
                self.commit_search_input.pop();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.commit_search_input.push(character);
            }
            KeyCode::Enter => {
                let query = self.commit_search_input.trim().to_string();
                self.load_commits(ops, &query)?;
                self.step = Step::CommitPicker;
            }
            _ => {}
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_extras(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        if self.extras.editing_filter {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.extras.toggle_filter_editing();
                }
                _ => {
                    self.extras.edit_filter(key);
                }
            }
            return Ok(FlowSignal::Continue);
        }

        match key.code {
            KeyCode::Esc => {
                self.step = match self.start_point {
                    Some(NewStartPoint::CurrentBranch) => Step::StartPointMode,
                    Some(NewStartPoint::Branch(_)) => Step::BranchPicker,
                    Some(NewStartPoint::Commit(_)) => Step::CommitPicker,
                    None => Step::StartPointMode,
                };
            }
            KeyCode::Up => self.extras.move_up(),
            KeyCode::Down => self.extras.move_down(),
            KeyCode::Enter | KeyCode::Char(' ') => self.extras.toggle_current(),
            KeyCode::Tab => {
                self.connect_yes_selected = true;
                self.step = Step::ConnectNow;
            }
            KeyCode::Char('/') => self.extras.toggle_filter_editing(),
            KeyCode::Char('a') => self.extras.select_all(),
            KeyCode::Char('n') => self.extras.select_none(),
            _ => {}
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_connect_now(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::ExtrasPicker;
            }
            KeyCode::Left | KeyCode::Up => {
                self.connect_yes_selected = true;
            }
            KeyCode::Right | KeyCode::Down => {
                self.connect_yes_selected = false;
            }
            KeyCode::Enter => {
                self.step = Step::Review;
            }
            _ => {}
        }
        Ok(FlowSignal::Continue)
    }

    fn on_key_review(&mut self, key: KeyEvent, ops: &dyn NewFlowOps) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc => {
                self.step = Step::ConnectNow;
            }
            KeyCode::Enter => {
                let Some(start_point) = self.start_point.clone() else {
                    return Ok(FlowSignal::Continue);
                };

                let request = NewRequest {
                    cwd: self.cwd.clone(),
                    worktree_name: self.name_input.clone(),
                    start_point,
                    add_worktrees_gitignore_entry: !self.prepare.gitignore_has_worktrees_entry
                        && self.gitignore_yes_selected,
                    selected_extras: self.extras.selected_for_copy(),
                    connect_now: self.connect_yes_selected,
                };

                match ops.execute_new(request) {
                    Ok(result) => {
                        self.success = Some(result);
                        self.step = Step::Success;
                        self.error_message = None;
                    }
                    Err(error) => {
                        self.error_message = Some(error.to_string());
                        self.step = Step::ErrorScreen;
                    }
                }
            }
            _ => {}
        }

        Ok(FlowSignal::Continue)
    }

    fn on_key_success(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => Ok(FlowSignal::Exit(UiExit::Completed)),
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn on_key_error(&mut self, key: KeyEvent) -> Result<FlowSignal> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.step = Step::Review;
                Ok(FlowSignal::Continue)
            }
            _ => Ok(FlowSignal::Continue),
        }
    }

    fn load_branches(&mut self, ops: &dyn NewFlowOps, query: &str) -> Result<()> {
        let items = ops
            .query_branches(&self.prepare.repo_root, query)
            .with_context(|| "failed to load branch list".to_string())?;
        self.branch_picker = Some(BranchPickerState {
            query: if query.trim().is_empty() {
                None
            } else {
                Some(query.trim().to_string())
            },
            items,
            selected: 0,
        });
        Ok(())
    }

    fn load_commits(&mut self, ops: &dyn NewFlowOps, query: &str) -> Result<()> {
        let items = ops
            .query_commits(&self.prepare.repo_root, query, 50)
            .with_context(|| "failed to load commit list".to_string())?;
        self.commit_picker = Some(CommitPickerState {
            query: if query.trim().is_empty() {
                None
            } else {
                Some(query.trim().to_string())
            },
            items,
            selected: 0,
        });
        Ok(())
    }

    fn render_gitignore_decision(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(3)])
            .areas(area);

        let prompt = Paragraph::new(format!(
            "Add 'worktrees/' to this repo's .gitignore?\n\nSelection: {}\nUse Left/Right (or Up/Down), Enter to continue.",
            if self.gitignore_yes_selected { "Yes" } else { "No" }
        ))
        .block(Block::default().borders(Borders::ALL).title("New: .gitignore"));
        frame.render_widget(prompt, body);

        let keys = Paragraph::new("Esc: exit flow    Ctrl+C: cancel")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_name_input(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(3)])
            .areas(area);

        let mut lines = vec![
            Line::from("Worktree name"),
            Line::from(""),
            Line::from(format!("> {}", self.name_input)),
            Line::from(""),
            Line::from("Rule: ^[a-z0-9][a-z0-9_-]{0,47}$"),
        ];

        if let Some(error) = &self.name_error {
            lines.push(Line::from(""));
            lines.push(Line::from(format!("Invalid: {error}")));
        }

        let paragraph =
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("New: Name"));
        frame.render_widget(paragraph, body);

        let keys =
            Paragraph::new("Type to edit    Enter: continue    Backspace: delete    Esc: back")
                .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_start_mode(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let items = vec![
            ListItem::new("From current branch"),
            ListItem::new("From other branch"),
            ListItem::new("From commit"),
        ];
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("New: Start point"),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            );

        let mut state = ListState::default();
        state.select(Some(self.start_mode_selected));
        frame.render_stateful_widget(list, body, &mut state);

        let keys = Paragraph::new("Up/Down: move    Enter: select    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_branch_picker(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let picker = self.branch_picker.as_ref();
        let mut rows = vec![ListItem::new("Search branches...")];
        if let Some(picker) = picker {
            if picker.query.is_some() {
                rows.push(ListItem::new("Show all branches"));
            }
            rows.extend(
                picker
                    .items
                    .iter()
                    .map(|branch| ListItem::new(branch.display.clone())),
            );
        }

        if rows.len() == 1 {
            rows.push(ListItem::new("No branches found"));
        }

        let list = List::new(rows)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("New: Branch picker"),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        let mut state = ListState::default();
        state.select(Some(picker.map(|value| value.selected).unwrap_or(0)));
        frame.render_stateful_widget(list, body, &mut state);

        let keys = Paragraph::new("Enter: choose    Up/Down: move    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_branch_search_input(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(3)])
            .areas(area);

        let paragraph = Paragraph::new(format!(
            "Enter branch filter and press Enter.\n\n> {}",
            self.branch_search_input
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("New: Branch search"),
        );
        frame.render_widget(paragraph, body);

        let keys = Paragraph::new("Type: filter    Enter: apply    Backspace: delete    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_commit_picker(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let picker = self.commit_picker.as_ref();
        let mut rows = vec![ListItem::new("Search commits by hash...")];
        if let Some(picker) = picker {
            if picker.query.is_some() {
                rows.push(ListItem::new("Show latest 50 commits"));
            }
            rows.extend(
                picker
                    .items
                    .iter()
                    .map(|commit| ListItem::new(commit.display.clone())),
            );
        }

        if rows.len() == 1 {
            rows.push(ListItem::new("No commits found"));
        }

        let list = List::new(rows)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("New: Commit picker"),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            );
        let mut state = ListState::default();
        state.select(Some(picker.map(|value| value.selected).unwrap_or(0)));
        frame.render_stateful_widget(list, body, &mut state);

        let keys = Paragraph::new("Enter: choose    Up/Down: move    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_commit_search_input(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(3)])
            .areas(area);

        let paragraph = Paragraph::new(format!(
            "Enter commit hash filter and press Enter.\n\n> {}",
            self.commit_search_input
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("New: Commit search"),
        );
        frame.render_widget(paragraph, body);

        let keys = Paragraph::new("Type: filter    Enter: apply    Backspace: delete    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_extras_picker(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(3)])
            .areas(area);

        let mut items = Vec::<ListItem<'_>>::new();
        for row in &self.extras.visible {
            let Some(node) = self.extras.nodes.get(&row.key) else {
                continue;
            };
            let prefix = "  ".repeat(row.depth);
            let marker = self.extras.mark_for(&row.key);
            let icon = if node.is_dir { "[D]" } else { "[F]" };
            let line = format!("{prefix}{marker} {icon} {}", node.label);
            items.push(ListItem::new(line));
        }

        if items.is_empty() {
            items.push(ListItem::new("No extra files or directories were found."));
        }

        let title = if self.extras.editing_filter {
            format!("New: Extras (filter: {}_)", self.extras.filter)
        } else {
            format!("New: Extras (filter: {})", self.extras.filter)
        };

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            );
        let mut state = ListState::default();
        if !self.extras.visible.is_empty() {
            state.select(Some(self.extras.cursor));
        }
        frame.render_stateful_widget(list, body, &mut state);

        let key_label = if self.extras.editing_filter {
            "Type: filter    Enter/Esc: finish filter edit"
        } else {
            "Up/Down: move    Enter/Space: toggle    a: all    n: none    /: filter    Tab: continue    Esc: back"
        };
        let keys =
            Paragraph::new(key_label).block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_connect_now(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(3)])
            .areas(area);

        let paragraph = Paragraph::new(format!(
            "Connect to the tmux session now?\n\nSelection: {}",
            if self.connect_yes_selected {
                "Yes"
            } else {
                "No"
            }
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("New: Connect now"),
        );
        frame.render_widget(paragraph, body);

        let keys =
            Paragraph::new("Left/Right (or Up/Down): choose    Enter: continue    Esc: back")
                .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_review(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let [body, footer] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(3)])
            .areas(area);

        let start_point = match &self.start_point {
            Some(NewStartPoint::CurrentBranch) => "From current branch".to_string(),
            Some(NewStartPoint::Branch(name)) => format!("From branch: {name}"),
            Some(NewStartPoint::Commit(hash)) => format!("From commit: {hash}"),
            None => "UNCONFIRMED".to_string(),
        };

        let extras_count = self.extras.selected_for_copy().len();
        let review = Paragraph::new(format!(
            "Review before create:\n\nworktree: {}\nstart point: {}\nadd .gitignore entry: {}\nselected extras: {}\nconnect now: {}\n",
            self.name_input,
            start_point,
            (!self.prepare.gitignore_has_worktrees_entry && self.gitignore_yes_selected),
            extras_count,
            self.connect_yes_selected
        ))
        .block(Block::default().borders(Borders::ALL).title("New: Review"));
        frame.render_widget(review, body);

        let keys = Paragraph::new("Enter: create worktree    Esc: back")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(keys, footer);
    }

    fn render_success(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let popup = centered_rect(80, 60, area);
        frame.render_widget(Clear, popup);

        let success = if let Some(result) = &self.success {
            format!(
                "Worktree created.\n\nPath: {}\nSession: {}\nAttach: {}\nConnected now: {}\n\nEnter/Esc to exit.",
                result.worktree_path.display(),
                result.session_name,
                result.attach_command,
                result.connected_now
            )
        } else {
            "Worktree created.\n\nEnter/Esc to exit.".to_string()
        };

        let widget =
            Paragraph::new(success).block(Block::default().borders(Borders::ALL).title("Success"));
        frame.render_widget(widget, popup);
    }

    fn render_error(&self, frame: &mut ratatui::Frame<'_>) {
        let area = frame.area();
        let popup = centered_rect(85, 70, area);
        frame.render_widget(Clear, popup);

        let message = self
            .error_message
            .as_deref()
            .unwrap_or("Unknown error while creating worktree.");
        let widget = Paragraph::new(format!(
            "Failed to create worktree.\n\n{message}\n\nEnter/Esc to return to review."
        ))
        .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(widget, popup);
    }
}

pub fn run_new_flow(app: &App<'_>, cwd: &Path) -> Result<UiExit> {
    let mut flow = NewFlow::new(app, cwd)?;
    let mut session = TerminalSession::enter()?;

    loop {
        session.draw(|frame| flow.render(frame))?;

        let event = event::read().context("failed to read terminal event")?;
        let Event::Key(key) = event else {
            continue;
        };

        if is_ctrl_c(key) {
            return Ok(UiExit::Canceled);
        }

        match flow.on_key(key, app)? {
            FlowSignal::Continue => {}
            FlowSignal::Exit(exit) => return Ok(exit),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use seshmux_app::{NewPrepare, NewRequest, NewResult, NewStartPoint};
    use seshmux_core::git::{BranchRef, BranchSource, CommitRef};

    use super::{FlowSignal, NewFlow, NewFlowOps, Step};

    struct FakeOps {
        prepare: NewPrepare,
        branches: Vec<BranchRef>,
        latest_commits: Vec<CommitRef>,
        searched_commits: Vec<CommitRef>,
        extras: Vec<PathBuf>,
        execute_calls: RefCell<Vec<NewRequest>>,
    }

    impl FakeOps {
        fn new(repo_root: PathBuf) -> Self {
            Self {
                prepare: NewPrepare {
                    repo_root: repo_root.clone(),
                    worktrees_dir: repo_root.join("worktrees"),
                    gitignore_has_worktrees_entry: false,
                },
                branches: vec![BranchRef {
                    name: "main".to_string(),
                    source: BranchSource::Local,
                    display: "main [local]".to_string(),
                }],
                latest_commits: vec![CommitRef {
                    hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    short_hash: "aaaaaaa".to_string(),
                    subject: "first".to_string(),
                    display: "aaaaaaa first".to_string(),
                }],
                searched_commits: vec![CommitRef {
                    hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                    short_hash: "bbbbbbb".to_string(),
                    subject: "second".to_string(),
                    display: "bbbbbbb second".to_string(),
                }],
                extras: Vec::new(),
                execute_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl NewFlowOps for FakeOps {
        fn prepare(&self, _cwd: &Path) -> Result<NewPrepare> {
            Ok(self.prepare.clone())
        }

        fn query_branches(&self, _repo_root: &Path, _query: &str) -> Result<Vec<BranchRef>> {
            Ok(self.branches.clone())
        }

        fn query_commits(
            &self,
            _repo_root: &Path,
            query: &str,
            _limit: usize,
        ) -> Result<Vec<CommitRef>> {
            if query.trim().is_empty() {
                Ok(self.latest_commits.clone())
            } else {
                Ok(self.searched_commits.clone())
            }
        }

        fn list_extras(&self, _repo_root: &Path) -> Result<Vec<PathBuf>> {
            Ok(self.extras.clone())
        }

        fn execute_new(&self, request: NewRequest) -> Result<NewResult> {
            self.execute_calls.borrow_mut().push(request.clone());
            Ok(NewResult {
                repo_root: request.cwd.clone(),
                worktrees_dir: request.cwd.join("worktrees"),
                worktree_path: request.cwd.join("worktrees").join(&request.worktree_name),
                branch_name: request.worktree_name.clone(),
                session_name: format!("repo/{}", request.worktree_name),
                attach_command: format!("tmux attach-session -t repo/{}", request.worktree_name),
                connected_now: request.connect_now,
            })
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn esc_on_first_step_exits_flow() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        let signal = flow.on_key(key(KeyCode::Esc), &ops).expect("signal");
        assert_eq!(signal, FlowSignal::Exit(super::UiExit::BackAtRoot));
    }

    #[test]
    fn new_flow_requires_at_least_one_commit() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.latest_commits.clear();

        let error = NewFlow::new(&ops, &repo_root).expect_err("flow should fail");
        assert!(error.to_string().contains("repository has no commits yet"));
    }

    #[test]
    fn commit_picker_shows_latest_then_search_results() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "test".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Down), &ops)
            .expect("select branch mode");
        flow.on_key(key(KeyCode::Down), &ops)
            .expect("select commit mode");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("enter commit picker");

        assert_eq!(flow.step, Step::CommitPicker);
        assert_eq!(
            flow.commit_picker
                .as_ref()
                .expect("commit picker")
                .items
                .first()
                .expect("latest")
                .short_hash,
            "aaaaaaa"
        );

        flow.on_key(key(KeyCode::Enter), &ops).expect("open search");
        assert_eq!(flow.step, Step::CommitSearchInput);

        for character in "bbb".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("type search");
        }
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("apply search");
        assert_eq!(flow.step, Step::CommitPicker);
        assert_eq!(
            flow.commit_picker
                .as_ref()
                .expect("commit picker")
                .items
                .first()
                .expect("searched")
                .short_hash,
            "bbbbbbb"
        );
    }

    #[test]
    fn extras_filter_preserves_selection_state() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join("dir/sub")).expect("dirs");
        std::fs::write(repo_root.join("dir/sub/one.txt"), "one").expect("file one");
        std::fs::write(repo_root.join("dir/sub/two.txt"), "two").expect("file two");

        let mut ops = FakeOps::new(repo_root.clone());
        ops.extras = vec![PathBuf::from("dir")];

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "abc".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("current mode");

        assert_eq!(flow.step, Step::ExtrasPicker);
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("toggle root dir");
        let before = flow.extras.selected_for_copy();
        assert!(before.iter().any(|path| path == &PathBuf::from("dir")));

        flow.on_key(key(KeyCode::Char('/')), &ops)
            .expect("enter filter edit");
        for character in "one".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("type filter");
        }
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("finish filter");

        let after = flow.extras.selected_for_copy();
        assert_eq!(before, after);
    }

    #[test]
    fn review_executes_with_collected_inputs() {
        let temp = tempfile::tempdir().expect("temp dir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("repo");
        let ops = FakeOps::new(repo_root.clone());

        let mut flow = NewFlow::new(&ops, &repo_root).expect("flow");
        flow.on_key(key(KeyCode::Enter), &ops).expect("gitignore");
        for character in "feature1".chars() {
            flow.on_key(key(KeyCode::Char(character)), &ops)
                .expect("name");
        }
        flow.on_key(key(KeyCode::Enter), &ops).expect("name enter");
        flow.on_key(key(KeyCode::Enter), &ops)
            .expect("start current");
        flow.on_key(key(KeyCode::Tab), &ops)
            .expect("extras continue");
        flow.on_key(key(KeyCode::Right), &ops).expect("connect no");
        flow.on_key(key(KeyCode::Enter), &ops).expect("review");
        flow.on_key(key(KeyCode::Enter), &ops).expect("execute");

        assert_eq!(flow.step, Step::Success);
        let calls = ops.execute_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].worktree_name, "feature1");
        assert_eq!(calls[0].start_point, NewStartPoint::CurrentBranch);
        assert!(!calls[0].connect_now);
    }
}
