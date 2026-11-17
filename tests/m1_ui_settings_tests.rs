use knu_diagnosis::capture::hotkey::{format_hotkey, parse_hotkey};
use knu_diagnosis::config::settings::{
    AppSettings, CUSTOM_MODEL_OPTION, DEFAULT_COPY_EXPLANATIONS_HOTKEY, DEFAULT_REASONING_MODEL,
    DEFAULT_VISION_MODEL, REASONING_MODEL_PRESETS, VISION_MODEL_PRESETS,
};
use knu_diagnosis::ui::qa_viewer::{
    build_detail_spans, build_qa_item_spans, RichSpan, ANSWER_FONT_TWIPS, BODY_FONT_TWIPS,
};
use knu_diagnosis::ui::tray::{format_explanations_for_clipboard, LastQaSummary, QaDisplayItem};

#[test]
fn settings_exports_presets_and_ctrl_alt_c_defaults() {
    let defaults = AppSettings::default();
    assert_eq!(defaults.vision_model, DEFAULT_VISION_MODEL);
    assert_eq!(defaults.reasoning_model, DEFAULT_REASONING_MODEL);
    assert_eq!(
        defaults.copy_explanations_hotkey,
        DEFAULT_COPY_EXPLANATIONS_HOTKEY
    );
    assert_eq!(defaults.copy_explanations_hotkey, "Ctrl+Alt+C");

    assert!(VISION_MODEL_PRESETS.contains(&"z-ai/glm-5.3-flash"));
    assert!(VISION_MODEL_PRESETS.contains(&"anthropic/claude-opus-5.5"));
    assert!(VISION_MODEL_PRESETS.contains(&"google/gemini-2.5-flash"));
    assert_eq!(VISION_MODEL_PRESETS.last().copied(), Some(CUSTOM_MODEL_OPTION));

    assert!(REASONING_MODEL_PRESETS.contains(&"openai/gpt-4o-mini"));
    assert!(REASONING_MODEL_PRESETS.contains(&"deepseek/deepseek-v4-pro-0813"));
    assert_eq!(
        REASONING_MODEL_PRESETS.last().copied(),
        Some(CUSTOM_MODEL_OPTION)
    );

    let parsed = parse_hotkey(&defaults.copy_explanations_hotkey).expect("valid default hotkey");
    assert_eq!(parsed.vk, b'C' as u32);
    assert_eq!(
        format_hotkey(true, true, false, false, parsed.vk).as_deref(),
        Some("Ctrl+Alt+C")
    );
}

#[test]
fn qa_viewer_spans_and_explanation_clipboard_work_together() {
    let items = vec![
        QaDisplayItem {
            label: "[Q1] Answer:".to_string(),
            correct_answer: "B (List)".to_string(),
            details: String::new(),
            explanation: "Lists support item assignment.\nTuples do not.".to_string(),
            fill_status: "Filled 'B. List'".to_string(),
            ver_note: "AiReasoned".to_string(),
            confidence: 0.95,
        },
        QaDisplayItem {
            label: "[Q2] Answer:".to_string(),
            correct_answer: "print((1, 2))".to_string(),
            details: String::new(),
            explanation: "Outputs the tuple literal.".to_string(),
            fill_status: "Filled 'Code'".to_string(),
            ver_note: "PythonExecuted (stdout: (1, 2))".to_string(),
            confidence: 1.0,
        },
    ];

    let spans_q2 = build_qa_item_spans(&items[1]);
    assert!(spans_q2.contains(&RichSpan {
        text: "print((1, 2))\r\n".to_string(),
        bold: true,
        height_twips: ANSWER_FONT_TWIPS,
    }));
    assert!(spans_q2.contains(&RichSpan {
        text: "Outputs the tuple literal.".to_string(),
        bold: true,
        height_twips: BODY_FONT_TWIPS,
    }));
    assert!(spans_q2.contains(&RichSpan {
        text: "(1, 2)".to_string(),
        bold: true,
        height_twips: BODY_FONT_TWIPS,
    }));
    assert!(spans_q2.contains(&RichSpan {
        text: "PythonExecuted (stdout: ".to_string(),
        bold: false,
        height_twips: BODY_FONT_TWIPS,
    }));

    let detail_only = build_detail_spans("Solo expl", "Not filled", "PythonExecuted (syntax OK)", 80.0);
    assert_eq!(detail_only[0].bold, false);
    assert_eq!(detail_only[1].bold, true);
    assert_eq!(detail_only[3].bold, false);

    let payload = format_explanations_for_clipboard(&[
        ("Q1".to_string(), items[0].explanation.clone()),
        ("Q2".to_string(), items[1].explanation.clone()),
    ]);
    let summary = LastQaSummary {
        raw_explanation: payload.clone(),
        items,
        ..Default::default()
    };
    assert_eq!(
        summary.raw_explanation,
        "Q1: Lists support item assignment.\nTuples do not.\r\nQ2: Outputs the tuple literal."
    );
}
