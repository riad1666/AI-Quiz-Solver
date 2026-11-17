use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

use crate::ai::prompts::{build_followup_prompt, vision_system_prompt, vision_user_prompt};
use crate::ai::response::{parse_multi_ai_response, AiParseError, MultiQuestionAiResponse};
use crate::capture::screen::ScreenCapture;
use crate::config::settings::AppSettings;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Debug, Error)]
pub enum OpenRouterError {
    #[error("OpenRouter API key is not configured")]
    MissingApiKey,
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("OpenRouter API returned status {status}: {body}")]
    ApiStatus { status: u16, body: String },
    #[error("OpenRouter response had no choices")]
    EmptyChoices,
    #[error("failed to parse structured AI response: {0}")]
    Parse(#[from] AiParseError),
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: MessageContent<'a>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct SolveOutcome {
    pub response: MultiQuestionAiResponse,
    pub model_used: String,
    pub usage: TokenUsage,
}

pub struct OpenRouterClient {
    http: Client,
    api_key: String,
}

impl OpenRouterClient {
    pub fn new(api_key: String, timeout_secs: u64) -> Result<Self, OpenRouterError> {
        if api_key.trim().is_empty() {
            return Err(OpenRouterError::MissingApiKey);
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(5)))
            .build()?;
        Ok(Self { http, api_key })
    }

    /// Sends the captured region directly to OpenRouter Vision AI and runs an automatic
    /// follow-up reasoning pass if `needs_followup` is requested by the model.
    pub async fn solve_from_capture(
        &self,
        capture: &ScreenCapture,
        settings: &AppSettings,
    ) -> Result<SolveOutcome, OpenRouterError> {
        let encoded = BASE64.encode(&capture.image_bytes);
        let data_url = format!("data:{};base64,{}", capture.mime_type, encoded);

        let user_prompt = vision_user_prompt(capture.width, capture.height);

        let messages = vec![
            ChatMessage {
                role: "system",
                content: MessageContent::Text(vision_system_prompt()),
            },
            ChatMessage {
                role: "user",
                content: MessageContent::Parts(vec![
                    ContentPart::Text { text: user_prompt },
                    ContentPart::ImageUrl {
                        image_url: ImageUrl { url: data_url },
                    },
                ]),
            },
        ];

        let (raw_text, mut total_usage) = self
            .send_with_retries(&settings.vision_model, messages, settings)
            .await?;

        let mut parsed = parse_multi_ai_response(&raw_text)?;
        let mut model_used = settings.vision_model.clone();

        if parsed.needs_followup {
            info!("AI requested automatic follow-up pass using reasoning model");
            let followup = build_followup_prompt(&parsed);
            let followup_messages = vec![
                ChatMessage {
                    role: "system",
                    content: MessageContent::Text(vision_system_prompt()),
                },
                ChatMessage {
                    role: "user",
                    content: MessageContent::Text(&followup),
                },
            ];
            let (followup_raw, followup_usage) = self
                .send_with_retries(&settings.reasoning_model, followup_messages, settings)
                .await?;
            parsed = parse_multi_ai_response(&followup_raw)?;
            model_used = settings.reasoning_model.clone();
            total_usage.prompt_tokens += followup_usage.prompt_tokens;
            total_usage.completion_tokens += followup_usage.completion_tokens;
        }

        Ok(SolveOutcome {
            response: parsed,
            model_used,
            usage: total_usage,
        })
    }

    async fn send_with_retries(
        &self,
        model: &str,
        messages: Vec<ChatMessage<'_>>,
        settings: &AppSettings,
    ) -> Result<(String, TokenUsage), OpenRouterError> {
        let payload = ChatRequest {
            model,
            messages,
            max_tokens: settings.max_tokens,
            temperature: settings.temperature,
        };

        let max_attempts = settings.retry_count.saturating_add(1).max(1);
        let mut last_err: Option<OpenRouterError> = None;

        for attempt in 1..=max_attempts {
            let res = self
                .http
                .post(OPENROUTER_URL)
                .bearer_auth(&self.api_key)
                .header("HTTP-Referer", "https://www.kangwon.ac.kr")
                .header("X-Title", "KNU Diagnosis")
                .json(&payload)
                .send()
                .await;

            match res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let body: ChatResponse = resp.json().await?;
                        let content = body
                            .choices
                            .into_iter()
                            .next()
                            .and_then(|c| c.message.content)
                            .ok_or(OpenRouterError::EmptyChoices)?;
                        return Ok((content, body.usage.unwrap_or_default()));
                    }

                    let body_text = resp.text().await.unwrap_or_default();
                    let err = OpenRouterError::ApiStatus {
                        status: status.as_u16(),
                        body: body_text,
                    };
                    if status.as_u16() == 429 || status.is_server_error() {
                        warn!(attempt, status = status.as_u16(), "Transient OpenRouter error; retrying");
                        last_err = Some(err);
                    } else {
                        return Err(err);
                    }
                }
                Err(err) => {
                    warn!(attempt, error = %err, "Network error calling OpenRouter; retrying");
                    last_err = Some(OpenRouterError::Http(err));
                }
            }

            if attempt < max_attempts {
                tokio::time::sleep(Duration::from_millis(400 * attempt as u64)).await;
            }
        }

        Err(last_err.unwrap_or(OpenRouterError::EmptyChoices))
    }
}
