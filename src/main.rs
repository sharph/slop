mod command;
mod config;
mod env;
mod llm;
mod peek;
mod prompt;
mod tools;

use anyhow::{Context, Result};
use clap::Parser;
use command::{CommandRunner, StdinSource, StdoutDest};
use config::{CliConfig, Config};
use env::Environment;
use llm::{LlmClient, Message, ToolCall};
use std::io::Write;

#[derive(Parser)]
#[command(name = "slop")]
#[command(about = "A pipe-aware LLM harness. Detects pipes and gives the LLM tools to inspect and transform the data stream.")]
#[command(version)]
struct Cli {
    /// The instruction for the LLM
    #[arg(required = true)]
    prompt: String,

    #[command(flatten)]
    config: CliConfig,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = Config::load(cli.config)?;

    if config.verbose {
        eprintln!("[slop] model: {}", config.model);
        eprintln!("[slop] api_base: {}", config.api_base);
    }

    // Detect environment
    let env = Environment::detect();
    if config.verbose {
        eprintln!("[slop] stdin: {:?}", env.stdin_type);
        eprintln!("[slop] stdout: {:?}", env.stdout_type);
    }

    // Build system prompt
    let system_prompt = prompt::build_system_prompt(&env, &cli.prompt, config.system.as_deref());

    // Initialize LLM client
    let llm = LlmClient::new(
        config.api_base.clone(),
        config.api_key.clone(),
        config.model.clone(),
    );

    // Initialize command runner
    let mut runner = CommandRunner::new(
        config.capture_limit,
        config.capture_limit_max,
        config.stderr_capture_limit,
        config.stderr_capture_limit_max,
    );

    // Tool definitions
    let tool_defs = tools::tool_definitions();

    // Build initial messages
    let mut messages = vec![
        Message {
            role: "system".to_string(),
            content: Some(system_prompt),
            tool_calls: None,
            tool_call_id: None,
        },
        Message {
            role: "user".to_string(),
            content: Some(cli.prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let mut last_exit_code = 0i32;
    let mut stdout_written = false; // Track if we've written anything to stdout
    let stdout = std::io::stdout();
    let mut stdout_lock = stdout.lock();
    let stderr = std::io::stderr();
    let mut stderr_lock = stderr.lock();

    // Main loop
    for iteration in 1..=config.max_iterations {
        if config.verbose {
            eprintln!("[slop] iteration {}/{}", iteration, config.max_iterations);
        }

        let (content, tool_calls) = llm.chat(&messages, &tool_defs).await?;

        // Handle text content
        if let Some(text) = &content {
            if tool_calls.is_empty() {
                // Final response (no more tool calls)
                if stdout_written {
                    // We already wrote output to stdout via tools — this text is commentary
                    if config.verbose {
                        eprintln!("[slop] final commentary ({} chars) → stderr", text.len());
                    }
                    stderr_lock.write_all(text.as_bytes())?;
                    stderr_lock.write_all(b"\n")?;
                    stderr_lock.flush()?;
                } else {
                    // No prior stdout output — this text IS the output
                    if config.verbose {
                        eprintln!("[slop] final response ({} chars) → stdout", text.len());
                    }
                    stdout_lock.write_all(text.as_bytes())?;
                    stdout_lock.flush()?;
                }
                std::process::exit(last_exit_code);
            } else {
                // Intermediate response with tool calls — commentary to stderr
                if config.verbose {
                    eprintln!("[slop] commentary: {}", text);
                }
                stderr_lock.write_all(text.as_bytes())?;
                stderr_lock.write_all(b"\n")?;
                stderr_lock.flush()?;
            }
        }

        // Add assistant message to conversation
        messages.push(Message {
            role: "assistant".to_string(),
            content: content.clone(),
            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls.clone()) },
            tool_call_id: None,
        });

        // Execute tool calls
        if tool_calls.is_empty() {
            // No content and no tool calls — shouldn't happen
            std::process::exit(last_exit_code);
        }

        for tool_call in &tool_calls {
            if config.verbose {
                eprintln!("[slop] tool call: {} ({})", tool_call.function.name, tool_call.function.arguments);
            }

            let result = execute_tool(
                tool_call,
                &mut runner,
                &env,
                &config,
                &mut stdout_lock,
                &mut stderr_lock,
            ).await;

            let result_json = match &result {
                Ok(r) => serde_json::to_string(r).unwrap_or_else(|_| "{}".to_string()),
                Err(e) => serde_json::json!({ "error": format!("{}", e) }).to_string(),
            };

            if config.verbose {
                eprintln!("[slop] tool result: {}", result_json);
            }

            // Track exit code and stdout writes from run_command
            if tool_call.function.name == "run_command" {
                if let Ok(r) = &result {
                    if let Some(exit_code) = r.get("exit_code").and_then(|v| v.as_i64()) {
                        last_exit_code = exit_code as i32;
                    }
                    // Check if this was a passthrough (stdout was written)
                    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments).unwrap_or_default();
                    let stdout_dest = args.get("stdout_dest").and_then(|v| v.as_str()).unwrap_or("passthrough");
                    if stdout_dest == "passthrough" {
                        stdout_written = true;
                    }
                }
            }

            // Track stdout writes from write_to_stdout
            if tool_call.function.name == "write_to_stdout" && result.is_ok() {
                stdout_written = true;
            }

            messages.push(Message {
                role: "tool".to_string(),
                content: Some(result_json),
                tool_calls: None,
                tool_call_id: Some(tool_call.id.clone()),
            });
        }
    }

    // Max iterations exceeded
    eprintln!("Error: max iterations ({}) exceeded", config.max_iterations);
    std::process::exit(1);
}

async fn execute_tool(
    tool_call: &ToolCall,
    runner: &mut CommandRunner,
    env: &Environment,
    config: &Config,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<serde_json::Value> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .context("Failed to parse tool arguments")?;

    match tool_call.function.name.as_str() {
        "peek_input" => {
            if !env.stdin_type.is_readable() {
                return Ok(serde_json::json!({
                    "text": "(stdin is not a data pipe — no piped input to peek)",
                    "bytes_read": 0,
                    "truncated": false
                }));
            }

            let bytes = args.get("bytes").and_then(|v| v.as_u64()).map(|v| v as usize);
            let lines = args.get("lines").and_then(|v| v.as_u64()).map(|v| v as usize);

            let max_bytes = bytes.unwrap_or(config.peek_bytes);
            let result = runner.peek(max_bytes, lines).await?;
            Ok(serde_json::to_value(result)?)
        }

        "run_command" => {
            let command = args.get("command")
                .and_then(|v| v.as_str())
                .context("Missing 'command' parameter")?;

            let stdin_source: StdinSource = args.get("stdin_source")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let stdin_text = args.get("stdin_text").and_then(|v| v.as_str());

            let stdout_dest: StdoutDest = args.get("stdout_dest")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            let capture_limit_override = args.get("capture_limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let stderr_capture_limit_override = args.get("stderr_capture_limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let result = runner.run(
                command,
                stdin_source,
                stdin_text,
                stdout_dest,
                capture_limit_override,
                stderr_capture_limit_override,
            ).await?;
            Ok(serde_json::to_value(result)?)
        }

        "write_to_stdout" => {
            let text = args.get("text")
                .and_then(|v| v.as_str())
                .context("Missing 'text' parameter")?;
            stdout.write_all(text.as_bytes())?;
            stdout.flush()?;
            Ok(serde_json::json!({ "bytes_written": text.len() }))
        }

        "write_to_stderr" => {
            let text = args.get("text")
                .and_then(|v| v.as_str())
                .context("Missing 'text' parameter")?;
            stderr.write_all(text.as_bytes())?;
            stderr.flush()?;
            Ok(serde_json::json!({ "bytes_written": text.len() }))
        }

        _ => {
            anyhow::bail!("Unknown tool: {}", tool_call.function.name);
        }
    }
}
