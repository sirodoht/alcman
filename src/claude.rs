use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{env, error::Error, fmt};

const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const USER_AGENT: &str = "alcman/0.1.0";

#[derive(Clone, Debug, Default)]
pub struct ClaudeConfig {
    api_key: Option<String>,
}

impl ClaudeConfig {
    pub fn from_env() -> Self {
        let api_key = env::var("CLAUDE_API_KEY").ok();
        Self { api_key }
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
}

#[derive(Clone)]
pub struct ClaudeClient {
    http: Client,
    config: ClaudeConfig,
}

impl ClaudeClient {
    pub fn new(config: ClaudeConfig) -> Self {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("failed to build reqwest client");

        Self { http, config }
    }

    pub fn has_api_key(&self) -> bool {
        self.config.api_key().is_some()
    }

    pub async fn extract_book_metadata(
        &self,
        query: &str,
        model: &str,
    ) -> Result<BookMetadata, ClaudeError> {
        let prompt = format!(
            "Identify this book: \"{query}\"\n\n\
            Return the information as JSON with these fields:\n\
            - title: the correct title (omit the subtitle if it exists)\n\
            - author: the author name (if multiple authors, separate with commas)\n\
            - publication_year: the original publication year if known, otherwise null\n\n\
            Return ONLY valid JSON, no other text."
        );

        let request = MessagesRequest {
            model: model.to_string(),
            max_tokens: 1024,
            system: Some(
                "You are a knowledgeable librarian assistant. \
                Always respond with valid JSON only, no markdown or extra text."
                    .to_string(),
            ),
            messages: vec![Message::user(prompt)],
        };

        let response = self.send_message(request).await?;
        let content = response
            .content
            .into_iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        if content.trim().is_empty() {
            return Err(ClaudeError::UnexpectedResponse(
                "Empty response from Claude".into(),
            ));
        }

        // Parse JSON response, stripping any markdown code fences if present
        let json_str = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        serde_json::from_str(json_str).map_err(|e| {
            ClaudeError::UnexpectedResponse(format!(
                "Failed to parse book metadata: {e}\nRaw: {content}"
            ))
        })
    }

    pub async fn edit_book_with_instruction(
        &self,
        current_title: &str,
        current_author: Option<&str>,
        current_publication_year: Option<i32>,
        instruction: &str,
        model: &str,
    ) -> Result<BookEditResult, ClaudeError> {
        let author_str = current_author.unwrap_or("unknown");
        let year_str = current_publication_year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let prompt = format!(
            "I have a book with these current details:\n\
            - Title: {current_title}\n\
            - Author: {author_str}\n\
            - Publication Year: {year_str}\n\n\
            User instruction: \"{instruction}\"\n\n\
            Apply the user's instruction to update the book details. \
            Return the updated information as JSON with these fields:\n\
            - title: the updated title (or keep original if not changing)\n\
            - author: the author name (if multiple authors, separate with commas; or null if unknown)\n\
            - publication_year: the updated publication year as a number (or null if unknown)\n\n\
            Return ONLY valid JSON, no other text."
        );

        let request = MessagesRequest {
            model: model.to_string(),
            max_tokens: 1024,
            system: Some(
                "You are a knowledgeable librarian assistant helping to update book records. \
                Follow the user's instructions precisely. For example, if they ask for a German title, \
                provide the German translation of the title. If they ask to fix spelling, correct it. \
                Always respond with valid JSON only, no markdown or extra text."
                    .to_string(),
            ),
            messages: vec![Message::user(prompt)],
        };

        let response = self.send_message(request).await?;
        let content = response
            .content
            .into_iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        if content.trim().is_empty() {
            return Err(ClaudeError::UnexpectedResponse(
                "Empty response from Claude".into(),
            ));
        }

        // Parse JSON response, stripping any markdown code fences if present
        let json_str = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        serde_json::from_str(json_str).map_err(|e| {
            ClaudeError::UnexpectedResponse(format!(
                "Failed to parse book edit result: {e}\nRaw: {content}"
            ))
        })
    }

    pub async fn send_message(
        &self,
        request: MessagesRequest,
    ) -> Result<MessagesResponse, ClaudeError> {
        let api_key = self
            .config
            .api_key()
            .ok_or(ClaudeError::MissingApiKey)?
            .to_string();

        // Log the request
        println!(
            "[HTTP] POST {} (model: {})",
            ANTHROPIC_MESSAGES_URL, request.model
        );

        let response = self
            .http
            .post(ANTHROPIC_MESSAGES_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&request)
            .send()
            .await
            .map_err(ClaudeError::Http)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ClaudeError::UnexpectedResponse(format!(
                "Anthropic request failed ({status}): {body}"
            )));
        }

        let payload = response.bytes().await.map_err(ClaudeError::Http)?;

        serde_json::from_slice(&payload).map_err(ClaudeError::Json)
    }
}

#[derive(Debug)]
pub enum ClaudeError {
    MissingApiKey,
    Http(reqwest::Error),
    Json(serde_json::Error),
    UnexpectedResponse(String),
}

#[derive(Debug, Deserialize)]
pub struct BookMetadata {
    pub title: String,
    pub author: Option<String>,
    pub publication_year: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct BookEditResult {
    pub title: String,
    pub author: Option<String>,
    pub publication_year: Option<i32>,
}

impl fmt::Display for ClaudeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClaudeError::MissingApiKey => write!(f, "CLAUDE_API_KEY is not set"),
            ClaudeError::Http(err) => write!(f, "HTTP error: {err}"),
            ClaudeError::Json(err) => write!(f, "Failed to parse response JSON: {err}"),
            ClaudeError::UnexpectedResponse(msg) => write!(f, "{msg}"),
        }
    }
}

impl Error for ClaudeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ClaudeError::Http(err) => Some(err),
            ClaudeError::Json(err) => Some(err),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user<T: Into<String>>(content: T) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant<T: Into<String>>(content: T) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
}
