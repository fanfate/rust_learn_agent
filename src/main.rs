use my_agent::{
    config::Config,
    llm::OpencodeGoClient,
    model::{Message, ToolKind},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Config = Config::from_env()?;
    println!("LLM config : {:?}", config);

    let client = OpencodeGoClient::new("v1-session", &config)?;

    test_loop_chat(&client, "请阅读下scratch/readme.md文件").await?;

    Ok(())
}

async fn test_loop_chat(client: &OpencodeGoClient, question: &str) -> anyhow::Result<()> {
    let mut messages = vec![];
    messages.push(Message::System {
        content: "你是一个有用的助手。".to_string(),
    });
    messages.push(Message::User {
        content: question.to_string(),
    });

    let tools = ToolKind::tools();

    let resp = client.chat(&messages, &tools).await?;
    print!("response is {:?}", resp);

    Ok(())
}

async fn test_chat(config: &Config) -> anyhow::Result<()> {
    let chat_url = format!("{}/chat/completions", config.llm_base_url);

    // for opencode go
    let opencode_client = reqwest::Client::builder()
        .user_agent("my-coding-agent/0.1")
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("x-opencode-session", "v0-session".parse()?);
            h
        })
        .build()?;

    get_chat(
        &opencode_client,
        &chat_url,
        &config.llm_model,
        &config.llm_api_key,
        "请阅读下scratch/readme.md文件",
    )
    .await?;

    Ok(())
}

async fn get_chat(
    client: &reqwest::Client,
    chat_url: &str,
    llm_model: &str,
    llm_api_key: &str,
    question: &str,
) -> anyhow::Result<()> {
    let mut messages = vec![];
    let system_prompt = serde_json::json!({
        "role" : "system",
        "content" : "You are a helpful assistant."
    });

    let user_message = serde_json::json!({
        "role" : "user",
        "content" : question
    });

    messages.push(system_prompt);
    messages.push(user_message);

    let functions_value = serde_json::json!({
        "type" : "function",
        "function" : {
            "name" : "read_file",
            "description" : "读取一个文件的内容",
            "parameters": {                 // ← JSON Schema 对象，不是字符串
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false // strict 模式下必需
            },
            "strict": true
        }
    });

    let tools = vec![functions_value];

    let json_value = serde_json::json!({
        "model" : llm_model,
        "messages" : &messages,
        "tools": &tools,
        "tool_choice" : "auto",
        "parallel_tool_calls" : true
    });

    let resp = client
        .post(chat_url)
        .bearer_auth(llm_api_key)
        .json(&json_value)
        .send()
        .await?;

    println!("res is \n{:?}", resp);

    let status = resp.status();
    let body = resp.text().await?;
    println!("status = {status}\nbody = {body}");

    let v: serde_json::Value = serde_json::from_str(&body)?;
    let assistant_msg = v["choices"][0]["message"].clone();

    println!("assistant_msg is\n{:?}", assistant_msg);

    let tool_call_id = assistant_msg["tool_calls"][0]["id"].clone();
    let tool_message = serde_json::json!({
        "role" : "tool",
        "tool_call_id" : &tool_call_id,
        "content" : "## 你好\n\n你好\n"
    });

    messages.push(assistant_msg);
    messages.push(tool_message);

    let json_tool_value = serde_json::json!({
        "model" : llm_model,
        "messages" : &messages,
        "tools": &tools,
        "tool_choice" : "auto",
        "parallel_tool_calls" : true
    });

    let tool_resp = client
        .post(chat_url)
        .bearer_auth(llm_api_key)
        .json(&json_tool_value)
        .send()
        .await?;

    println!("tool_res is \n{:?}", tool_resp);

    let status = tool_resp.status();
    let body = tool_resp.text().await?;
    println!("tool_call:\nstatus = {status}\nbody = {body}");

    Ok(())
}
