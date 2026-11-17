use std::collections::HashSet;

use crate::ai::response::{AiAnswerResponse, QuestionCategory};
use crate::interaction::safety::is_forbidden_submit_label;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Edit,
    Document,
    RadioButton,
    CheckBox,
    ComboBox,
    ListItem,
    ChoiceButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenBounds {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlCandidate {
    pub index: usize,
    pub kind: ControlKind,
    pub name: String,
    pub automation_id: String,
    pub center_x: i32,
    pub center_y: i32,
    pub is_enabled: bool,
    pub is_offscreen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchResult {
    Unambiguous(ControlCandidate),
    Ambiguous(String),
    NotFound,
}

/// Filters controls to those whose center lies within `bounds` expanded by `margin_px`.
pub fn filter_candidates_in_region(
    candidates: &[ControlCandidate],
    bounds: ScreenBounds,
    margin_px: i32,
) -> Vec<ControlCandidate> {
    let min_x = bounds.left - margin_px;
    let max_x = bounds.left + bounds.width + margin_px;
    let min_y = bounds.top - margin_px;
    let max_y = bounds.top + bounds.height + margin_px;

    candidates
        .iter()
        .filter(|c| {
            c.center_x >= min_x
                && c.center_x <= max_x
                && c.center_y >= min_y
                && c.center_y <= max_y
        })
        .cloned()
        .collect()
}

/// Scores visible enabled controls against the AI's answer, `target_hint`, and question anchor,
/// excluding any control indices already claimed by earlier questions in the same batch.
pub fn match_target_control_excluding(
    candidates: &[ControlCandidate],
    ai: &AiAnswerResponse,
    anchor_x: i32,
    anchor_y: i32,
    used_indices: &HashSet<usize>,
) -> MatchResult {
    let expects_choice = matches!(
        ai.question_type,
        QuestionCategory::Mcq | QuestionCategory::TrueFalse
    );

    let mut scored: Vec<(i32, &ControlCandidate)> = candidates
        .iter()
        .filter(|c| c.is_enabled && !c.is_offscreen && !used_indices.contains(&c.index))
        .filter(|c| !is_forbidden_submit_label(&c.name))
        .filter_map(|c| {
            let score = score_candidate(c, ai, expects_choice, anchor_x, anchor_y);
            if score > 0 {
                Some((score, c))
            } else {
                None
            }
        })
        .collect();

    if scored.is_empty() {
        return MatchResult::NotFound;
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if scored.len() >= 2 && (scored[0].0 - scored[1].0) < 12 {
        return MatchResult::Ambiguous(format!(
            "Ambiguous target: '{}' and '{}' scored too closely ({} vs {})",
            scored[0].1.name, scored[1].1.name, scored[0].0, scored[1].0
        ));
    }

    MatchResult::Unambiguous(scored[0].1.clone())
}

pub fn match_target_control(
    candidates: &[ControlCandidate],
    ai: &AiAnswerResponse,
    anchor_x: i32,
    anchor_y: i32,
) -> MatchResult {
    match_target_control_excluding(candidates, ai, anchor_x, anchor_y, &HashSet::new())
}

fn score_candidate(
    c: &ControlCandidate,
    ai: &AiAnswerResponse,
    expects_choice: bool,
    anchor_x: i32,
    anchor_y: i32,
) -> i32 {
    let mut score = 0i32;
    let name_lower = c.name.to_ascii_lowercase();
    let id_lower = c.automation_id.to_ascii_lowercase();

    if expects_choice {
        match c.kind {
            ControlKind::RadioButton | ControlKind::CheckBox | ControlKind::ListItem => {
                score += 45;
            }
            ControlKind::ChoiceButton => score += 30,
            ControlKind::ComboBox => score += 25,
            ControlKind::Edit | ControlKind::Document => return 0,
        }

        let ans_lower = ai.answer.trim().to_ascii_lowercase();
        let mut text_matched = false;

        if !ans_lower.is_empty() && name_lower == ans_lower {
            score += 85;
            text_matched = true;
        } else if !ans_lower.is_empty()
            && (name_lower.starts_with(&format!("{ans_lower}."))
                || name_lower.starts_with(&format!("{ans_lower})"))
                || name_lower.starts_with(&format!("{ans_lower}:"))
                || name_lower.starts_with(&format!("option {ans_lower}")))
        {
            score += 75;
            text_matched = true;
        }

        if let Some(ans_text) = &ai.answer_text {
            let text_lower = ans_text.trim().to_ascii_lowercase();
            if !text_lower.is_empty() && name_lower.contains(&text_lower) {
                score += 80;
                text_matched = true;
            }
        }

        // Do not click generic buttons unless their label actually matches the choice text.
        if c.kind == ControlKind::ChoiceButton && !text_matched {
            return 0;
        }
    } else {
        match c.kind {
            ControlKind::Edit | ControlKind::Document => score += 65,
            ControlKind::ComboBox => score += 25,
            _ => return 0,
        }
    }

    if let Some(hint) = &ai.target_hint {
        let hint_lower = hint.to_ascii_lowercase();
        for token in hint_lower.split_whitespace().filter(|t| t.len() >= 3) {
            if name_lower.contains(token) || id_lower.contains(token) {
                score += 15;
            }
        }
    }

    // Proximity bonus to this specific question's anchor (up to +40)
    let dx = (c.center_x - anchor_x).abs();
    let dy = (c.center_y - anchor_y).abs();
    let dist = dx + dy;
    if dist < 800 {
        score += (40 - (dist / 20)).max(0);
    }

    score
}
