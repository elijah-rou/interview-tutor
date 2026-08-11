#!/usr/bin/env python3
import json
import os
import sys
import time

if "--version" in sys.argv:
    print("codex-cli 0.146.0")
    raise SystemExit(0)
if len(sys.argv) != 3 or sys.argv[1:] != ["app-server", "--stdio"]:
    raise SystemExit(2)

codex_home = os.environ.get("CODEX_HOME")
mode = "normal"
capture_path = None
if codex_home and os.path.isfile(os.path.join(codex_home, "fake-mode")):
    with open(os.path.join(codex_home, "fake-mode"), encoding="utf-8") as file:
        mode = file.read().strip() or "normal"
    capture_path = os.path.join(codex_home, "fake-capture.jsonl")


def record(value):
    if capture_path:
        with open(capture_path, "a", encoding="utf-8") as file:
            file.write(json.dumps(value, sort_keys=True) + "\n")


record({"kind": "process", "argv": sys.argv[1:], "cwd": os.getcwd(), "environment_names": sorted(os.environ)})
with open("fake-session-artifact", "w", encoding="utf-8") as file:
    file.write("removed with process cwd")
if mode == "stderr-flood":
    sys.stderr.write("e" * (2 * 1024 * 1024))
    sys.stderr.flush()

start_count = 1
if codex_home:
    count_path = os.path.join(codex_home, "fake-start-count")
    try:
        with open(count_path, encoding="utf-8") as file:
            start_count = int(file.read()) + 1
    except FileNotFoundError:
        pass
    with open(count_path, "w", encoding="utf-8") as file:
        file.write(str(start_count))

thread_number = 0
for raw in sys.stdin:
    message = json.loads(raw)
    record({"kind": "message", "json": message})
    if "id" not in message or "method" not in message:
        continue
    request_id = message["id"]
    method = message["method"]
    if method == "initialize":
        if mode == "queue-flood":
            sys.stdout.write("".join('{"method":"future/flood","params":{}}\n' for _ in range(10000)))
            sys.stdout.flush()
        result = {"userAgent": "fake"}
    elif method == "account/read":
        result = {"account": {"type": "chatgpt", "email": None, "planType": "unknown"}, "requiresOpenaiAuth": True}
    elif method == "thread/start":
        thread_number += 1
        cwd = "/unexpected-cwd" if mode == "bad-cwd" else os.getcwd()
        result = {
            "thread": {"id": f"thread-{thread_number}", "cwd": cwd, "ephemeral": True, "path": None},
            "cwd": cwd,
            "approvalPolicy": "never",
            "sandbox": {"type": "readOnly", "networkAccess": False},
        }
    elif method == "turn/start":
        if mode == "child-death" or (mode == "restart" and start_count == 1):
            os._exit(17)
        if mode == "eof":
            sys.stdout.write('{"method":"partial"')
            sys.stdout.flush()
            raise SystemExit(0)
        if mode == "malformed":
            print("not-json", flush=True)
            continue
        if mode == "oversize":
            print("x" * (2 * 1024 * 1024 + 1), flush=True)
            continue
        result = {"turn": {"id": "turn-1"}}
        print(json.dumps({"id": request_id, "result": result}), flush=True)
        params = message["params"]
        thread_id = params["threadId"]
        if mode in {"timeout-ack", "interrupt-no-ack"}:
            continue
        request_methods = {
            "approval-command": "item/commandExecution/requestApproval",
            "approval-file": "item/fileChange/requestApproval",
            "approval-permissions": "item/permissions/requestApproval",
            "approval-user-input": "item/tool/requestUserInput",
            "approval-mcp": "mcpServer/elicitation/request",
            "approval-unknown": "future/serverRequest",
        }
        if mode in request_methods:
            print(json.dumps({"id": "server-1", "method": request_methods[mode], "params": {"threadId": thread_id, "turnId": "turn-1", "itemId": "item-1"}}), flush=True)
            continue
        if mode == "terminal-error":
            print(json.dumps({"method": "error", "params": {"threadId": thread_id, "turnId": "turn-1", "willRetry": False, "error": {"message": "terminal"}}}), flush=True)
            continue
        text = '{"kind":"question","text":"What invariant holds?","assessment":"continue"}'
        print(json.dumps({"method": "error", "params": {"threadId": "unrelated", "turnId": "unrelated", "willRetry": False, "error": {"message": "ignore"}}}), flush=True)
        print(json.dumps({"method": "error", "params": {"threadId": thread_id, "turnId": "turn-1", "willRetry": True, "error": {"message": "retry"}}}), flush=True)
        print(json.dumps({"method": "future/notification", "params": {}}), flush=True)
        print(json.dumps({"method": "item/started", "params": {"threadId": thread_id, "turnId": "turn-1", "item": {"id": "item-1", "type": "agentMessage", "text": ""}, "startedAtMs": 1}}), flush=True)
        print(json.dumps({"method": "item/agentMessage/delta", "params": {"threadId": thread_id, "turnId": "turn-1", "itemId": "item-1", "delta": text}}), flush=True)
        print(json.dumps({"method": "item/completed", "params": {"threadId": thread_id, "turnId": "turn-1", "item": {"id": "item-1", "type": "agentMessage", "text": text}, "completedAtMs": 2}}), flush=True)
        print(json.dumps({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": "turn-1", "status": "completed", "items": []}}}), flush=True)
        continue
    elif method == "turn/interrupt":
        if mode == "interrupt-no-ack":
            time.sleep(60)
            continue
        result = {}
        print(json.dumps({"id": request_id, "result": result}), flush=True)
        print(json.dumps({"method": "turn/completed", "params": {"threadId": message["params"]["threadId"], "turn": {"id": message["params"]["turnId"], "status": "interrupted", "items": []}}}), flush=True)
        continue
    else:
        print(json.dumps({"id": request_id, "error": {"code": -32601, "message": "unknown"}}), flush=True)
        continue
    print(json.dumps({"id": request_id, "result": result}), flush=True)
