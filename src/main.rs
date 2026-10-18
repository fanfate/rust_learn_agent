use my_agent::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Config = Config::from_env()?;
    println!("LLM config : {:?}", config);

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
        "你好。",
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
    let json_value = serde_json::json!({
        "model" : llm_model,
        "messages" : [
        {
            "role" : "system",
            "content" : "You are a helpful assistant."
        },
        {
            "role" : "user",
            "content" : question
        }
    ]
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

    Ok(())
}
