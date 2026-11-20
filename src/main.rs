use my_agent::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: Config = Config::from_env()?;
    println!("LLM config : {:?}", config);
    // let mut client = OpencodeGoClient::new("v1-session", &config)?;
    //
    // // let logger_config = LogSink::File("data/run.log");
    // let logger_config = LogSink::File(&my_agent::time_log());
    // my_agent::check_new(&mut client, &logger_config, &config.sandbox_root).await?;

    Ok(())
}
