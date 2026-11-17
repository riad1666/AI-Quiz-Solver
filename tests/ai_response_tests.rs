use knu_diagnosis::ai::prompts::{build_followup_prompt, vision_user_prompt};
use knu_diagnosis::ai::response::{
    parse_ai_response, parse_multi_ai_response, FillActionKind, QuestionCategory, VerificationKind,
};

#[test]
fn parses_multi_question_mixed_types_and_maps_target_point() {
    let raw = r#"```json
{
  "questions": [
    {
      "question_number": "1",
      "question_type": "mcq",
      "question_text": "Which Python collection is mutable?",
      "answer": "B",
      "answer_text": "List",
      "target_hint": "Left radio o circle for B: List",
      "action": "click_option",
      "target_point": [240, 120],
      "explanation": "Lists can be modified after creation.",
      "confidence": 0.96,
      "verification": "ai_reasoned",
      "code": null,
      "relative_box": [100, 100, 300, 500]
    },
    {
      "question_number": "2 (1)",
      "question_type": "fill_in_blank",
      "question_text": "Gap (1) above the code",
      "answer": "len",
      "answer_text": null,
      "target_hint": "Input box (1)",
      "action": "fill_text",
      "target_point": [550, 300],
      "explanation": "len() returns list length.",
      "confidence": 0.99,
      "verification": "ai_reasoned",
      "code": null,
      "relative_box": [500, 200, 600, 400]
    }
  ],
  "needs_followup": false,
  "followup_prompt": null
}
```"#;

    let parsed = parse_multi_ai_response(raw).expect("should parse multi-question JSON");
    assert_eq!(parsed.questions.len(), 2);
    assert_eq!(parsed.questions[0].question_type, QuestionCategory::Mcq);
    assert_eq!(parsed.questions[0].effective_action(), FillActionKind::ClickOption);
    assert_eq!(parsed.questions[1].question_type, QuestionCategory::FillInBlank);
    assert_eq!(parsed.questions[1].effective_action(), FillActionKind::FillText);

    assert_eq!(
        parsed.questions[0].target_screen_point(100, 200, 1000, 500),
        Some((220, 320))
    );
    assert_eq!(
        parsed.questions[1].target_screen_point(100, 200, 1000, 500),
        Some((400, 475))
    );
}

#[test]
fn wraps_single_question_object_and_rejects_invalid_fields() {
    let single = r#"{
      "question_number": "5",
      "question_type": "mcq",
      "question_text": "Which is immutable?",
      "answer": "A",
      "answer_text": "Tuple",
      "target_hint": "Option A",
      "explanation": "Tuples cannot be mutated.",
      "confidence": 0.95,
      "verification": "ai_reasoned",
      "code": null
    }"#;
    let parsed = parse_ai_response(single).expect("single object fallback");
    assert_eq!(parsed.answer, "A");
    assert_eq!(parsed.verification, VerificationKind::AiReasoned);

    let bad_conf = single.replace("0.95", "1.5");
    assert!(parse_multi_ai_response(&bad_conf).is_err());

    let empty_ans = single.replace("\"A\"", "\"   \"").replace("\"Tuple\"", "null");
    assert!(parse_multi_ai_response(&empty_ans).is_err());
}

#[test]
fn builds_vision_and_followup_prompts() {
    let prompt = vision_user_prompt(800, 600);
    assert!(prompt.contains("800x600"));

    let raw = r#"{
      "questions": [
        {
          "question_number": "12",
          "question_type": "debugging",
          "question_text": "Fix the off-by-one bug",
          "answer": "range(len(nums) - 1)",
          "answer_text": null,
          "target_hint": "Code editor",
          "explanation": "Prevents IndexError",
          "confidence": 0.7,
          "verification": "ai_reasoned",
          "code": "for i in range(len(nums) - 1): pass"
        }
      ],
      "needs_followup": true,
      "followup_prompt": "Check empty list edge case"
    }"#;
    let parsed = parse_multi_ai_response(raw).expect("parse");
    let followup = build_followup_prompt(&parsed);
    assert!(followup.contains("Check empty list edge case"));
    assert!(followup.contains("Fix the off-by-one bug"));
}
