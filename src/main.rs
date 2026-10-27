use my_agent::{agent::run_agent, config::Config, llm::OpencodeGoClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Config = Config::from_env()?;
    println!("LLM config : {:?}", config);
    let client = OpencodeGoClient::new("v1-session", &config)?;

    run_agent(&client, "看下沙箱根目录'.'有什么文件", 5).await?;

    // check_error(&client).await?;

    Ok(())
}

async fn check_error(client: &OpencodeGoClient) -> anyhow::Result<()> {
    run_agent(
        client,
        "查看并尝试修复沙箱下的my_api文件夹内的rust错误,临时文件请放在沙箱下的tmp文件夹内。可以使用“cargo check --message-format=short”命令减少输出。",
        50,
    )
    .await
}
