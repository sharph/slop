use crate::peek::PeekBuffer;
use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StdinSource {
    #[default]
    Inherit,
    None,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StdoutDest {
    #[default]
    Passthrough,
    Capture,
    Discard,
}

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

pub struct CommandRunner {
    peek_buffer: PeekBuffer,
    capture_limit: usize,
    capture_limit_max: usize,
    stderr_capture_limit: usize,
    stderr_capture_limit_max: usize,
    /// Async stdin handle for peeking and forwarding.
    stdin: Arc<Mutex<tokio::io::Stdin>>,
}

impl CommandRunner {
    pub fn new(
        capture_limit: usize,
        capture_limit_max: usize,
        stderr_capture_limit: usize,
        stderr_capture_limit_max: usize,
    ) -> Self {
        CommandRunner {
            peek_buffer: PeekBuffer::new(),
            capture_limit,
            capture_limit_max,
            stderr_capture_limit,
            stderr_capture_limit_max,
            stdin: Arc::new(Mutex::new(tokio::io::stdin())),
        }
    }

    pub async fn peek(&mut self, max_bytes: usize, max_lines: Option<usize>) -> std::io::Result<crate::peek::PeekResult> {
        let mut stdin = self.stdin.lock().await;
        if let Some(max_lines) = max_lines {
            peek_lines(&mut stdin, &mut self.peek_buffer, max_lines).await
        } else {
            peek_bytes(&mut stdin, &mut self.peek_buffer, max_bytes).await
        }
    }

    pub async fn run(
        &mut self,
        command: &str,
        stdin_source: StdinSource,
        stdin_text: Option<&str>,
        stdout_dest: StdoutDest,
        capture_limit_override: Option<usize>,
        stderr_capture_limit_override: Option<usize>,
    ) -> Result<CommandResult> {
        // Resolve capture limits: LLM override clamped to max, or default
        let stdout_limit = capture_limit_override
            .map(|l| l.min(self.capture_limit_max))
            .unwrap_or(self.capture_limit);
        let stderr_limit = stderr_capture_limit_override
            .map(|l| l.min(self.stderr_capture_limit_max))
            .unwrap_or(self.stderr_capture_limit);
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);

        // Configure stdin
        match stdin_source {
            StdinSource::Inherit | StdinSource::String => {
                cmd.stdin(std::process::Stdio::piped());
            }
            StdinSource::None => {
                cmd.stdin(std::process::Stdio::null());
            }
        }

        // Configure stdout
        match stdout_dest {
            StdoutDest::Passthrough => {
                cmd.stdout(std::process::Stdio::inherit());
            }
            StdoutDest::Capture => {
                cmd.stdout(std::process::Stdio::piped());
            }
            StdoutDest::Discard => {
                cmd.stdout(std::process::Stdio::null());
            }
        }

        // Always capture stderr
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // Handle stdin
        if let Some(mut child_stdin) = child.stdin.take() {
            match stdin_source {
                StdinSource::Inherit => {
                    // Write peek buffer first
                    let peek_data = self.peek_buffer.take();
                    if !peek_data.is_empty() {
                        child_stdin.write_all(&peek_data).await?;
                    }

                    // Forward remaining original stdin to child's stdin
                    let mut stdin = self.stdin.lock().await;
                    let copy_result = tokio::io::copy(&mut *stdin, &mut child_stdin).await;
                    drop(child_stdin);
                    if let Err(e) = copy_result {
                        tracing::warn!("Error forwarding stdin to command: {}", e);
                    }
                }
                StdinSource::String => {
                    let text = stdin_text.unwrap_or("");
                    child_stdin.write_all(text.as_bytes()).await?;
                    drop(child_stdin);
                }
                StdinSource::None => {
                    drop(child_stdin);
                }
            }
        }

        // Handle stdout capture
        let stdout = if stdout_dest == StdoutDest::Capture {
            if let Some(mut child_stdout) = child.stdout.take() {
                let mut buf = Vec::new();
                let limit = stdout_limit;
                let mut truncated = false;

                let mut total = 0;
                let mut chunk = [0u8; 8192];
                loop {
                    let to_read = (limit + 1).saturating_sub(total).min(chunk.len());
                    if to_read == 0 {
                        truncated = true;
                        break;
                    }
                    match child_stdout.read(&mut chunk[..to_read]).await {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            total += n;
                            if total > limit {
                                truncated = true;
                                buf.truncate(limit);
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                // If truncated, drain remaining output
                if truncated {
                    let mut drain = Vec::new();
                    let _ = child_stdout.read_to_end(&mut drain).await;
                }

                let text = String::from_utf8_lossy(&buf).into_owned();
                Some((text, truncated))
            } else {
                None
            }
        } else {
            None
        };

        // Capture stderr (with drain to prevent child from blocking on full pipe)
        let stderr = if let Some(mut child_stderr) = child.stderr.take() {
            let mut buf = Vec::new();
            // Read up to stderr_limit bytes
            {
                let mut limited = (&mut child_stderr).take(stderr_limit as u64);
                let _ = limited.read_to_end(&mut buf).await;
            }
            // Drain remaining stderr so child doesn't block on a full pipe
            let mut drain = Vec::new();
            let _ = child_stderr.read_to_end(&mut drain).await;
            let text = String::from_utf8_lossy(&buf).into_owned();
            if text.is_empty() { None } else { Some(text) }
        } else {
            None
        };

        // Wait for the command to finish
        let status = child.wait().await?;
        let exit_code = status.code().unwrap_or(-1);

        let (stdout_text, stdout_truncated) = match stdout {
            Some((t, tr)) => (Some(t), Some(tr)),
            None => (None, None),
        };

        Ok(CommandResult {
            exit_code,
            stdout: stdout_text,
            stdout_truncated,
            stderr,
        })
    }
}

async fn peek_bytes(
    stdin: &mut tokio::io::Stdin,
    peek_buffer: &mut PeekBuffer,
    max_bytes: usize,
) -> std::io::Result<crate::peek::PeekResult> {
    let mut buf = vec![0u8; max_bytes.min(1024 * 1024)];
    let mut total_read = 0;
    let mut truncated = false;

    while total_read < max_bytes {
        let remaining = max_bytes - total_read;
        let to_read = remaining.min(buf.len());
        match stdin.read(&mut buf[..to_read]).await {
            Ok(0) => break,
            Ok(n) => {
                peek_buffer.append(&buf[..n]);
                total_read += n;
                if n < to_read {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }

    // Check if there's more data
    if total_read == max_bytes {
        let mut probe = [0u8; 1];
        if let Ok(1) = stdin.read(&mut probe).await {
            truncated = true;
            peek_buffer.append(&probe);
        }
    }

    let text = String::from_utf8_lossy(peek_buffer.data()).into_owned();
    Ok(crate::peek::PeekResult {
        text,
        bytes_read: total_read,
        truncated,
    })
}

async fn peek_lines(
    stdin: &mut tokio::io::Stdin,
    peek_buffer: &mut PeekBuffer,
    max_lines: usize,
) -> std::io::Result<crate::peek::PeekResult> {
    let mut buf = Vec::new();
    let mut lines = 0usize;
    let mut chunk = [0u8; 8192];
    let mut truncated = false;

    loop {
        match stdin.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                let chunk_lines = chunk[..n].iter().filter(|&&b| b == b'\n').count();
                let new_total = lines + chunk_lines;

                if new_total > max_lines {
                    // Find the cut point for the RESULT (first N lines only),
                    // but store the ENTIRE chunk in the peek buffer so no data is lost.
                    let allowed = max_lines - lines;
                    let mut cut_at = n;
                    let mut newlines = 0;
                    for (i, &b) in chunk[..n].iter().enumerate() {
                        if b == b'\n' {
                            newlines += 1;
                            if newlines > allowed {
                                cut_at = i;
                                break;
                            }
                        }
                    }
                    // Result: only the first N lines
                    buf.extend_from_slice(&chunk[..cut_at]);
                    // Peek buffer: the ENTIRE chunk (all will be replayed to commands)
                    peek_buffer.append(&chunk[..n]);
                    truncated = true;
                    break;
                }

                buf.extend_from_slice(&chunk[..n]);
                peek_buffer.append(&chunk[..n]);
                lines = new_total;

                if lines >= max_lines {
                    // We read exactly max_lines or slightly more; all stored in buffer
                    truncated = true;
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }

    let text = String::from_utf8_lossy(&buf).into_owned();
    Ok(crate::peek::PeekResult {
        text,
        bytes_read: buf.len(),
        truncated,
    })
}
