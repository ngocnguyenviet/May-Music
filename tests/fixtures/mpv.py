#!/usr/bin/env python3
"""Offline mpv IPC fixture, controlled by files in an isolated test directory."""
import json
import os
import select
import socket
import sys
import time
from pathlib import Path

if "--version" in sys.argv:
    print("mpv fixture")
    sys.exit()
root = Path(os.environ["MAY_TEST_DIR"])
path = next(a.split("=", 1)[1] for a in sys.argv if a.startswith("--input-ipc-server="))
with (root / "mpv.log").open("a") as log:
    log.write(json.dumps({"pid": os.getpid(), "socket": path, "args": sys.argv}) + "\n")
server = socket.socket(socket.AF_UNIX)
server.bind(path)
server.listen(1)
conn, _ = server.accept()
buffer = b""
paused = False
loaded = False

def emit(event):
    conn.sendall(json.dumps(event).encode() + b"\n")

try:
    while True:
        ready, _, _ = select.select([conn], [], [], 0.03)
        if ready:
            data = conn.recv(65536)
            if not data:
                break
            buffer += data
            while b"\n" in buffer:
                line, buffer = buffer.split(b"\n", 1)
                request = json.loads(line)
                cmd = request["command"]
                emit({"error": "success"})
                if cmd[0] == "loadfile":
                    with (root / "loads.log").open("a") as log:
                        log.write(json.dumps(cmd) + "\n")
                    if (root / "fail-on-load").exists() or (root / "always-fail-on-load").exists():
                        (root / "fail-on-load").unlink(missing_ok=True)
                        emit({"event": "end-file", "reason": "error", "file_error": "HTTP 403 fixture"})
                        continue
                    loaded = True
                    emit({"event": "file-loaded"})
                    emit({"event": "property-change", "name": "duration", "data": 180.0})
                    if len(cmd) > 4:
                        emit({"event": "property-change", "name": "time-pos", "data": float(cmd[4].get("start", 0))})
                elif cmd[:2] == ["set_property", "pause"]:
                    paused = cmd[2]
                    emit({"event": "property-change", "name": "pause", "data": paused})
                elif cmd[0] == "seek":
                    emit({"event": "property-change", "name": "time-pos", "data": cmd[1]})
        trigger = root / "player-event"
        if loaded and trigger.exists():
            event = trigger.read_text().strip()
            trigger.unlink()
            if event == "eof":
                emit({"event": "property-change", "name": "time-pos", "data": 180.0})
                emit({"event": "end-file", "reason": "eof"})
                loaded = False
            elif event == "premature":
                emit({"event": "property-change", "name": "time-pos", "data": 60.0})
                emit({"event": "end-file", "reason": "eof"})
                loaded = False
            elif event == "buffering":
                emit({"event": "property-change", "name": "paused-for-cache", "data": True})
            elif event == "buffered":
                emit({"event": "property-change", "name": "paused-for-cache", "data": False})
            elif event == "error":
                emit({"event": "end-file", "reason": "error", "file_error": "fixture playback error"})
                loaded = False
except (BrokenPipeError, ConnectionResetError):
    pass
