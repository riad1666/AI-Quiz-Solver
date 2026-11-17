use std::collections::HashSet;

use knu_diagnosis::ai::response::{AiAnswerResponse, QuestionCategory, VerificationKind};
use knu_diagnosis::config::settings::AppSettings;
use knu_diagnosis::interaction::controls::{
    filter_candidates_in_region, match_target_control, match_target_control_excluding,
    ControlCandidate, ControlKind, MatchResult, ScreenBounds,
};
use knu_diagnosis::interaction::safety::{evaluate_fill_safety, SafetyDecision};
use knu_diagnosis::interaction::uia::is_point_in_bounds;

fn sample_mcq_ai(confidence: f32) -> AiAnswerResponse {
    AiAnswerResponse {
        question_number: Some("3".to_string()),
        question_type: QuestionCategory::Mcq,
        question_text: "Which collection is mutable?".to_string(),
        answer: "B".to_string(),
        answer_text: Some("List".to_string()),
        target_hint: Some("Option B: List".to_string()),
        action: None,
        target_point: None,
        explanation: "Lists are mutable.".to_string(),
        confidence,
        verification: VerificationKind::AiReasoned,
        code: None,
        relative_box: None,
        needs_followup: false,
        followup_prompt: None,
    }
}

#[test]
fn matches_unambiguous_mcq_option() {
    let ai = sample_mcq_ai(0.95);
    let candidates = vec![
        ControlCandidate {
            index: 0,
            kind: ControlKind::RadioButton,
            name: "A. Tuple".to_string(),
            automation_id: "opt_a".to_string(),
            center_x: 300,
            center_y: 300,
            is_enabled: true,
            is_offscreen: false,
        },
        ControlCandidate {
            index: 1,
            kind: ControlKind::RadioButton,
            name: "B. List".to_string(),
            automation_id: "opt_b".to_string(),
            center_x: 300,
            center_y: 340,
            is_enabled: true,
            is_offscreen: false,
        },
    ];

    let m = match_target_control(&candidates, &ai, 310, 335);
    match m {
        MatchResult::Unambiguous(c) => assert_eq!(c.name, "B. List"),
        other => panic!("expected unambiguous match, got {other:?}"),
    }

    let settings = AppSettings::default();
    assert_eq!(
        evaluate_fill_safety(&ai, &match_target_control(&candidates, &ai, 310, 335), &settings),
        SafetyDecision::Allow
    );
}

#[test]
fn scopes_candidates_to_region_and_prevents_multi_question_collision() {
    let candidates = vec![
        ControlCandidate {
            index: 0,
            kind: ControlKind::Edit,
            name: "Unrelated Outside Box".to_string(),
            automation_id: "out".to_string(),
            center_x: 50,
            center_y: 50,
            is_enabled: true,
            is_offscreen: false,
        },
        ControlCandidate {
            index: 1,
            kind: ControlKind::Edit,
            name: "Answer".to_string(),
            automation_id: "q1_input".to_string(),
            center_x: 400,
            center_y: 300,
            is_enabled: true,
            is_offscreen: false,
        },
        ControlCandidate {
            index: 2,
            kind: ControlKind::Edit,
            name: "Answer".to_string(),
            automation_id: "q2_input".to_string(),
            center_x: 400,
            center_y: 550,
            is_enabled: true,
            is_offscreen: false,
        },
    ];

    let bounds = ScreenBounds {
        left: 200,
        top: 200,
        width: 500,
        height: 500,
    };
    let scoped = filter_candidates_in_region(&candidates, bounds, 20);
    assert_eq!(scoped.len(), 2);

    let q1 = AiAnswerResponse {
        question_number: Some("1".to_string()),
        question_type: QuestionCategory::FillInBlank,
        question_text: "Q1".to_string(),
        answer: "42".to_string(),
        answer_text: None,
        target_hint: None,
        action: None,
        target_point: None,
        explanation: "42".to_string(),
        confidence: 0.95,
        verification: VerificationKind::AiReasoned,
        code: None,
        relative_box: Some([100, 200, 300, 600]),
        needs_followup: false,
        followup_prompt: None,
    };
    let (ax1, ay1) = q1.anchor_screen_coords(200, 200, 500, 500);
    let mut used = HashSet::new();
    let m1 = match_target_control_excluding(&scoped, &q1, ax1, ay1, &used);
    match m1 {
        MatchResult::Unambiguous(c) => {
            assert_eq!(c.index, 1);
            used.insert(c.index);
        }
        other => panic!("expected q1 to match index 1, got {other:?}"),
    }

    let q2 = AiAnswerResponse {
        question_number: Some("2".to_string()),
        relative_box: Some([600, 200, 800, 600]),
        ..q1
    };
    let (ax2, ay2) = q2.anchor_screen_coords(200, 200, 500, 500);
    let m2 = match_target_control_excluding(&scoped, &q2, ax2, ay2, &used);
    match m2 {
        MatchResult::Unambiguous(c) => assert_eq!(c.index, 2),
        other => panic!("expected q2 to match index 2, got {other:?}"),
    }
}

#[test]
fn rejects_low_confidence_and_submit_controls() {
    let low_conf = sample_mcq_ai(0.40);
    let candidates = vec![ControlCandidate {
        index: 0,
        kind: ControlKind::RadioButton,
        name: "B. List".to_string(),
        automation_id: "opt_b".to_string(),
        center_x: 300,
        center_y: 340,
        is_enabled: true,
        is_offscreen: false,
    }];
    let settings = AppSettings::default();
    let m = match_target_control(&candidates, &low_conf, 300, 340);
    assert!(matches!(
        evaluate_fill_safety(&low_conf, &m, &settings),
        SafetyDecision::RequireConfirmation(_)
    ));

    let submit_candidate = MatchResult::Unambiguous(ControlCandidate {
        index: 0,
        kind: ControlKind::ChoiceButton,
        name: "Submit Quiz".to_string(),
        automation_id: "btn_submit".to_string(),
        center_x: 300,
        center_y: 500,
        is_enabled: true,
        is_offscreen: false,
    });
    let high_conf = sample_mcq_ai(0.95);
    assert!(matches!(
        evaluate_fill_safety(&high_conf, &submit_candidate, &settings),
        SafetyDecision::Deny(_)
    ));
}

#[test]
fn detects_ambiguity_when_two_edits_tie() {
    let ai = AiAnswerResponse {
        question_number: None,
        question_type: QuestionCategory::FillInBlank,
        question_text: "Fill the blank".to_string(),
        answer: "42".to_string(),
        answer_text: None,
        target_hint: None,
        action: None,
        target_point: None,
        explanation: "Result is 42".to_string(),
        confidence: 0.92,
        verification: VerificationKind::AiReasoned,
        code: None,
        relative_box: None,
        needs_followup: false,
        followup_prompt: None,
    };
    let candidates = vec![
        ControlCandidate {
            index: 0,
            kind: ControlKind::Edit,
            name: "Answer".to_string(),
            automation_id: "q1".to_string(),
            center_x: 200,
            center_y: 200,
            is_enabled: true,
            is_offscreen: false,
        },
        ControlCandidate {
            index: 1,
            kind: ControlKind::Edit,
            name: "Answer".to_string(),
            automation_id: "q2".to_string(),
            center_x: 200,
            center_y: 220,
            is_enabled: true,
            is_offscreen: false,
        },
    ];
    let m = match_target_control(&candidates, &ai, 200, 210);
    assert!(matches!(m, MatchResult::Ambiguous(_)));
}

#[test]
fn verifies_vision_target_point_stays_inside_selected_bounds() {
    let bounds = ScreenBounds {
        left: 100,
        top: 150,
        width: 600,
        height: 400,
    };
    assert!(is_point_in_bounds(100, 150, bounds));
    assert!(is_point_in_bounds(450, 320, bounds));
    assert!(!is_point_in_bounds(50, 320, bounds));
    assert!(!is_point_in_bounds(450, 600, bounds));
}
