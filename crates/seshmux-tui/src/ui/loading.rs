use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use ratatui::Frame;
use ratatui::text::{Line, Text};
use seshmux_core::command_runner::SystemCommandRunner;

use crate::new_flow::extras::{ExtrasIndex, build_extras_index_from_paths};
use crate::theme;
use crate::ui::modal::{ModalSpec, render_modal};

const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadingState {
    frame_index: usize,
}

impl LoadingState {
    pub(crate) fn next_frame(&mut self) {
        self.frame_index = (self.frame_index + 1) % FRAMES.len();
    }

    fn current_frame(&self) -> &'static str {
        FRAMES[self.frame_index]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlaggedBucket {
    pub(crate) bucket: String,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BucketPlan {
    pub(crate) flagged: Vec<FlaggedBucket>,
}

impl BucketPlan {
    pub(crate) fn flagged_count(&self) -> usize {
        self.flagged.len()
    }
}

#[derive(Debug)]
pub(crate) enum ExtrasLoadEvent {
    Collecting,
    Classifying {
        candidate_count: usize,
    },
    AwaitingSkipDecision {
        flagged_bucket_count: usize,
    },
    DoneCollect {
        token: u64,
        candidates: Vec<PathBuf>,
        plan: BucketPlan,
    },
    Building {
        filtered_count: usize,
    },
    Done {
        token: u64,
        result: Result<ExtrasIndex, String>,
    },
}

pub(crate) trait ExtrasLoader: Send + Sync {
    fn spawn_collect_and_classify(
        &self,
        repo_root: PathBuf,
        token: u64,
        skip_rules: BTreeSet<String>,
    ) -> Receiver<ExtrasLoadEvent>;

    fn spawn_build(&self, candidates: Vec<PathBuf>, token: u64) -> Receiver<ExtrasLoadEvent>;
}

#[derive(Debug, Default)]
pub(crate) struct SystemExtrasLoader;

impl SystemExtrasLoader {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl ExtrasLoader for SystemExtrasLoader {
    fn spawn_collect_and_classify(
        &self,
        repo_root: PathBuf,
        token: u64,
        skip_rules: BTreeSet<String>,
    ) -> Receiver<ExtrasLoadEvent> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ExtrasLoadEvent::Collecting);

            let runner = SystemCommandRunner::new();
            let candidates = match seshmux_core::extras::list_extra_candidates(&repo_root, &runner)
            {
                Ok(candidates) => candidates,
                Err(error) => {
                    let _ = sender.send(ExtrasLoadEvent::Done {
                        token,
                        result: Err(format!("{error:#}")),
                    });
                    return;
                }
            };

            let _ = sender.send(ExtrasLoadEvent::Classifying {
                candidate_count: candidates.len(),
            });

            let flagged = seshmux_core::extras::classify_flagged_buckets(&candidates, &skip_rules)
                .into_iter()
                .map(|(bucket, count)| FlaggedBucket { bucket, count })
                .collect::<Vec<_>>();
            let plan = BucketPlan { flagged };

            let _ = sender.send(ExtrasLoadEvent::AwaitingSkipDecision {
                flagged_bucket_count: plan.flagged_count(),
            });
            let _ = sender.send(ExtrasLoadEvent::DoneCollect {
                token,
                candidates,
                plan,
            });
        });

        receiver
    }

    fn spawn_build(&self, candidates: Vec<PathBuf>, token: u64) -> Receiver<ExtrasLoadEvent> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ExtrasLoadEvent::Building {
                filtered_count: candidates.len(),
            });
            let result =
                build_extras_index_from_paths(&candidates).map_err(|error| format!("{error:#}"));
            let _ = sender.send(ExtrasLoadEvent::Done { token, result });
        });
        receiver
    }
}

pub(crate) fn render_loading_modal(
    frame: &mut Frame<'_>,
    title: &str,
    message: &str,
    key_hint: &str,
    loading: &LoadingState,
) {
    let body = Text::from(vec![
        Line::from(""),
        Line::from(format!("{} {}", loading.current_frame(), message)),
    ]);
    render_modal(
        frame,
        ModalSpec {
            title,
            title_style: Some(theme::focus_prompt()),
            body,
            key_hint: Some(key_hint),
            width_pct: 72,
            height_pct: 42,
        },
    );
}
