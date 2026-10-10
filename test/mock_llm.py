#!/usr/bin/env python3
"""Mock OpenAI-compatible API server for testing slop.

Responds to chat completions with tool calls based on simple pattern matching
against the user prompt. Logs all requests to stderr for debugging.
"""
import json
import sys
from http.server import HTTPServer, BaseHTTPRequestHandler

# Track conversation state across requests (per-connection would be better,
# but this simple global works for testing)
call_count = 0

class MockHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # Suppress default logging

    def do_POST(self):
        global call_count
        call_count += 1

        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length)
        request = json.loads(body)

        # Log the request
        print(f"\n=== Request #{call_count} ===", file=sys.stderr)
        for msg in request.get('messages', []):
            role = msg.get('role', '?')
            content = msg.get('content', '') or ''
            if len(content) > 200:
                content = content[:200] + '...'
            print(f"  [{role}] {content}", file=sys.stderr)
            if msg.get('tool_calls'):
                for tc in msg['tool_calls']:
                    print(f"    -> tool: {tc['function']['name']}({tc['function']['arguments']})", file=sys.stderr)
            if role == 'tool':
                print(f"    <- result: {content[:200]}", file=sys.stderr)

        # Extract the user prompt
        user_prompt = ""
        for msg in request.get('messages', []):
            if msg.get('role') == 'user':
                user_prompt = msg.get('content', '')
                break

        # Count how many tool results we've received (to know where we are in the flow)
        tool_results = [m for m in request.get('messages', []) if m.get('role') == 'tool']
        num_tool_results = len(tool_results)

        # Decide what to respond with based on the prompt and conversation state
        response = self.decide_response(user_prompt, num_tool_results, request)

        print(f"  Response: {json.dumps(response)[:300]}", file=sys.stderr)

        # Send response
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(response).encode())

    def decide_response(self, prompt, num_tool_results, request):
        """Decide what response to give based on the prompt and state."""
        prompt_lower = prompt.lower()

        # Simple text generation (no tools)
        if 'generate numbers between 1 and 5' in prompt_lower:
            return self.text_response("1\n2\n3\n4\n5")

        # Fibonacci — use a tool (computation should use run_command)
        if 'fib' in prompt_lower:
            if num_tool_results == 0:
                # Use semicolons for a one-liner, or use $'...' for newlines
                return self.tool_call_response(
                    "run_command",
                    {
                        "command": "python3 -c \"a,b=0,1\nwhile a<10000: print(a); a,b=b,a+b\"",
                        "stdin_source": "none",
                        "stdout_dest": "passthrough"
                    },
                    text="I'll compute the Fibonacci numbers using Python."
                )
            else:
                return self.text_response("")

        # Sort numerically
        if 'sort' in prompt_lower and 'numerically' in prompt_lower:
            if num_tool_results == 0:
                # First, peek at the input
                return self.tool_call_response("peek_input", {"lines": 20})
            elif num_tool_results == 1:
                # Run sort command
                return self.tool_call_response("run_command", {
                    "command": "sort -n",
                    "stdout_dest": "passthrough"
                })
            else:
                return self.text_response("")

        # Emit x's for numbers
        if 'emit one x' in prompt_lower or 'one x for' in prompt_lower:
            if num_tool_results == 0:
                return self.tool_call_response("peek_input", {"lines": 20})
            elif num_tool_results == 1:
                return self.tool_call_response("run_command", {
                    "command": "python3 -c \"import sys; [print('x' * int(l.strip())) for l in sys.stdin if l.strip()]\"",
                    "stdout_dest": "passthrough"
                })
            else:
                # Command already ran with passthrough, we're done
                return self.text_response("")

        # Filter out 404s
        if 'filter out 404' in prompt_lower or 'filter 404' in prompt_lower:
            if num_tool_results == 0:
                return self.tool_call_response("peek_input", {"lines": 10})
            elif num_tool_results == 1:
                return self.tool_call_response("run_command", {
                    "command": "grep -v ' 404 '",
                    "stdout_dest": "passthrough"
                })
            else:
                return self.text_response("")

        # Most popular pages
        if 'most popular' in prompt_lower or 'popular pages' in prompt_lower:
            if num_tool_results == 0:
                return self.tool_call_response("peek_input", {"lines": 10})
            elif num_tool_results == 1:
                # Run analysis command with capture
                return self.tool_call_response("run_command", {
                    "command": "awk '{print $3}' | sort | uniq -c | sort -rn | head -10",
                    "stdout_dest": "capture"
                })
            else:
                # We have the analysis results, write a summary
                last_tool_result = ""
                for msg in reversed(request.get('messages', [])):
                    if msg.get('role') == 'tool':
                        try:
                            result = json.loads(msg.get('content', '{}'))
                            last_tool_result = result.get('stdout', '')
                        except:
                            last_tool_result = msg.get('content', '')
                        break
                return self.text_response(f"Top pages by hits:\n{last_tool_result}")

        # Summarize
        if 'summarize' in prompt_lower or 'summary' in prompt_lower:
            if num_tool_results == 0:
                return self.tool_call_response("peek_input", {"lines": 20})
            else:
                return self.text_response("This is a summary of the input data.")

        # Default: just respond with text
        return self.text_response(f"I received your prompt: {prompt}")

    def text_response(self, content):
        """Create a simple text response (no tool calls)."""
        return {
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "created": 0,
            "model": "mock",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
        }

    def tool_call_response(self, tool_name, args, text=None):
        """Create a response with a tool call."""
        msg = {
            "role": "assistant",
            "content": text,
            "tool_calls": [{
                "id": f"call_{tool_name}",
                "type": "function",
                "function": {
                    "name": tool_name,
                    "arguments": json.dumps(args),
                },
            }],
        }
        return {
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "created": 0,
            "model": "mock",
            "choices": [{
                "index": 0,
                "message": msg,
                "finish_reason": "tool_calls",
            }],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
        }


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
    server = HTTPServer(('127.0.0.1', port), MockHandler)
    print(f"Mock LLM server listening on http://127.0.0.1:{port}/v1", file=sys.stderr)
    print(f"Use: slop --api-base http://127.0.0.1:{port}/v1 --model mock 'your prompt'", file=sys.stderr)
    server.serve_forever()


if __name__ == '__main__':
    main()
