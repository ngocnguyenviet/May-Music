#!/usr/bin/env python3
"""Offline protocol fixture. Never used by the normal application."""
import json
import os
import sys
import time
from pathlib import Path

args = sys.argv[1:]
if "--version" in args:
    print("fixture")
    sys.exit()
root = Path(os.environ["MAY_TEST_DIR"])
query = args[-1]
with (root / "youtube.log").open("a") as log:
    log.write(json.dumps(args) + "\n")
if "slow" in query:
    time.sleep(1.5)
if "--format" in args:
    if "badbadbad00" in query:
        print("Video unavailable", file=sys.stderr)
        sys.exit(1)
    print(json.dumps({"url": "https://example.invalid/audio?" + query.split("?", 1)[-1], "http_headers": {"User-Agent": "Mây fixture", "Accept": "audio/webm,audio/*"}}))
else:
    if query.startswith("https://www.youtube.com/watch?v=") and "--no-playlist" in args:
        video_id = query.split("v=", 1)[1]
        print(json.dumps({"id": video_id, "title": "Bài từ link YouTube", "channel": "Mây Sessions", "duration": 180}))
        sys.exit()
    ids = ["radio000001", "radio000002"] if "list=RD" in query else ["music000001", "music000002"]
    if "--empty" in query or (root / "no-radio").exists():
        ids = []
    names = {"music000001": "Nắng bên thềm", "music000002": "Một chiều dịu êm", "radio000001": "Gió qua ô cửa", "radio000002": "Mây trôi thật chậm"}
    print(json.dumps({"entries": [{"id": i, "title": names[i], "channel": "Mây Sessions", "duration": 180} for i in ids]}))
