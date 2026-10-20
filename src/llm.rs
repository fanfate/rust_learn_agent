use serde::Serialize;
use serde_json::Value;
use std::fmt::Debug;

use crate::{
    config::Config,
    model::{ChatResponse, Message, WireTool},
};

pub struct OpencodeGoClient {
    pub session_id: String,
    chat_url: String,
    default_model: String,
    api_key: String,
    client: reqwest::Client,
}

impl Debug for OpencodeGoClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpencodeGoClient")
            .field("session_id", &self.session_id)
            .field("chat_url", &self.chat_url)
            .field("default_model", &self.default_model)
            .field("api_key", &"***")
            .field("client", &"<reqwest::Client>")
            .finish()
    }
}

impl OpencodeGoClient {
    pub fn new(session_id: &str, config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            session_id: session_id.to_string(),
            chat_url: format!("{}/chat/completions", config.llm_base_url),
            default_model: config.llm_model.to_string(),
            api_key: config.llm_api_key.to_string(),
            client: reqwest::Client::builder()
                .user_agent("my-coding-agent/0.1")
                .default_headers({
                    let mut h = reqwest::header::HeaderMap::new();
                    h.insert("x-opencode-session", session_id.parse()?);
                    h
                })
                .build()?,
        })
    }

    pub async fn chat(
        &self,
        messages: &[Message],
        tools: &[WireTool],
    ) -> anyhow::Result<ChatResponse> {
        let chat_request = WireRequest {
            model: &self.default_model,
            messages,
            tools,
            tool_choice: "auto",
            parallel_tool_calls: false,
        };

        let resp = self
            .client
            .post(&self.chat_url)
            .bearer_auth(&self.api_key)
            .json(&chat_request)
            .send()
            .await?;

        println!("res is \n{:?}", resp);
        let status = resp.status();
        let body = resp.text().await?;
        println!("status = {status}\nbody = {body}");

        let chat_response: ChatResponse = serde_json::from_str(&body)?;
        Ok(chat_response)
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: &'a [WireTool],
    tool_choice: &'static str,
    parallel_tool_calls: bool,
}
