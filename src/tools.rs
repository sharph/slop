use serde_json::{json, Value};

/// Tool definitions in OpenAI function-calling format.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "peek_input",
                "description": "Read the first N bytes or lines from stdin. The peeked data is buffered and will be replayed to any subsequent command that reads from stdin (stdin_source=\"inherit\"). Use this to understand the shape of the input before deciding what to do.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "bytes": {
                            "type": "integer",
                            "description": "Max bytes to read (default: 4096)"
                        },
                        "lines": {
                            "type": "integer",
                            "description": "Max lines to read. Takes precedence over bytes if both are given."
                        }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Execute a shell command (via sh -c). You control where its stdin comes from and where its stdout goes. Use this to process data streams, run computations, or transform input.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute"
                        },
                        "stdin_source": {
                            "type": "string",
                            "enum": ["inherit", "none", "string"],
                            "description": "Where the command reads stdin from. 'inherit' (default) = original stdin with peeked data replayed. 'none' = /dev/null. 'string' = use stdin_text parameter."
                        },
                        "stdin_text": {
                            "type": "string",
                            "description": "Text to use as stdin when stdin_source='string'"
                        },
                        "stdout_dest": {
                            "type": "string",
                            "enum": ["passthrough", "capture", "discard"],
                            "description": "Where the command's stdout goes. 'passthrough' (default) = directly to slop's stdout (use for large streams). TERMINAL: slop exits immediately after this command finishes. 'capture' = returned to you (use to analyze output). 'discard' = /dev/null."
                        },
                        "capture_limit": {
                            "type": "integer",
                            "description": "Override the default stdout capture limit (bytes). Only used when stdout_dest='capture'. Clamped to the configured maximum (default max: 65536)."
                        },
                        "stderr_capture_limit": {
                            "type": "integer",
                            "description": "Override the default stderr capture limit (bytes). Clamped to the configured maximum (default max: 65536)."
                        }
                    },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_to_stdout",
                "description": "Write text to slop's stdout. Can be called multiple times; text is appended in order. Use this to produce your final output. Set close=true on the final call to close the stream — TERMINAL: slop exits immediately (sends EOF to downstream commands).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Text to write to stdout"
                        },
                        "close": {
                            "type": "boolean",
                            "description": "If true, close stdout after writing. This sends EOF to any downstream command in a pipeline. Use this on your final write_to_stdout call. Default: false."
                        }
                    },
                    "required": ["text"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_to_stderr",
                "description": "Write text to slop's stderr. Visible to the user but does not pollute the stdout data stream. Use for progress messages, explanations, or commentary.",
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
        }),
    ]
}
