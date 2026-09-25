"""Bounded localhost Chat Completions control for native connection projections.

The launch spec contains only a synthetic credential. Output is metadata/counts.
"""
import http.server
import json
import os
import pathlib
import select
import signal
import subprocess
import sys
import threading
import time

KEY = "synthetic-chat-product-test-key"
requests = []


class Recorder(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"data":[{"id":"connection-test-model"}]}')

    def do_POST(self):
        size = int(self.headers.get("Content-Length", 0))
        if size > 2 * 1024 * 1024 or len(requests) >= 2:
            self.send_error(400, "Bounded recorder exhausted")
            return
        payload = json.loads(self.rfile.read(size))
        names = [t.get("function", {}).get("name", "") for t in payload.get("tools", [])]
        results = json.dumps([m for m in payload.get("messages", []) if m.get("role") == "tool"])
        requests.append({
            "model": payload.get("model"), "path": self.path, "tools": names,
            "max_tokens": payload.get("max_tokens", payload.get("max_completion_tokens")),
            "key_in_tool_result": results.count(KEY),
            "mcp_env_clean": "SESSION_ENV_CLEAN" in results,
            "auth_matches": self.headers.get("Authorization") == "Bearer " + KEY,
        })
        tool = next((name for name in names if name.endswith("inspect_environment")), None)
        if len(requests) == 1 and tool:
            message = {"role": "assistant", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": tool, "arguments": "{}"}}]}
            finish = "tool_calls"
        else:
            message = {"role": "assistant", "content": "LOCAL_CONTROL_DONE"}
            finish = "stop"
        base = {"id": "chatcmpl-control", "object": "chat.completion", "created": 0, "model": payload.get("model")}
        if payload.get("stream"):
            for index, call in enumerate(message.get("tool_calls", [])):
                call["index"] = index
            body = "".join("data: " + json.dumps({**base, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": delta, "finish_reason": stop}]}) + "\n\n" for delta, stop in [(message, None), ({}, finish)]) + "data: [DONE]\n\n"
            kind = "text/event-stream"
        else:
            body = json.dumps({**base, "choices": [{"index": 0, "message": message, "finish_reason": finish}], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
            kind = "application/json"
        encoded = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Recorder)
threading.Thread(target=server.serve_forever, daemon=True).start()
print(json.dumps({"base_url": f"http://127.0.0.1:{server.server_port}/v1"}), flush=True)
if not select.select([sys.stdin], [], [], 30)[0]:
    raise SystemExit(2)
spec = json.loads(sys.stdin.readline())
env = {key: value for key, value in spec["env"].items() if value is not None}
root = pathlib.Path(spec["cwd"])
mcp = root / "session_mcp.py"
mcp.write_text('''import json,os,sys
for line in sys.stdin:
 q=json.loads(line)
 if 'id' not in q: continue
 method=q['method']
 if method=='initialize': result={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'session-control','version':'1'}}
 elif method=='tools/list': result={'tools':[{'name':'inspect_environment','description':'Check synthetic session environment','inputSchema':{'type':'object','properties':{}}}]}
 else: result={'content':[{'type':'text','text':'SESSION_ENV_DIRTY' if 'synthetic-chat-product-test-key' in os.environ.values() else 'SESSION_ENV_CLEAN'}]}
 print(json.dumps({'jsonrpc':'2.0','id':q['id'],'result':result}),flush=True)
''')
# An account extension must not enter the explicit session tool set.
if spec["runtime"] == "goose":
    config = pathlib.Path(env["GOOSE_PATH_ROOT"]) / "config/config.yaml"
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text("extensions:\n  account_fixture:\n    enabled: true\n    type: stdio\n    name: account_fixture\n    cmd: /usr/bin/false\n    args: []\n")
proc = subprocess.Popen(spec["argv"], cwd=root, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True)
sequence = 0
buffer = b""
deadline = time.monotonic() + 60


def send(frame):
    proc.stdin.write((json.dumps(frame) + "\n").encode())
    proc.stdin.flush()


def rpc(method, params):
    global sequence, buffer
    sequence += 1
    rid = sequence
    send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
    while time.monotonic() < deadline:
        if not select.select([proc.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
            break
        data = os.read(proc.stdout.fileno(), 65536)
        if not data:
            raise RuntimeError("adapter exited")
        buffer += data
        while b"\n" in buffer:
            line, buffer = buffer.split(b"\n", 1)
            try:
                frame = json.loads(line)
            except ValueError:
                continue
            if frame.get("method") == "session/request_permission":
                options = frame["params"]["options"]
                option = next((o for o in options if o.get("kind") == "allow_once"), options[0])
                send({"jsonrpc": "2.0", "id": frame["id"], "result": {"outcome": {"outcome": "selected", "optionId": option["optionId"]}}})
            if frame.get("id") == rid and ("result" in frame or "error" in frame):
                if "error" in frame:
                    raise RuntimeError("ACP error code " + str(frame["error"].get("code")))
                return frame["result"]
    raise RuntimeError("bounded ACP deadline exceeded")


result = {}
try:
    rpc("initialize", {"protocolVersion": 1, "clientCapabilities": {}})
    session = rpc("session/new", {"cwd": str(root), "mcpServers": [{"name": "buzz_session_control", "command": "/usr/bin/python3", "args": [str(mcp)], "env": []}]})
    if spec.get("post_session"):
        update = spec["post_session"]
        update["params"]["sessionId"] = session["sessionId"]
        rpc(update["method"], update["params"])
    response = rpc("session/prompt", {"sessionId": session["sessionId"], "prompt": [{"type": "text", "text": "Run the requested localhost session control."}]})
    result["completed"] = response.get("stopReason") == "end_turn"
finally:
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    proc.wait(timeout=5)
    result["requests"] = requests
    result["home_key_occurrences"] = sum(p.read_bytes().count(KEY.encode()) for p in pathlib.Path(env["HOME"]).rglob("*") if p.is_file())
    print(json.dumps(result), flush=True)
    server.shutdown()
