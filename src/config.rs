use dotenvy::dotenv;

pub const LLM_ENV_KEYS: &[&str] = &["LLM_BASE_URL", "LLM_MODEL", "LLM_API_KEY"];

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("环境变量 {name} 未设置")]
    Missing { name: &'static str },

    #[error("环境变量 {name} 值不是UTF-8字符")]
    NotUnicode { name: &'static str },

    #[error("环境变量 {name} 值为空")]
    Empty { name: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueError {
    Missing,
    NotUnicode,
}

fn get_env(name: &str) -> Result<String, ValueError> {
    let value = std::env::var(name).map_err(|err| match err {
        std::env::VarError::NotPresent => ValueError::Missing,
        std::env::VarError::NotUnicode(_) => ValueError::NotUnicode,
    })?;
    Ok(value)
}

fn format_url(url: String) -> String {
    url.trim_end_matches('/').to_string()
}

fn get_map(
    name: &'static str,
    get: &impl Fn(&str) -> Result<String, ValueError>,
) -> Result<String, ConfigError> {
    let value = get(name).map_err(|err| match err {
        ValueError::Missing => ConfigError::Missing { name },
        ValueError::NotUnicode => ConfigError::NotUnicode { name },
    })?;
    if value.is_empty() {
        return Err(ConfigError::Empty { name });
    }

    Ok(value)
}

#[derive(Clone)]
pub struct Config {
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_api_key: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("llm_base_url", &self.llm_base_url)
            .field("llm_model", &self.llm_model)
            .field("llm_api_key", &"***")
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Config, ConfigError> {
        dotenv().ok();
        Self::from_map(&get_env)
    }

    fn from_map(get: &impl Fn(&str) -> Result<String, ValueError>) -> Result<Config, ConfigError> {
        let llm_base_url = format_url(get_map("LLM_BASE_URL", get)?);
        let llm_model = get_map("LLM_MODEL", get)?;
        let llm_api_key = get_map("LLM_API_KEY", get)?;
        Ok(Self {
            llm_base_url,
            llm_model,
            llm_api_key,
        })
    }
}
