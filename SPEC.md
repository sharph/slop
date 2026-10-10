# slop — Specification

> A pipe-aware LLM harness. `slop` detects its position in a Unix pipeline and gives the LLM tools to peek at, transform, and route the data stream.

---

## 1. Overview

`slop` is a command-line tool that calls an LLM (any OpenAI-compatible API). Unlike a simple "prompt → response" wrapper, `slop` is a **stream orchestrator**:

- When stdin is a **pipe**, `slop` buffers a peek of the head and gives the LLM tools to inspect and transform the stream.
- When stdin is a **TTY**, `slop` simply relays the prompt and the LLM's response is the output.
- When stdout is a **pipe**, `slop` ensures only data (not commentary) flows to the next command in the chain.

The LLM is given tools — not the raw data. For large streams (gigabytes of logs), the LLM never loads the data into its context. It runs real shell commands (`grep`, `awk`, `sort`, Python one-liners) that stream stdin → stdout, and only captures output when it needs to analyze it.

### The Big Idea

```
cat access.log | slop "filter out 404s" | slop "what are the most popular pages?"
```

- First `slop`: LLM peeks at the log, runs `grep -v ' 404 '` with stdout passthrough. Streams through untouched by the LLM.
- Second `slop`: LLM peeks at the filtered log, runs `awk '{print $7}' | sort | uniq -c | sort -rn | head -20` with stdout **captured**, reads the result, then writes a human summary to stdout.

---

## 2. Design Principles

| Principle | Rationale |
|-----------|-----------|
| **LLM as orchestrator, not processor** | The LLM decides *what* to do; shell commands do the *work* on data. |
| **Streams are never fully loaded** | Gigabyte-scale inputs flow through pipes. The LLM sees samples and command output, not raw data. |
| **stdout is sacred** | Only data goes to stdout. LLM commentary goes to stderr. |
| **Unix philosophy** | Small, composable, works in pipelines. Exit codes propagate. |
| **Trust the LLM** | No sandboxing in v1. The LLM can run arbitrary shell commands. |
| **OpenAI-compatible** | Works with OpenAI, Ollama, llama.cpp, LM Studio, vLLM, etc. |

---

## 3. CLI Interface

### Synopsis

```
slop [OPTIONS] <PROMPT>
```

### Arguments

| Argument | Description |
|----------|-------------|
| `<PROMPT>` | The instruction for the LLM. Required. |

### Options

| Flag | Env Var | Config Key | Description |
|------|---------|------------|-------------|
| `--model <MODEL>` | `SLOP_MODEL` | `model` | Model name (e.g., `gpt-4o`, `llama3.1`) |
| `--api-base <URL>` | `SLOP_API_BASE` | `api_base` | OpenAI-compatible API base URL |
| `--api-key <KEY>` | `SLOP_API_KEY` | `api_key` | API key |
| `--system <TEXT>` | `SLOP_SYSTEM` | `system` | Additional system prompt text |
| `--max-iterations <N>` | `SLOP_MAX_ITERATIONS` | `max_iterations` | Max tool-calling rounds (default: 20) |
| `--peek-bytes <N>` | `SLOP_PEEK_BYTES` | `peek_bytes` | Default peek size in bytes (default: 4096) |
| `--capture-limit <N>` | `SLOP_CAPTURE_LIMIT` | `capture_limit` | Default max bytes captured from command stdout (default: 4096) |
| `--capture-limit-max <N>` | `SLOP_CAPTURE_LIMIT_MAX` | `capture_limit_max` | Max bytes the LLM can request for stdout capture (default: 65536) |
| `--stderr-capture-limit <N>` | `SLOP_STDERR_CAPTURE_LIMIT` | `stderr_capture_limit` | Default max bytes captured from command stderr (default: 4096) |
| `--stderr-capture-limit-max <N>` | `SLOP_STDERR_CAPTURE_LIMIT_MAX` | `stderr_capture_limit_max` | Max bytes the LLM can request for stderr capture (default: 65536) |
| `--verbose` | `SLOP_VERBOSE` | `verbose` | Print LLM reasoning to stderr |
| `--help` | | | Print help |
| `--version` | | | Print version |

### Configuration File

Located at `~/.config/slop/config.toml` (or `$XDG_CONFIG_HOME/slop/config.toml`):

```toml
model = "gpt-4o"
api_base = "https://api.openai.com/v1"
api_key = "sk-..."
max_iterations = 20
peek_bytes = 4096
capture_limit = 4096
capture_limit_max = 65536
stderr_capture_limit = 4096
stderr_capture_limit_max = 65536
verbose = false
```

Precedence: **CLI flag > env var > config file > default**

---

## 4. Environment Detection

On startup, `slop` inspects stdin and stdout:

| Stream | Types | Detection |
|--------|-------|-----------|
| **stdin** | `Pipe`, `File`, `Tty`, `Null` | `std::io::stdin().is_terminal()` + metadata |
| **stdout** | `Pipe`, `File`, `Tty`, `Null` | `std::io::stdout().is_terminal()` + metadata |

This information is passed to the LLM in the system prompt so it knows whether data is flowing in/out or if it's interacting with a human.

### stdin handling by type

| stdin type | Behavior |
|------------|----------|
| `Pipe` | Peekable. `peek_input` reads head into a buffer; `run_command` with `stdin_source="inherit"` replays the buffer then forwards the rest of the pipe. |
| `File` | Same as `Pipe` (seekable, but treated the same for simplicity). |
| `Tty` | Not peekable. `peek_input` returns empty with a note. The LLM should not try to read stdin. |
| `Null` | Not peekable. Same as `Tty`. |

---

## 5. LLM Integration

### API

- **Protocol**: OpenAI Chat Completions API (`POST /chat/completions`)
- **Tool calling**: OpenAI function calling format (`tools` + `tool_choice: "auto"`)
- **No streaming** of LLM responses (tool calls require complete responses). Streaming of *data* is handled by passthrough commands.

### Request shape

```json
{
  "model": "<model>",
  "messages": [
    { "role": "system", "content": "<system prompt>" },
    { "role": "user", "content": "<user prompt>" }
  ],
  "tools": [
    { "type": "function", "function": { "name": "peek_input", ... } },
    { "type": "function", "function": { "name": "run_command", ... } },
    { "type": "function", "function": { "name": "write_to_stdout", ... } },
    { "type": "function", "function": { "name": "write_to_stderr", ... } }
  ],
  "tool_choice": "auto"
}
```

### System Prompt

The system prompt is constructed dynamically based on the environment:

```
You are an AI assistant running inside a command-line tool called `slop`.

The user invoked: slop "<PROMPT>"

Environment:
- stdin: <description>   (e.g., "a pipe from another command" / "an interactive terminal")
- stdout: <description>  (e.g., "a pipe to another command" / "an interactive terminal")

You have access to tools that let you inspect and manipulate the data stream.

## Tools

### peek_input
Read the first N bytes or lines from stdin. The peeked data is buffered and will be
replayed to any subsequent command that reads from stdin (stdin_source="inherit").
Use this to understand the shape of the input before deciding what to do.

Parameters:
- bytes (integer, optional): Max bytes to read. Default: 4096.
- lines (integer, optional): Max lines to read. If both bytes and lines are given, lines takes precedence.

Returns: The peeked text. If stdin is a terminal, returns empty with a note.

### run_command
Execute a shell command. The command runs in `sh -c`. You control where its stdin
comes from and where its stdout goes.

Parameters:
- command (string, required): The shell command to execute.
- stdin_source (string, optional): Where the command reads stdin from.
    "inherit" (default): The original stdin, with any peeked data replayed first.
    "none": /dev/null (no input).
    "string": Use the stdin_text parameter as input.
- stdin_text (string, optional): Text to use as stdin when stdin_source="string".
- stdout_dest (string, optional): Where the command's stdout goes.
    "passthrough" (default): Directly to slop's stdout. Use this for large streams
      that should flow through without loading into your context.
    "capture": Captured and returned to you. Use this when you need to analyze
      the output before deciding what to do.
    "discard": /dev/null.
- capture_limit (integer, optional): Override the stdout capture limit in bytes.
    Default: 4096. Maximum: 65536. Only used when stdout_dest="capture".
- stderr_capture_limit (integer, optional): Override the stderr capture limit
    in bytes. Default: 4096. Maximum: 65536.

Returns: { "exit_code": <int>, "stdout": <string|null>, "stdout_truncated": <bool>, "stderr": <string|null> }
- If stdout_dest="passthrough" or "discard", stdout is null.
- If stdout_dest="capture", stdout contains the captured output (truncated at
  the capture limit). stdout_truncated is true if output was truncated.
- stderr is always captured (up to the stderr capture limit) and returned.

IMPORTANT: Once a command consumes stdin (stdin_source="inherit"), the original
stdin is exhausted. Subsequent commands with stdin_source="inherit" will get
immediate EOF.

### write_to_stdout
Write text to slop's stdout. Can be called multiple times; text is appended in order.
Use this to produce your final output. Set close=true on the final call to close the
stream (sends EOF to downstream commands).

Parameters:
- text (string, required): The text to write.
- close (boolean, optional): If true, close stdout after writing. Sends EOF to any
  downstream command in a pipeline. Use on the final write_to_stdout call. Default: false.

### write_to_stderr
Write text to slop's stderr. This is visible to the user but does NOT pollute
the stdout data stream. Use this for progress messages, explanations, or commentary.

Parameters:
- text (string, required): The text to write.

## Guidelines

1. Always peek_input first when stdin is a pipe, to understand the data shape.
2. For large streams, use run_command with stdout_dest="passthrough" to stream
   data through without loading it into your context.
3. Use stdout_dest="capture" only when you need to analyze command output.
4. Chain shell commands with pipes (e.g., "grep foo | sort | uniq -c") rather
   than making multiple run_command calls.
5. When stdin is a terminal, respond directly for simple questions. But for tasks
   involving computation, data generation, or anything a shell command could do more
   reliably, prefer run_command — even without piped input. For example,
   `slop "generate fib numbers < 10000"` should run a Python one-liner rather than
   computing the numbers yourself. Your text response goes to stdout automatically
   when you don't use tools.
6. Your text responses (without tool calls) are written to stdout as the final
   output. When you also make tool calls, your text is shown on stderr as commentary.
7. Exit code: slop exits with the exit code of the last run_command, or 0 if no
   command was run.
```

### Conversation Loop

```
1. Detect environment (stdin/stdout types)
2. Build system prompt with environment info
3. messages = [system, user(prompt)]
4. Loop (up to max_iterations):
   a. Call LLM API with tools
   b. If response has text content AND no tool calls:
      → Write text to stdout (final output)
      → Exit with last_command_exit_code (default 0)
   c. If response has text content AND tool calls:
      → Write text to stderr (commentary)
      → Execute each tool call sequentially
      → Append assistant message + tool results to messages
      → Continue loop
   d. If response has only tool calls (no text):
      → Execute each tool call sequentially
      → Append assistant message + tool results to messages
      → Continue loop
5. If max_iterations exceeded: error to stderr, exit 1
```

**Stdout discipline**: Once any output has been written to stdout (via `write_to_stdout`
or `run_command` with `passthrough`), the LLM's final text response goes to **stderr**
(commentary), not stdout. This prevents the LLM from duplicating output or polluting
the data stream with explanations.

**Terminal tool calls**: `run_command` with `stdout_dest="passthrough"` and
`write_to_stdout(close=true)` are **terminal** — slop exits immediately after the
tool completes, with the exit code of the command (or 0 for `write_to_stdout`).
The conversation does not continue. This ensures the command's output IS the final
output, with no risk of the LLM adding more data afterward.

**Closing stdout**: The LLM can close stdout early via `write_to_stdout(close=true)`.
This flushes and closes fd 1 at the OS level, sending EOF to downstream commands
immediately — useful in pipelines where the downstream command should not wait for
slop to exit. After closing, no more writes to stdout are possible.

**Content format compatibility**: The `content` field in LLM responses can be either
a plain string (OpenAI) or an array of content parts (Mistral, etc.). Only `"text"`
type parts are extracted; other types (e.g., `"thinking"`) are ignored.

---

## 6. Tool Specifications

### 6.1 `peek_input`

**Purpose**: Read a sample of stdin without losing data.

**Schema**:

```json
{
  "name": "peek_input",
  "description": "Read the first N bytes or lines from stdin. Peeked data is buffered and replayed to subsequent commands.",
  "parameters": {
    "type": "object",
    "properties": {
      "bytes": {
        "type": "integer",
        "description": "Max bytes to read (default: 4096)"
      },
      "lines": {
        "type": "integer",
        "description": "Max lines to read. Takes precedence over bytes if both given."
      }
    }
  }
}
```

**Behavior**:

1. If stdin is a TTY or Null: return `"(stdin is a terminal — no piped input to peek)"`
2. Read up to N bytes (or lines) from stdin
3. Append to internal `peek_buffer`
4. Return the text to the LLM

**Implementation note**: The peek buffer is prepended to stdin for any subsequent `run_command` with `stdin_source="inherit"`. Peeking multiple times accumulates in the buffer.

**Return**:

```json
{
  "peeked": "<text>",
  "bytes_read": 4096,
  "truncated": true
}
```

---

### 6.2 `run_command`

**Purpose**: Execute a shell command with full control over stdin/stdout routing.

**Schema**:

```json
{
  "name": "run_command",
  "description": "Execute a shell command with control over stdin source and stdout destination.",
  "parameters": {
    "type": "object",
    "properties": {
      "command": {
        "type": "string",
        "description": "Shell command to execute (run via sh -c)"
      },
      "stdin_source": {
        "type": "string",
        "enum": ["inherit", "none", "string"],
        "description": "Where stdin comes from. 'inherit' = original stdin (with peek buffer replayed), 'none' = /dev/null, 'string' = use stdin_text. Default: 'inherit'."
      },
      "stdin_text": {
        "type": "string",
        "description": "Text to use as stdin when stdin_source='string'"
      },
      "stdout_dest": {
        "type": "string",
        "enum": ["passthrough", "capture", "discard"],
        "description": "Where stdout goes. 'passthrough' = directly to slop's stdout, 'capture' = returned to you, 'discard' = /dev/null. Default: 'passthrough'."
      },
      "capture_limit": {
        "type": "integer",
        "description": "Override the stdout capture limit in bytes. Default: 4096. Maximum: 65536. Only used when stdout_dest='capture'."
      },
      "stderr_capture_limit": {
        "type": "integer",
        "description": "Override the stderr capture limit in bytes. Default: 4096. Maximum: 65536."
      }
    },
    "required": ["command"]
  }
}
```

**Behavior by `stdin_source`**:

| Source | Implementation |
|--------|---------------|
| `inherit` | Spawn command with piped stdin. Write `peek_buffer` to command's stdin. Then forward remaining original stdin → command's stdin (via a pump thread/task). Close stdin when original is exhausted. Clear `peek_buffer` after writing. |
| `none` | Command's stdin = `/dev/null` |
| `string` | Command's stdin = piped. Write `stdin_text` to it. Close immediately. |

**Behavior by `stdout_dest`**:

| Dest | Implementation |
|------|---------------|
| `passthrough` | Command's stdout = inherit slop's stdout FD. Data flows directly. No copying through slop. |
| `capture` | Command's stdout = piped. Read up to `capture_limit` bytes. Truncate with a note if exceeded. Return in tool result. |
| `discard` | Command's stdout = `/dev/null` |

**stderr**: Always captured (up to the stderr capture limit, default 4096 bytes) and returned in the tool result. Not sent to slop's stderr (to avoid confusing the user with command errors that the LLM is handling).

**Capture limits**: The LLM can override the default capture limits via the `capture_limit` and `stderr_capture_limit` parameters. Values are clamped to the configured maximum (`capture_limit_max` / `stderr_capture_limit_max`, default 65536).

**Return**:

```json
{
  "exit_code": 0,
  "stdout": "<captured output or null>",
  "stdout_truncated": false,
  "stderr": "<captured stderr or null>"
}
```

**Exit code tracking**: slop remembers the exit code of the last `run_command`. If no command was run, exit code is 0.

**Concurrency**: Tool calls within a single LLM response are executed **sequentially**, in order.

---

### 6.3 `write_to_stdout`

**Purpose**: Write text to slop's stdout. Can optionally close the stream.

**Schema**:

```json
{
  "name": "write_to_stdout",
  "description": "Write text to slop's stdout. Can be called multiple times; text is appended in order. Set close=true on the final call to close the stream (sends EOF to downstream commands).",
  "parameters": {
    "type": "object",
    "properties": {
      "text": {
        "type": "string",
        "description": "Text to write to stdout"
      },
      "close": {
        "type": "boolean",
        "description": "If true, close stdout after writing. Sends EOF to downstream. Use on the final write_to_stdout call. Default: false."
      }
    },
    "required": ["text"]
  }
}
```

**Behavior**: Write text to stdout immediately. Flush after each call.

If `close=true`:
1. Write the text and flush
2. Close fd 1 at the OS level (`libc::close(STDOUT_FILENO)`) — sends EOF downstream
3. Mark stdout as closed — subsequent `write_to_stdout` calls and `run_command` with `stdout_dest="passthrough"` return an error

**Return**:

```json
{
  "bytes_written": 42,
  "closed": true
}
```

(`closed` is only present when `close=true` was passed.)

---

### 6.4 `write_to_stderr`

**Purpose**: Write text to slop's stderr (commentary, progress, not data).

**Schema**:

```json
{
  "name": "write_to_stderr",
  "description": "Write text to slop's stderr. Visible to the user but does not pollute stdout.",
  "parameters": {
    "type": "object",
    "properties": {
      "text": {
        "type": "string",
        "description": "Text to write to stderr"
      }
    },
    "required": ["text"]
  }
}
```

**Behavior**: Write text to stderr immediately. Flush after each call.

**Return**:

```json
{
  "bytes_written": 42
}
```

---

## 7. Execution Flow (Detailed)

### Startup

```
1. Parse CLI args, load config (file → env → flags)
2. Validate: model is set, api_base is set, api_key is set (if required by endpoint)
3. Detect stdin type and stdout type
4. Initialize peek_buffer = empty
5. Initialize last_exit_code = 0
```

### Peek Implementation

```
peek_input(bytes=N or lines=M):
  if stdin is TTY or Null:
    return "(stdin is a terminal — no piped input)"
  if lines specified:
    read up to M lines from stdin, append to peek_buffer
  else:
    read up to N bytes from stdin, append to peek_buffer
  return peeked text
```

### run_command Implementation

```
run_command(command, stdin_source, stdin_text, stdout_dest):
  spawn sh -c <command> with:
    stdin:  pipe (for inherit/string) or null (for none)
    stdout: inherit (for passthrough) or pipe (for capture) or null (for discard)
    stderr: pipe (always captured)

  if stdin_source == "inherit":
    write peek_buffer to child's stdin
    clear peek_buffer
    spawn pump task: copy remaining original stdin → child's stdin
  elif stdin_source == "string":
    write stdin_text to child's stdin
  close child's stdin

  if stdout_dest == "capture":
    read child's stdout up to capture_limit bytes

  read child's stderr up to 64 KiB
  wait for child to exit
  last_exit_code = child's exit code

  return { exit_code, stdout (or null), stdout_truncated, stderr }
```

### Main Loop

```
messages = [system_prompt, user_prompt]

for iteration in 1..=max_iterations:
  response = llm.chat(messages, tools)
  assistant_msg = response.message
  messages.push(assistant_msg)

  if assistant_msg.content is not empty:
    if assistant_msg.tool_calls is empty:
      # Final response — write to stdout
      stdout.write(assistant_msg.content)
      exit(last_exit_code)
    else:
      # Commentary — write to stderr
      stderr.write(assistant_msg.content)

  if assistant_msg.tool_calls is not empty:
    for tool_call in assistant_msg.tool_calls:
      result = execute_tool(tool_call)
      messages.push({
        role: "tool",
        tool_call_id: tool_call.id,
        content: json(result)
      })
    continue

  # No content and no tool calls — shouldn't happen, but treat as empty response
  exit(last_exit_code)

# Max iterations exceeded
stderr.write("Error: max iterations exceeded")
exit(1)
```

---

## 8. Error Handling

| Scenario | Behavior |
|----------|----------|
| LLM API error (network, auth, rate limit) | Print error to stderr, exit 1 |
| LLM returns malformed response | Print error to stderr, exit 1 |
| `run_command` fails to spawn | Return error in tool result, LLM can retry |
| `run_command` exits non-zero | Return exit code in tool result. For `passthrough`, slop exits immediately with this code. For `capture`/`discard`, LLM decides what to do. |
| `run_command` with `passthrough` completes | **TERMINAL**: slop exits immediately with the command's exit code |
| `write_to_stdout(close=true)` completes | **TERMINAL**: slop exits immediately with code 0 |
| Max iterations exceeded | Error to stderr, exit 1 |
| Missing config (no model/api_key) | Error to stderr with helpful message, exit 1 |
| Broken pipe (downstream command exits early) | Exit gracefully with SIGPIPE behavior (exit code 141 or 0) |
| Capture limit exceeded | Truncate, set `stdout_truncated: true` in result |
| LLM requests capture limit above max | Clamp to configured maximum |
| Write to stdout after `close=true` | Return error: "stdout is already closed" |
| `run_command` with `passthrough` after `close=true` | Return error: stdout is closed |

---

## 9. Examples

### Example 1: Simple generation (stdin = TTY)

```bash
$ slop "generate numbers between 1 and 5, one number per line"
1
2
3
4
5
```

LLM responds with text, no tools. Text → stdout.

### Example 2: The original chain

```bash
$ slop "generate numbers between 1 and 5, one number per line" \
  | slop "sort list numerically" \
  | slop "for each line, emit one x for the number specified"
x
xx
xxx
xxxx
xxxxx
```

- **slop 1**: stdin=TTY, stdout=pipe. LLM generates "1\n2\n3\n4\n5" → stdout.
- **slop 2**: stdin=pipe, stdout=pipe. LLM peeks (sees numbers), runs `sort -n` with passthrough.
- **slop 3**: stdin=pipe, stdout=TTY. LLM peeks (sees sorted numbers), runs a Python one-liner:
  ```python
  python3 -c "import sys; [print('x' * int(l)) for l in sys.stdin]"
  ```
  with passthrough.

### Example 3: Large log analysis

```bash
$ cat 5gb_access.log | slop "filter out 404s" | slop "what are the most popular pages?"
The most popular pages are:
1. /home       (152,341 hits)
2. /about      (89,203 hits)
3. /products   (67,891 hits)
...
```

- **slop 1**: stdin=pipe (5GB), stdout=pipe. LLM peeks (sees log format), runs `grep -v ' 404 '` with `stdout_dest="passthrough"`. Streams 5GB through without loading into context.
- **slop 2**: stdin=pipe (filtered), stdout=TTY. LLM peeks, runs `awk '{print $7}' | sort | uniq -c | sort -rn | head -20` with `stdout_dest="capture"`. Reads the top-20 list, then calls `write_to_stdout` with a formatted summary.

### Example 4: LLM writes its own output after analysis

```bash
$ cat data.csv | slop "summarize this CSV: count rows, list columns, find any anomalies"
```

LLM peeks, runs `wc -l` (capture), `head -1` (capture), maybe `awk` for stats (capture), then `write_to_stdout` with a summary.

### Example 5: Multi-step reasoning

```bash
$ cat messy.txt | slop "clean up this text: fix encoding, remove duplicates, sort lines"
```

LLM might:
1. `peek_input` → sees mojibake and duplicates
2. `run_command("iconv -f latin1 -t utf-8", stdout_dest="capture")` → cleaned text
3. `run_command("sort -u", stdin_source="string", stdin_text=<cleaned>, stdout_dest="passthrough")` → sorted unique lines to stdout

---

## 10. Implementation Notes (Rust)

### Crates

| Crate | Purpose |
|-------|---------|
| `clap` (derive) | CLI argument parsing |
| `tokio` (full) | Async runtime for concurrent I/O |
| `reqwest` | HTTP client for LLM API |
| `serde` + `serde_json` | Serialization |
| `toml` | Config file parsing |
| `dirs` | Config directory resolution |
| `is-terminal` / `std::io::IsTerminal` | TTY detection |
| `anyhow` / `thiserror` | Error handling |
| `tracing` / `tracing-subscriber` | Logging (verbose mode) |

### Project Structure

```
slop/
├── Cargo.toml
├── SPEC.md
└── src/
    ├── main.rs          # Entry point, CLI parsing, main loop
    ├── config.rs        # Config loading (file → env → flags → defaults)
    ├── env.rs           # Environment detection (stdin/stdout types)
    ├── llm.rs           # OpenAI-compatible API client
    ├── tools.rs         # Tool schemas (JSON) + tool dispatch
    ├── peek.rs          # Stdin peek buffer (read head, replay later)
    ├── command.rs       # Command spawning with pipe control
    └── prompt.rs        # System prompt construction
```

### Key Implementation Details

**Peek buffer**: A `Vec<u8>` that accumulates peeked data. When `run_command` uses `stdin_source="inherit"`, the buffer is written to the child's stdin first, then a tokio task pumps the remaining original stdin.

```rust
struct PeekBuffer {
    data: Vec<u8>,
}

impl PeekBuffer {
    async fn peek(&mut self, stdin: &mut Stdin, max: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; max];
        let n = stdin.read(&mut buf).await?;
        buf.truncate(n);
        self.data.extend_from_slice(&buf);
        Ok(buf)
    }
}
```

**Command spawning** (simplified):

```rust
async fn run_command(
    command: &str,
    stdin_source: StdinSource,
    stdin_text: Option<&str>,
    stdout_dest: StdoutDest,
    peek_buffer: &mut PeekBuffer,
    original_stdin: &mut Stdin,
    capture_limit: usize,
) -> Result<CommandResult> {
    let mut cmd = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(/* pipe or null */)
        .stdout(/* inherit, pipe, or null */)
        .stderr(Stdio::piped())
        .spawn()?;

    // Handle stdin
    if let Some(mut child_stdin) = cmd.stdin.take() {
        match stdin_source {
            StdinSource::Inherit => {
                // Write peek buffer
                child_stdin.write_all(&peek_buffer.data).await?;
                peek_buffer.data.clear();
                // Pump remaining original stdin
                let mut original = original_stdin.lock();
                tokio::io::copy(&mut original, &mut child_stdin).await?;
            }
            StdinSource::String => {
                child_stdin.write_all(stdin_text.unwrap_or("").as_bytes()).await?;
            }
            StdinSource::None => {}
        }
    }

    // Handle stdout capture
    let stdout = if stdout_dest == StdoutDest::Capture {
        let mut out = cmd.stdout.take().unwrap();
        let mut buf = Vec::new();
        out.take(capture_limit as u64).read_to_end(&mut buf).await?;
        Some(String::from_utf8_lossy(&buf).into_owned())
    } else {
        None
    };

    // Capture stderr
    let stderr = {
        let mut err = cmd.stderr.take().unwrap();
        let mut buf = Vec::new();
        err.take(65536).read_to_end(&mut buf).await?;
        String::from_utf8_lossy(&buf).into_owned()
    };

    let status = cmd.wait().await?;
    Ok(CommandResult {
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr: Some(stderr),
    })
}
```

**Passthrough stdout**: Use `Stdio::inherit()` so the child writes directly to slop's stdout file descriptor. Zero-copy streaming.

**LLM client** (simplified):

```rust
async fn chat(
    client: &reqwest::Client,
    config: &Config,
    messages: &[Message],
    tools: &[Tool],
) -> Result<ChatResponse> {
    let resp = client
        .post(format!("{}/chat/completions", config.api_base))
        .bearer_auth(&config.api_key)
        .json(&json!({
            "model": config.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(resp)
}
```

### Testing Strategy

| Layer | Approach |
|-------|----------|
| Unit tests | Peek buffer, config parsing, environment detection, tool schema validation |
| Integration tests | Spawn `slop` with piped stdin/stdout, mock LLM API (via `wiremock` or a local mock server) |
| E2E tests | Full pipeline: `slop ... \| slop ... \| slop ...` with a mock or real LLM |
| Manual tests | Real-world pipelines with logs, CSVs, etc. |

---

## 11. Open Questions / Future Work

- [ ] **Streaming LLM responses**: Currently no streaming. Could add for the "generate text" case (stdin=TTY).
- [ ] **More stdin sources**: Read from a file path, URL, etc.
- [ ] **More stdout destinations**: Write to a file, append mode, etc.
- [ ] **Parallel tool calls**: Execute independent tool calls concurrently.
- [ ] **Sandboxing**: Run commands in a restricted environment (bubblewrap, containers).
- [ ] **Caching**: Cache LLM responses for identical prompts + peeked input.
- [ ] **Interactive mode**: When stdin is a TTY, support a multi-turn conversation.
- [ ] **Shell completions**: Bash/zsh/fish completions.
- [ ] **Man page**: Full Unix man page.

---

## 12. Version History

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0 (draft) | 2026-10-07 | Initial specification |

---

*This is a living document. Update as the design evolves.*
