use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowState {
    Idle,
    Capturing,
    Analyzing,
    LocatingControl,
    Filling,
    Completed,
    Failed,
    Cancelled,
}

impl WorkflowState {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Capturing | Self::Analyzing | Self::LocatingControl | Self::Filling
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Capturing => "Capturing screen...",
            Self::Analyzing => "Analyzing with Vision AI...",
            Self::LocatingControl => "Locating answer field...",
            Self::Filling => "Filling answer...",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

/// Thread-safe state coordinator preventing overlapping workflow executions.
#[derive(Debug, Clone)]
pub struct WorkflowCoordinator {
    state: Arc<Mutex<WorkflowState>>,
    cancel_requested: Arc<AtomicBool>,
}

impl Default for WorkflowCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowCoordinator {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(WorkflowState::Idle)),
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Attempts to start a new workflow. Returns `false` if one is already running.
    pub fn try_begin(&self) -> bool {
        let mut guard = self.state.lock().expect("workflow state mutex poisoned");
        if guard.is_busy() {
            return false;
        }
        self.cancel_requested.store(false, Ordering::SeqCst);
        *guard = WorkflowState::Capturing;
        true
    }

    pub fn set_state(&self, next: WorkflowState) {
        let mut guard = self.state.lock().expect("workflow state mutex poisoned");
        *guard = next;
    }

    pub fn state(&self) -> WorkflowState {
        *self.state.lock().expect("workflow state mutex poisoned")
    }

    pub fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        let mut guard = self.state.lock().expect("workflow state mutex poisoned");
        if guard.is_busy() {
            *guard = WorkflowState::Cancelled;
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prevents_overlapping_runs_and_supports_cancel() {
        let coord = WorkflowCoordinator::new();
        assert_eq!(coord.state(), WorkflowState::Idle);
        assert!(coord.try_begin());
        assert!(!coord.try_begin());

        coord.request_cancel();
        assert!(coord.is_cancelled());
        assert_eq!(coord.state(), WorkflowState::Cancelled);

        // After cancellation, a new run can begin.
        assert!(coord.try_begin());
        assert!(!coord.is_cancelled());
    }
}
