use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

/// Content can be a plain string (OpenAI) or an array of parts (Mistral, etc.)
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ContentField {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    part_type: String,
    #[serde(default)]
    text: Option<String>,
    // Note: "thinking" parts (Mistral) are ignored — serde skips unknown fields
}

impl ContentField {
    /// Extract text content, ignoring non-text parts (like "thinking").
    fn into_text(self) -> Option<String> {
        match self {
            ContentField::Text(s) => Some(s),
            ContentField::Parts(parts) => {
                let texts: Vec<String> = parts
                    .into_iter()
                    .filter(|p| p.part_type == "text")
                    .filter_map(|p| p.text)
                    .collect();
                if texts.is_empty() {
                    None
                } else {
                    Some(texts.join(""))
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default, deserialize_with = "deserialize_content")]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

fn deserialize_content<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let opt = Option::<ContentField>::deserialize(deserializer)?;
    Ok(opt.and_then(|c| c.into_text()))
}

pub struct LlmClient {
    client: reqwest::Client,
    api_base: String,
    api_key: Option<String>,
    model: String,
}

impl LlmClient {
    pub fn new(api_base: String, api_key: Option<String>, model: String) -> Self {
        LlmClient {
            client: reqwest::Client::new(),
            api_base: api_base.trim_end_matches('/').to_string(),
            api_key,
            model,
        }
    }

    pub async fn chat(
        &self,
        messages: &[Message],
        tools: &[Value],
    ) -> Result<(Option<String>, Vec<ToolCall>)> {
        let body = json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
        });

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.api_base))
            .json(&body);

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.context("Failed to send request to LLM API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM API error ({}): {}", status, body);
        }

        let chat_resp: ChatResponse = resp.json().await.context("Failed to parse LLM response")?;

        let choice = chat_resp
            .choices
            .into_iter()
            .next()
            .context("LLM returned no choices")?;

        let content = choice.message.content.filter(|c| !c.is_empty());
        let tool_calls = choice.message.tool_calls.unwrap_or_default();

        Ok((content, tool_calls))
    }
}
