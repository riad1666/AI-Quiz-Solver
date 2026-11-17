use crate::ai::response::AiAnswerResponse;
use crate::config::settings::AppSettings;
use crate::interaction::controls::MatchResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyDecision {
    Allow,
    RequireConfirmation(String),
    Deny(String),
}

/// Evaluates whether auto-filling is permitted and unambiguous.
/// Never allows clicking submit/next/finish controls.
pub fn evaluate_fill_safety(
    ai: &AiAnswerResponse,
    match_result: &MatchResult,
    settings: &AppSettings,
) -> SafetyDecision {
    if !settings.auto_fill_enabled {
        return SafetyDecision::Deny("Auto-fill is disabled in settings".to_string());
    }

    if ai.confidence < settings.confidence_threshold {
        return SafetyDecision::RequireConfirmation(format!(
            "AI confidence ({:.0}%) is below threshold ({:.0}%)",
            ai.confidence * 100.0,
            settings.confidence_threshold * 100.0
        ));
    }

    match match_result {
        MatchResult::Unambiguous(candidate) => {
            if is_forbidden_submit_label(&candidate.name)
                || is_forbidden_submit_label(&candidate.automation_id)
            {
                return SafetyDecision::Deny(format!(
                    "Refusing to interact with potential submission/navigation control '{}'",
                    candidate.name
                ));
            }
            SafetyDecision::Allow
        }
        MatchResult::Ambiguous(reason) => SafetyDecision::RequireConfirmation(reason.clone()),
        MatchResult::NotFound => {
            SafetyDecision::Deny("No matching answer control found in selected region".to_string())
        }
    }
}

pub fn is_forbidden_submit_label(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "submit",
        "finish",
        "turn in",
        "send quiz",
        "complete exam",
        "check answer",
        "next question",
        "save and continue",
        "grade",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}
