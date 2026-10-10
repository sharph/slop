use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub model: String,
    pub api_base: String,
    pub api_key: Option<String>,
    pub system: Option<String>,
    pub max_iterations: u32,
    pub peek_bytes: usize,
    pub capture_limit: usize,
    pub capture_limit_max: usize,
    pub stderr_capture_limit: usize,
    pub stderr_capture_limit_max: usize,
    pub verbose: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    model: Option<String>,
    api_base: Option<String>,
    api_key: Option<String>,
    system: Option<String>,
    max_iterations: Option<u32>,
    peek_bytes: Option<usize>,
    capture_limit: Option<usize>,
    capture_limit_max: Option<usize>,
    stderr_capture_limit: Option<usize>,
    stderr_capture_limit_max: Option<usize>,
    verbose: Option<bool>,
}

#[derive(Debug, clap::Args)]
pub struct CliConfig {
    /// Model name (e.g., gpt-4o, llama3.1)
    #[arg(long, env = "SLOP_MODEL")]
    pub model: Option<String>,

    /// OpenAI-compatible API base URL
    #[arg(long, env = "SLOP_API_BASE")]
    pub api_base: Option<String>,

    /// API key
    #[arg(long, env = "SLOP_API_KEY")]
    pub api_key: Option<String>,

    /// Additional system prompt text
    #[arg(long, env = "SLOP_SYSTEM")]
    pub system: Option<String>,

    /// Max tool-calling rounds
    #[arg(long, env = "SLOP_MAX_ITERATIONS")]
    pub max_iterations: Option<u32>,

    /// Default peek size in bytes
    #[arg(long, env = "SLOP_PEEK_BYTES")]
    pub peek_bytes: Option<usize>,

    /// Default max bytes captured from command stdout
    #[arg(long, env = "SLOP_CAPTURE_LIMIT")]
    pub capture_limit: Option<usize>,

    /// Max bytes the LLM can request for stdout capture (upper bound)
    #[arg(long, env = "SLOP_CAPTURE_LIMIT_MAX")]
    pub capture_limit_max: Option<usize>,

    /// Default max bytes captured from command stderr
    #[arg(long, env = "SLOP_STDERR_CAPTURE_LIMIT")]
    pub stderr_capture_limit: Option<usize>,

    /// Max bytes the LLM can request for stderr capture (upper bound)
    #[arg(long, env = "SLOP_STDERR_CAPTURE_LIMIT_MAX")]
    pub stderr_capture_limit_max: Option<usize>,

    /// Print LLM reasoning to stderr
    #[arg(long, short, env = "SLOP_VERBOSE")]
    pub verbose: bool,
}

impl Config {
    pub fn load(cli: CliConfig) -> Result<Self> {
        let file = Self::load_file()?;

        let model = cli
            .model
            .or(file.model)
            .context("No model specified. Use --model, SLOP_MODEL, or set 'model' in ~/.config/slop/config.toml")?;

        let api_base = cli
            .api_base
            .or(file.api_base)
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        let api_key = cli.api_key.or(file.api_key);

        Ok(Config {
            model,
            api_base,
            api_key,
            system: cli.system.or(file.system),
            // Precedence: CLI flag > env var (via clap) > config file > default
            max_iterations: cli.max_iterations.or(file.max_iterations).unwrap_or(20),
            peek_bytes: cli.peek_bytes.or(file.peek_bytes).unwrap_or(4096),
            capture_limit: cli.capture_limit.or(file.capture_limit).unwrap_or(4096),
            capture_limit_max: cli.capture_limit_max.or(file.capture_limit_max).unwrap_or(65536),
            stderr_capture_limit: cli.stderr_capture_limit.or(file.stderr_capture_limit).unwrap_or(4096),
            stderr_capture_limit_max: cli.stderr_capture_limit_max.or(file.stderr_capture_limit_max).unwrap_or(65536),
            verbose: cli.verbose || file.verbose.unwrap_or(false),
        })
    }

    fn load_file() -> Result<ConfigFile> {
        let path = Self::config_path();
        match path {
            Some(p) if p.exists() => {
                let content = std::fs::read_to_string(&p)
                    .with_context(|| format!("Failed to read config file: {}", p.display()))?;
                toml::from_str(&content)
                    .with_context(|| format!("Failed to parse config file: {}", p.display()))
            }
            _ => Ok(ConfigFile::default()),
        }
    }

    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("slop").join("config.toml"))
    }
}
