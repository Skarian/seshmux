use std::backtrace::Backtrace;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, Once, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};

#[derive(Default)]
struct RuntimeDiagnostics {
    path: Option<PathBuf>,
    file: Option<File>,
}

static HOOK_ONCE: Once = Once::new();
static STATE: OnceLock<Mutex<RuntimeDiagnostics>> = OnceLock::new();

fn diagnostics_state() -> &'static Mutex<RuntimeDiagnostics> {
    STATE.get_or_init(|| Mutex::new(RuntimeDiagnostics::default()))
}

pub struct DiagnosticsSession {
    path: Option<PathBuf>,
}

impl DiagnosticsSession {
    pub fn initialize(enabled: bool) -> Result<Self> {
        install_panic_hook();
        if !enabled {
            let mut state = diagnostics_state()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.path = None;
            state.file = None;
            return Ok(Self { path: None });
        }

        let path = create_diagnostics_log_path()?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to create diagnostics log at {}", path.display()))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        writeln!(
            file,
            "seshmux diagnostics start\nversion={}\nstart_epoch_ms={now}\npid={}",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        )
        .with_context(|| format!("failed to write diagnostics header to {}", path.display()))?;
        writeln!(file, "argv={:?}", std::env::args().collect::<Vec<String>>())
            .with_context(|| format!("failed to write diagnostics args to {}", path.display()))?;
        file.flush()
            .with_context(|| format!("failed to flush diagnostics header to {}", path.display()))?;

        let mut state = diagnostics_state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.path = Some(path.clone());
        state.file = Some(file);

        Ok(Self { path: Some(path) })
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn record<S: AsRef<str>>(&self, entry: S) {
        append_line(entry.as_ref());
    }
}

fn install_panic_hook() {
    HOOK_ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|panic_info| {
            let payload = panic_payload(panic_info);
            let location = panic_info
                .location()
                .map(|value| format!("{}:{}:{}", value.file(), value.line(), value.column()))
                .unwrap_or_else(|| "UNCONFIRMED".to_string());
            let backtrace = Backtrace::force_capture();

            append_line("panic captured");
            append_line(format!("panic_message={payload}"));
            append_line(format!("panic_location={location}"));
            append_line(format!("panic_backtrace={backtrace:?}"));

            let path = diagnostics_state()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .path
                .clone();

            eprintln!("Fatal internal error in seshmux.");
            match path {
                Some(path) => {
                    eprintln!("Diagnostics written to {}", path.display());
                }
                None => {
                    eprintln!("Run `seshmux --diagnostics` to capture a diagnostics log.");
                }
            }
        }));
    });
}

fn panic_payload(panic_info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(payload) = panic_info.payload().downcast_ref::<&str>() {
        return (*payload).to_string();
    }
    if let Some(payload) = panic_info.payload().downcast_ref::<String>() {
        return payload.clone();
    }
    "unknown panic payload".to_string()
}

fn append_line<S: AsRef<str>>(line: S) {
    let mut state = diagnostics_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(file) = state.file.as_mut() else {
        return;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let _ = writeln!(file, "[{now}] {}", line.as_ref());
    let _ = file.flush();
}

fn create_diagnostics_log_path() -> Result<PathBuf> {
    let config_path = seshmux_core::config::resolve_config_path()
        .context("failed to resolve seshmux config path for diagnostics")?;
    let config_dir = config_path.parent().ok_or_else(|| {
        anyhow!(
            "failed to resolve diagnostics directory from config path {}",
            config_path.display()
        )
    })?;

    let diagnostics_dir = config_dir.join("diagnostics");
    fs::create_dir_all(&diagnostics_dir).with_context(|| {
        format!(
            "failed to create diagnostics directory {}",
            diagnostics_dir.display()
        )
    })?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(diagnostics_dir.join(format!("{now}.log")))
}
