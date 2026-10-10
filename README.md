# slop

> A pipe-aware LLM harness. Detects pipes and gives the LLM tools to inspect and transform the data stream.

```
slop "generate numbers between 1 and 5, one number per line" | slop "sort list numerically" | slop "for each line, emit one x for the number specified"
x
xx
xxx
xxxx
xxxxx
```

## What is slop?

`slop` is a command-line tool that calls an LLM (any OpenAI-compatible API). Unlike a simple "prompt → response" wrapper, `slop` is a **stream orchestrator**:

- When stdin is a **pipe**, `slop` gives the LLM tools to peek at, transform, and route the data stream.
- When stdin is a **TTY**, `slop` simply relays the prompt and the LLM's response is the output.
- When stdout is a **pipe**, `slop` ensures only data (not commentary) flows to the next command.

The LLM is given **tools**, not raw data. For large streams (gigabytes of logs), the LLM never loads the data into its context. It runs real shell commands (`grep`, `awk`, `sort`, Python one-liners) that stream stdin → stdout, and only captures output when it needs to analyze it.

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
# Binary at: target/release/slop
```

## Usage

```
slop [OPTIONS] <PROMPT>
```

### Options

| Flag | Env Var | Description |
|------|---------|-------------|
| `--model <MODEL>` | `SLOP_MODEL` | Model name (e.g., `gpt-4o`, `llama3.1`) |
| `--api-base <URL>` | `SLOP_API_BASE` | OpenAI-compatible API base URL |
| `--api-key <KEY>` | `SLOP_API_KEY` | API key |
| `--system <TEXT>` | `SLOP_SYSTEM` | Additional system prompt text |
| `--max-iterations <N>` | `SLOP_MAX_ITERATIONS` | Max tool-calling rounds (default: 20) |
| `--peek-bytes <N>` | `SLOP_PEEK_BYTES` | Default peek size in bytes (default: 4096) |
| `--capture-limit <N>` | `SLOP_CAPTURE_LIMIT` | Default max bytes captured from command stdout (default: 4096) |
| `--capture-limit-max <N>` | `SLOP_CAPTURE_LIMIT_MAX` | Max bytes the LLM can request for stdout capture (default: 65536) |
| `--stderr-capture-limit <N>` | `SLOP_STDERR_CAPTURE_LIMIT` | Default max bytes captured from command stderr (default: 4096) |
| `--stderr-capture-limit-max <N>` | `SLOP_STDERR_CAPTURE_LIMIT_MAX` | Max bytes the LLM can request for stderr capture (default: 65536) |
| `-v, --verbose` | `SLOP_VERBOSE` | Print LLM reasoning to stderr |

### Configuration File

Located at `~/.config/slop/config.toml`:

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

## How It Works

1. `slop` detects stdin/stdout types (pipe, file, TTY)
2. Calls the LLM with a system prompt explaining the environment and available tools
3. The LLM decides what to do using these tools:

### Tools

| Tool | Purpose |
|------|---------|
| `peek_input` | Read the head of stdin (buffered, replayed to commands — doesn't lose data) |
| `run_command` | Execute a shell command with configurable stdin source and stdout destination |
| `write_to_stdout` | Write text to stdout. Optional `close=true` parameter closes the stream (sends EOF to downstream commands) |
| `write_to_stderr` | Write text to stderr (commentary, doesn't pollute the pipe) |

### `run_command` options

- **`stdin_source`**: `"inherit"` (original stdin + peek buffer), `"none"`, or `"string"` (inline text)
- **`stdout_dest`**: `"passthrough"` (direct to slop's stdout — for streaming), `"capture"` (return to LLM), or `"discard"`
- **`capture_limit`**: Override stdout capture limit in bytes (default: 4096, max: 65536)
- **`stderr_capture_limit`**: Override stderr capture limit in bytes (default: 4096, max: 65536)

## Examples

### Simple generation

```bash
$ slop "generate numbers between 1 and 5, one number per line"
1
2
3
4
5
```

### Computation (uses run_command)

```bash
$ slop "generate fib numbers < 10000"
0
1
1
2
3
5
8
...
```

### Full pipeline

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

### Large log analysis

```bash
$ cat 5gb_access.log | slop "filter out 404s" | slop "what are the most popular pages?"
Top pages by hits:
   152341 /home
    89203 /about
    67891 /products
```

The first `slop` streams 5GB through `grep` without loading it into the LLM's context. The second `slop` runs an `awk | sort | uniq -c` pipeline, captures the top results, and writes a summary.

### With a local model (Ollama)

```bash
$ slop --api-base http://localhost:11434/v1 --model llama3.1 "summarize this" < article.txt
```

## Development

```bash
# Build
cargo build

# Run tests (requires Python 3 for mock server)
./test/run_tests.sh

# Run with mock LLM server
python3 test/mock_llm.py 8080 &
./target/debug/slop --api-base http://127.0.0.1:8080/v1 --model mock "your prompt"

# Lint
cargo clippy
```

## Spec

See [SPEC.md](SPEC.md) for the full specification.

## License

MIT
