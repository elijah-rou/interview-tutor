#!/usr/bin/env python3
import json
import os
import select
import sys
import time

agent_dir = os.environ.get("PI_CODING_AGENT_DIR")
mode = "normal"
capture_path = None
if agent_dir and os.path.isfile(os.path.join(agent_dir, "fake-mode")):
    with open(os.path.join(agent_dir, "fake-mode"), encoding="utf-8") as file:
        mode = file.read().strip() or "normal"
    capture_path = os.path.join(agent_dir, "fake-capture.jsonl")


def record(value):
    if capture_path:
        with open(capture_path, "a", encoding="utf-8") as file:
            file.write(json.dumps(value, sort_keys=True) + "\n")


def emit(value):
    delimiter = "\r\n" if mode == "crlf" else "\n"
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + delimiter)
    sys.stdout.flush()


if "--version" in sys.argv:
    record({"kind": "version"})
    print("0.84.2")
    raise SystemExit(0)

expected = [
    "--mode",
    "rpc",
    "--no-session",
    "--no-tools",
    "--no-extensions",
    "--no-skills",
    "--no-prompt-templates",
    "--no-themes",
    "--no-context-files",
    "--no-approve",
    "--offline",
]
if sys.argv[1:] != expected:
    raise SystemExit(2)

record(
    {
        "kind": "process",
        "argv": sys.argv[1:],
        "cwd": os.getcwd(),
        "environment_names": sorted(os.environ),
        "forced_offline": os.environ.get("PI_OFFLINE") == "1"
        and os.environ.get("PI_SKIP_VERSION_CHECK") == "1",
        "forced_telemetry_off": os.environ.get("PI_TELEMETRY") == "0",
        "pid": os.getpid(),
    }
)
with open("fake-pi-artifact", "w", encoding="utf-8") as file:
    file.write("removed with cwd")
if mode == "stderr":
    sys.stderr.write("e" * (2 * 1024 * 1024))
    sys.stderr.flush()

provider = "fixture-provider"
last_text = None
prompt_count = 0


def assistant(text, stop_reason="stop"):
    return {
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "api": "fixture",
        "provider": provider,
        "model": "fixture-model",
        "usage": {
            "input": 1,
            "output": 1,
            "cacheRead": 0,
            "cacheWrite": 0,
            "totalTokens": 2,
            "cost": {
                "input": 0,
                "output": 0,
                "cacheRead": 0,
                "cacheWrite": 0,
                "total": 0,
            },
        },
        "stopReason": stop_reason,
        "timestamp": 1,
    }


def complete(text, stop_reason="stop"):
    global last_text
    last_text = text
    message = assistant(text, stop_reason)
    emit({"type": "agent_start"})
    emit({"type": "turn_start"})
    emit({"type": "message_start", "message": assistant("")})
    emit(
        {
            "type": "message_update",
            "usage": message["usage"],
            "assistantMessageEvent": {
                "type": "text_delta",
                "contentIndex": 0,
                "delta": text,
            },
        }
    )
    emit({"type": "message_end", "message": message})
    emit({"type": "turn_end", "message": message, "toolResults": []})
    emit({"type": "agent_end", "messages": [message], "willRetry": False})
    emit({"type": "agent_settled"})


for raw in sys.stdin:
    command = json.loads(raw)
    command_type = command.get("type")
    command_id = command.get("id")
    record({"kind": "command", "command_type": command_type, "id": command_id})
    if command_type == "get_state":
        if mode == "malformed":
            sys.stdout.write("{not-json}\n")
            sys.stdout.flush()
            continue
        model = None if mode == "auth" else {
            "id": "fixture-model",
            "name": "Fixture Model",
            "api": "fixture",
            "provider": provider,
            "baseUrl": "https://fixture.invalid",
            "reasoning": False,
            "input": ["text"],
            "contextWindow": 1000,
            "maxTokens": 100,
            "cost": {},
        }
        emit(
            {
                "id": "unexpected-id" if mode == "wrong-id" else command_id,
                "type": "response",
                "command": "get_state",
                "success": True,
                "data": {
                    "model": model,
                    "thinkingLevel": "off",
                    "isStreaming": False,
                    "isCompacting": False,
                    "steeringMode": "one-at-a-time",
                    "followUpMode": "one-at-a-time",
                    "sessionFile": None,
                    "sessionId": "fixture",
                    "autoCompactionEnabled": True,
                    "messageCount": 0,
                    "pendingMessageCount": 0,
                },
            }
        )
    elif command_type == "prompt":
        prompt_count += 1
        emit(
            {
                "id": command_id,
                "type": "response",
                "command": "prompt",
                "success": True,
            }
        )
        if mode == "eof":
            raise SystemExit(0)
        if mode == "flood":
            sys.stdout.write(
                "".join(
                    '{"type":"future_flood","padding":"' + ("x" * 1024) + '"}\n'
                    for _ in range(3000)
                )
            )
            sys.stdout.flush()
            time.sleep(5)
            continue
        if mode == "tool":
            emit({"type": "agent_start"})
            emit({"type": "tool_execution_start", "toolCallId": "tool-1"})
            continue
        if mode == "hold":
            emit({"type": "agent_start"})
            emit({"type": "turn_start"})
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                ready, _, _ = select.select([sys.stdin], [], [], 0.05)
                if not ready:
                    continue
                abort = json.loads(sys.stdin.readline())
                record(
                    {
                        "kind": "command",
                        "command_type": abort.get("type"),
                        "id": abort.get("id"),
                    }
                )
                if abort.get("type") != "abort":
                    raise SystemExit(3)
                emit(
                    {
                        "id": abort["id"],
                        "type": "response",
                        "command": "abort",
                        "success": True,
                    }
                )
                emit({"type": "agent_settled"})
                break
            continue
        if mode == "correction" and prompt_count == 1:
            complete("{}")
        elif mode == "correction":
            complete(
                json.dumps(
                    {
                        "kind": "question",
                        "text": "Corrected question",
                        "assessment": "continue",
                    },
                    separators=(",", ":"),
                )
            )
        elif mode == "non-stop":
            complete("not accepted", "length")
        else:
            complete(
                json.dumps(
                    {
                        "kind": "question",
                        "text": "What invariant holds?",
                        "assessment": "continue",
                    },
                    separators=(",", ":"),
                )
            )
    elif command_type == "get_last_assistant_text":
        text = last_text
        if mode == "stale":
            text = "stale text"
        if mode == "null":
            text = None
        emit(
            {
                "id": command_id,
                "type": "response",
                "command": "get_last_assistant_text",
                "success": True,
                "data": {"text": text},
            }
        )
    elif command_type == "abort":
        emit(
            {
                "id": command_id,
                "type": "response",
                "command": "abort",
                "success": True,
            }
        )
        emit({"type": "agent_settled"})
    else:
        emit(
            {
                "id": command_id,
                "type": "response",
                "command": command_type or "parse",
                "success": False,
                "error": "unexpected fixture command",
            }
        )
