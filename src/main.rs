use my_agent::{agent::run_agent, config::Config, llm::OpencodeGoClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Config = Config::from_env()?;
    println!("LLM config : {:?}", config);
    let client = OpencodeGoClient::new("v1-session", &config)?;

    run_agent(&client, "看下沙箱根目录'.'有什么文件", 5).await?;

    Ok(())
}
