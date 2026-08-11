#!/usr/bin/env python3
import json
import os
import signal
import sys

if "--version" in sys.argv:
    print("codex-cli 0.146.0")
    raise SystemExit(0)
if len(sys.argv) < 2 or sys.argv[1] != "app-server":
    raise SystemExit(2)

thread_number = 0
for raw in sys.stdin:
    message = json.loads(raw)
    if "id" not in message:
        continue
    request_id = message["id"]
    method = message.get("method")
    if method == "initialize":
        result = {"userAgent": "fake"}
    elif method == "account/read":
        result = {"account": {"type": "chatgpt", "email": None, "planType": "unknown"}, "requiresOpenaiAuth": True}
    elif method == "thread/start":
        thread_number += 1
        result = {"thread": {"id": f"thread-{thread_number}", "ephemeral": True, "path": None}, "approvalPolicy": "never", "sandbox": {"type": "readOnly", "networkAccess": False}}
    elif method == "turn/start":
        result = {"turn": {"id": "turn-1"}}
        print(json.dumps({"id": request_id, "result": result}), flush=True)
        params = message["params"]
        text = '{"kind":"question","text":"What invariant holds?","assessment":"continue"}'
        print(json.dumps({"method": "future/notification", "params": {}}), flush=True)
        print(json.dumps({"method": "item/agentMessage/delta", "params": {"threadId": params["threadId"], "turnId": "turn-1", "itemId": "item-1", "delta": text}}), flush=True)
        print(json.dumps({"method": "turn/completed", "params": {"threadId": params["threadId"], "turn": {"id": "turn-1", "status": "completed"}}}), flush=True)
        continue
    elif method == "turn/interrupt":
        result = {}
    else:
        print(json.dumps({"id": request_id, "error": {"code": -32601, "message": "unknown"}}), flush=True)
        continue
    print(json.dumps({"id": request_id, "result": result}), flush=True)
