"""Bounded localhost provider for the opt-in installed Codex projection test.

The parent sends only a synthetic-key launch specification over stdin. No config,
credentials, or environment belonging to another agent is read. Output is counts.
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

KEY = "synthetic-codex-product-test-key"
requests = []


class Recorder(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        body = b'{"data":[{"id":"connection-test-model"}]}'
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        size = int(self.headers.get("Content-Length", 0))
        if size > 2 * 1024 * 1024 or len(requests) >= 2:
            self.send_error(400, "Bounded recorder exhausted")
            return
        payload = json.loads(self.rfile.read(size))
        results = [x for x in payload.get("input", []) if x.get("type") == "function_call_output"]
        rendered = json.dumps(payload)
        requests.append({
            "model": payload.get("model"),
            "key_in_result": sum(json.dumps(x).count(KEY) for x in results),
            "exec_succeeded": any("exited with code 0" in json.dumps(x) and "PATH=" in json.dumps(x) for x in results),
            "global_skill_markers": rendered.count("SYNTHETIC_GLOBAL_SKILL_MARKER"),
            "auth_matches": self.headers.get("Authorization") == "Bearer " + KEY,
        })
        item = ({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "exec_command", "arguments": json.dumps({"cmd": "/usr/bin/env", "login": False, "max_output_tokens": 4096})}
                if not results else {"type": "message", "id": "msg_1", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "CONTROL_OK", "annotations": []}]})
        response = {"id": "resp_1", "object": "response", "status": "completed", "model": payload["model"], "created_at": 0, "output": [item], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}
        events = [{"type": "response.created", "response": {**response, "status": "in_progress", "output": []}}, {"type": "response.output_item.added", "output_index": 0, "item": {**item, "arguments": ""} if not results else item}]
        if not results:
            events.append({"type": "response.function_call_arguments.delta", "item_id": item["id"], "output_index": 0, "delta": item["arguments"]})
        events += [{"type": "response.output_item.done", "output_index": 0, "item": item}, {"type": "response.completed", "response": response}]
        body = "".join("event: " + event["type"] + "\ndata: " + json.dumps(event) + "\n\n" for event in events).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Recorder)
threading.Thread(target=server.serve_forever, daemon=True).start()
base = f"http://127.0.0.1:{server.server_port}/v1"
print(json.dumps({"base_url": base}), flush=True)
# Bound even a broken parent that never sends the launch specification.
if not select.select([sys.stdin], [], [], 30)[0]:
    raise SystemExit(2)
spec = json.loads(sys.stdin.readline())
env = {key: value for key, value in spec["env"].items() if value is not None}
skill = pathlib.Path(env["HOME"]) / ".agents/skills/control/SKILL.md"
skill.parent.mkdir(parents=True, exist_ok=True)
skill.write_text("---\nname: product-control\ndescription: SYNTHETIC_GLOBAL_SKILL_MARKER\n---\nLocal test fixture.\n")
proc = subprocess.Popen([spec["adapter"]], cwd=spec["cwd"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True)
sequence = 0
buffer = b""
deadline = time.monotonic() + 60


def send(frame):
    proc.stdin.write((json.dumps(frame) + "\n").encode())
    proc.stdin.flush()


def rpc(method, params):
    global sequence, buffer
    sequence += 1
    send({"jsonrpc": "2.0", "id": sequence, "method": method, "params": params})
    while time.monotonic() < deadline:
        if not select.select([proc.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
            break
        data = os.read(proc.stdout.fileno(), 65536)
        if not data:
            break
        buffer += data
        if len(buffer) > 2 * 1024 * 1024:
            raise RuntimeError("bounded ACP buffer exhausted")
        while b"\n" in buffer:
            line, buffer = buffer.split(b"\n", 1)
            try:
                frame = json.loads(line)
            except ValueError:
                continue
            if "method" in frame and "id" in frame:
                if frame["method"] == "session/request_permission":
                    option = next(x for x in frame["params"]["options"] if x["kind"] == "allow_once")
                    send({"jsonrpc": "2.0", "id": frame["id"], "result": {"outcome": {"outcome": "selected", "optionId": option["optionId"]}}})
                else:
                    send({"jsonrpc": "2.0", "id": frame["id"], "result": {}})
            elif frame.get("id") == sequence:
                if "error" in frame:
                    raise RuntimeError("ACP returned an error")
                return frame["result"]
    raise RuntimeError("bounded ACP request failed")


result = {"completed": False}
try:
    rpc("initialize", {"protocolVersion": 1, "clientCapabilities": {}})
    rpc("providers/set", {"providerId": "openai", "apiType": "openai", "baseUrl": base, "headers": {}})
    session = rpc("session/new", {"cwd": spec["cwd"], "mcpServers": []})
    answer = rpc("session/prompt", {"sessionId": session["sessionId"], "prompt": [{"type": "text", "text": "Run the environment control and finish."}]})
    result["completed"] = answer.get("stopReason") == "end_turn"
    result["mode"] = session.get("modes", {}).get("currentModeId")
finally:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.wait(timeout=3)
    server.shutdown()
    home = pathlib.Path(env["CODEX_HOME"])
    result["home_key_occurrences"] = sum(path.read_bytes().count(KEY.encode()) for path in home.rglob("*") if path.is_file() and not path.is_symlink())
    result["requests"] = requests
    print(json.dumps(result), flush=True)
