use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiParseError {
    #[error("AI response did not contain valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("AI response contained zero questions")]
    EmptyQuestions,
    #[error("AI response confidence {0} is outside [0.0, 1.0]")]
    InvalidConfidence(f32),
    #[error("AI response question is missing both answer and answer_text")]
    EmptyAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionCategory {
    Mcq,
    TrueFalse,
    FillInBlank,
    PredictOutput,
    FindError,
    Debugging,
    WriteCode,
    CompleteCode,
    ExplainConcept,
    ShortAnswer,
    AlgorithmsDs,
    Mixed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FillActionKind {
    ClickOption,
    FillText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    AiReasoned,
    PythonExecuted,
    StaticChecked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAnswerResponse {
    pub question_number: Option<String>,
    pub question_type: QuestionCategory,
    pub question_text: String,
    pub answer: String,
    pub answer_text: Option<String>,
    pub target_hint: Option<String>,
    pub action: Option<FillActionKind>,
    /// Normalized [y, x] point (0..1000) of the exact UI control (radio o/0 circle or text box) inside the captured region.
    pub target_point: Option<[i32; 2]>,
    pub explanation: String,
    pub confidence: f32,
    pub verification: VerificationKind,
    pub code: Option<String>,
    /// Optional normalized bounding box [y_min, x_min, y_max, x_max] in 0..1000 inside the captured region.
    pub relative_box: Option<[i32; 4]>,
    #[serde(default)]
    pub needs_followup: bool,
    pub followup_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiQuestionAiResponse {
    pub questions: Vec<AiAnswerResponse>,
    #[serde(default)]
    pub needs_followup: bool,
    pub followup_prompt: Option<String>,
}

impl AiAnswerResponse {
    pub fn effective_action(&self) -> FillActionKind {
        if let Some(a) = self.action {
            return a;
        }
        match self.question_type {
            QuestionCategory::Mcq | QuestionCategory::TrueFalse => FillActionKind::ClickOption,
            _ => FillActionKind::FillText,
        }
    }

    /// Converts normalized `target_point` (`[y, x]` in `0..1000`) to physical screen `(x, y)` coordinates
    /// clamped inside the captured screen region.
    pub fn target_screen_point(
        &self,
        region_left: i32,
        region_top: i32,
        region_width: u32,
        region_height: u32,
    ) -> Option<(i32, i32)> {
        let [ny, nx] = self.target_point?;
        let cy = (ny as f32).clamp(2.0, 998.0) / 1000.0;
        let cx = (nx as f32).clamp(2.0, 998.0) / 1000.0;
        let sx = region_left + (cx * region_width as f32).round() as i32;
        let sy = region_top + (cy * region_height as f32).round() as i32;
        Some((sx, sy))
    }

    /// Maps normalized `relative_box` (`0..1000`) to physical screen coordinates inside the captured rect.
    /// Falls back to `target_point` or the center of the captured rect.
    pub fn anchor_screen_coords(
        &self,
        region_left: i32,
        region_top: i32,
        region_width: u32,
        region_height: u32,
    ) -> (i32, i32) {
        if let Some(pt) = self.target_screen_point(region_left, region_top, region_width, region_height) {
            return pt;
        }
        if let Some([y_min, x_min, y_max, x_max]) = self.relative_box {
            let cy = ((y_min + y_max) as f32 / 2.0).clamp(0.0, 1000.0) / 1000.0;
            let cx = ((x_min + x_max) as f32 / 2.0).clamp(0.0, 1000.0) / 1000.0;
            let sx = region_left + (cx * region_width as f32).round() as i32;
            let sy = region_top + (cy * region_height as f32).round() as i32;
            (sx, sy)
        } else {
            (
                region_left + (region_width as i32) / 2,
                region_top + (region_height as i32) / 2,
            )
        }
    }
}

/// Extracts and validates structured JSON (supporting both `{ "questions": [...] }`
/// and single-question objects wrapped automatically into a 1-element list).
pub fn parse_multi_ai_response(raw: &str) -> Result<MultiQuestionAiResponse, AiParseError> {
    let json_slice = strip_markdown_fences(raw.trim());

    let multi: MultiQuestionAiResponse =
        if let Ok(m) = serde_json::from_str::<MultiQuestionAiResponse>(json_slice) {
            m
        } else {
            let single: AiAnswerResponse = serde_json::from_str(json_slice)?;
            MultiQuestionAiResponse {
                needs_followup: single.needs_followup,
                followup_prompt: single.followup_prompt.clone(),
                questions: vec![single],
            }
        };

    if multi.questions.is_empty() {
        return Err(AiParseError::EmptyQuestions);
    }

    for q in &multi.questions {
        validate_single(q)?;
    }

    Ok(multi)
}

/// Convenience parser for single-question callers/tests.
pub fn parse_ai_response(raw: &str) -> Result<AiAnswerResponse, AiParseError> {
    let mut multi = parse_multi_ai_response(raw)?;
    Ok(multi.questions.remove(0))
}

fn validate_single(q: &AiAnswerResponse) -> Result<(), AiParseError> {
    if !(0.0..=1.0).contains(&q.confidence) {
        return Err(AiParseError::InvalidConfidence(q.confidence));
    }
    if q.answer.trim().is_empty()
        && q.answer_text
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(AiParseError::EmptyAnswer);
    }
    Ok(())
}

fn strip_markdown_fences(input: &str) -> &str {
    if let Some(start) = input.find('{') {
        if let Some(end) = input.rfind('}') {
            if end >= start {
                return &input[start..=end];
            }
        }
    }
    input
}
