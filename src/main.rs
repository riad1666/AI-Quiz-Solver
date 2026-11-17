#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};

use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use knu_diagnosis::config::settings::AppSettings;
use knu_diagnosis::core::state::WorkflowCoordinator;
use knu_diagnosis::storage::db::HistoryDb;
use knu_diagnosis::ui::tray::{run_tray_app, LastQaSummary};

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn main() {
    init_logging();
    info!("Starting KNU Diagnosis (Kangwon National University)");

    let settings_path = match AppSettings::default_path() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("Failed to resolve config directory: {err}");
            std::process::exit(1);
        }
    };

    let settings = match AppSettings::load_or_create(&settings_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Failed to load settings from {}: {err}", settings_path.display());
            std::process::exit(1);
        }
    };

    let history_db = match HistoryDb::default_path().and_then(|p| HistoryDb::open(&p)) {
        Ok(db) => Some(Arc::new(Mutex::new(db))),
        Err(err) => {
            error!("Failed to open SQLite history database: {err}");
            None
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to initialize Tokio runtime");

    let _rt_guard = runtime.enter();

    let coordinator = WorkflowCoordinator::new();
    let last_qa = Arc::new(Mutex::new(None::<LastQaSummary>));

    if let Err(err) = run_tray_app(coordinator, settings, last_qa, history_db) {
        eprintln!("KNU Diagnosis exited with UI error: {err}");
        std::process::exit(1);
    }

    info!("KNU Diagnosis exited cleanly");
}
