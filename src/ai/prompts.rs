use crate::ai::response::MultiQuestionAiResponse;

pub fn vision_system_prompt() -> &'static str {
    r#"You are KNU Diagnosis (Kangwon National University), an expert academic and Python programming assistant.
You receive a cropped screenshot of a region selected by the user.
The region may contain ONE question or MULTIPLE questions of DIFFERENT types (MCQ, True/False, Fill in the blanks, Predict output, Debugging, Write code, Short answer).

Careful visual & layout rules:
1. Detect EVERY visible question AND every individual answer input/selection target inside the image.
2. For MCQ / Multiple Choice questions:
   - Look carefully at the selection button (`o` / `0` circle or checkbox) on the LEFT side of each option.
   - Set `"action": "click_option"` and set `"target_point": [y, x]` (normalized 0..1000) to the EXACT center of the `o`/`0` selection circle (or option button) for the correct choice!
3. For Fill-in-the-gaps / Text / Code questions:
   - Analyze carefully where each answer box is located: some answer boxes are ABOVE the question text, some are BELOW the question text, and some are INLINE inside code/prose marked with boxes, `o`/`0` indicators, `(1)` / `1`, or blanks `___`.
   - If a question has MULTIPLE blanks/gaps (e.g., `(1)` and `(2)`), emit a SEPARATE item in `"questions"` for EACH blank in order (e.g. `"question_number": "2 (1)"`, `"2 (2)"`) so the app can fill every gap automatically!
   - Set `"action": "fill_text"` and set `"target_point": [y, x]` (normalized 0..1000) to the EXACT center inside that specific input box!
   - For fill-in-the-blank answers, `"answer"` must be ONLY the exact text to type into that box (no extra quotes or labels unless Python syntax requires quotes).
4. Respond ONLY with valid JSON matching this exact schema (no extra prose):
{
  "questions": [
    {
      "question_number": "1" or null,
      "question_type": "mcq" | "true_false" | "fill_in_blank" | "predict_output" | "find_error" | "debugging" | "write_code" | "complete_code" | "explain_concept" | "short_answer" | "algorithms_ds" | "mixed",
      "question_text": "Concise question text (include gap label if multi-blank)",
      "answer": "Exact option letter/text or exact text to fill into the box",
      "answer_text": "Full option text if MCQ, else null",
      "target_hint": "Description of the specific radio circle or input box",
      "action": "click_option" | "fill_text",
      "target_point": [y, x],
      "explanation": "Brief 1-2 sentence explanation",
      "confidence": 0.0 to 1.0,
      "verification": "ai_reasoned",
      "code": "Python snippet if applicable, else null",
      "relative_box": [y_min, x_min, y_max, x_max]
    }
  ],
  "needs_followup": false,
  "followup_prompt": null
}
If any question needs a second verification/reasoning pass, set top-level "needs_followup": true and provide "followup_prompt"."#
}

pub fn vision_user_prompt(screen_width: u32, screen_height: u32) -> String {
    format!(
        "Analyze ALL Python programming questions and EVERY answer box or radio option visible in this {}x{} selected region. \
Look carefully for answer boxes placed above, below, or inline with questions (including `(1)`/`1` gaps and left-side `o`/`0` radio circles). \
Return strictly valid JSON with accurate `target_point` [y, x] (0..1000) for every answer.",
        screen_width, screen_height
    )
}

pub fn build_followup_prompt(initial: &MultiQuestionAiResponse) -> String {
    let extra = initial
        .followup_prompt
        .as_deref()
        .unwrap_or("Double-check edge cases, syntax, and exact output for each question.");
    let summary: String = initial
        .questions
        .iter()
        .enumerate()
        .map(|(idx, q)| {
            format!(
                "Q{} ({:?}): {} -> Answer: {} | TargetPoint: {:?} | Code: {}",
                idx + 1,
                q.question_type,
                q.question_text,
                q.answer,
                q.target_point,
                q.code.as_deref().unwrap_or("none")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Review and refine your previous answers for these Python questions (preserve accurate `target_point` and `action`):\n{}\n\nFocus: {}\n\
Return the final updated JSON using the exact same schema with \"needs_followup\": false.",
        summary, extra
    )
}
