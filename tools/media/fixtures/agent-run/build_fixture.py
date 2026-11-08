#!/usr/bin/env python3
"""Turn one `claude -p --output-format stream-json` run into the tutorial's fixture files.

Usage: build_fixture.py <stream.jsonl> <run folder> <out dir>
Writes transcript.txt and actions.jsonl and prints how many strings it replaced. It keeps the
agent's text and every tool call in order. It replaces only the run's paths and shortens the
drumspike tool's name; HOW.md states the counts it prints.
"""
import json
import sys
import textwrap
from pathlib import Path

WIDTH = 96
TOOL_PREFIX = "mcp__drumspike__"


def main(stream, run, out):
    run = str(Path(run))
    home = str(Path(run) / "home")
    counts = {"paths": 0, "tool names": 0}

    def clean(text):
        counts["paths"] += text.count(home) + text.count(run)
        return text.replace(home, "<home>").replace(run, "<run folder>")

    lines, actions, pending, sample = [], [], {}, {}
    for raw in Path(stream).read_text().splitlines():
        row = json.loads(raw)
        if not isinstance(row, dict) or "message" not in row:
            continue
        content = row["message"]["content"]
        if isinstance(content, str):
            continue
        for block in content:
            kind = block.get("type")
            if kind == "text" and row["type"] == "assistant":
                for para in clean(block["text"]).splitlines():
                    if para.startswith("|"):
                        lines.append("  " + para)
                    else:
                        wrapped = textwrap.wrap(para, WIDTH - 2) or [""]
                        lines.extend("  " + w for w in wrapped)
            elif kind == "tool_use":
                name, args = block["name"], block["input"]
                if name.startswith(TOOL_PREFIX):
                    name = name[len(TOOL_PREFIX):]
                    counts["tool names"] += 1
                if name == "agent_call":
                    shown = " ".join([args["fn"], *(a if isinstance(a, str) else json.dumps(a) for a in args.get("args", []))])
                elif name == "Skill":
                    shown = args["skill"]
                elif name == "Write":
                    shown = clean(args["file_path"])
                elif name == "ToolSearch":
                    shown = args["query"]
                else:
                    shown = clean(json.dumps(args))[:80]
                lines.append(f"> {name} {shown}".rstrip())
                pending[block["id"]] = (len(lines) - 1, args if name == "agent_call" else None)
            elif kind == "tool_result" and block["tool_use_id"] in pending:
                at, call = pending.pop(block["tool_use_id"])
                if call is None:
                    continue
                body = block["content"]
                if isinstance(body, list):
                    body = "".join(part.get("text", "") for part in body)
                result = json.loads(body)
                fn, a = call["fn"], call.get("args", [])
                if fn == "hits":
                    sample.update({(a[0], h["index"]): h["sample"] for h in result})
                elif fn == "move":
                    assert result["moved"], result
                    track, index = a[0], a[1]
                    actions.append({"fn": "move", "track": track, "index": index,
                                    "from": result["from"], "to": result["to"]})
                    sample[(track, index)] = result["to"]
                    lines[at] += f"   moved {result['from']} to {result['to']}"
                elif fn == "confirm":
                    assert result["confirmed"], result
                    track, index = a[0], a[1]
                    at_sample = sample[(track, index)]
                    actions.append({"fn": "confirm", "track": track, "index": index,
                                    "from": at_sample, "to": at_sample})

    out = Path(out)
    (out / "transcript.txt").write_text("\n".join(lines) + "\n")
    (out / "actions.jsonl").write_text("".join(json.dumps(a) + "\n" for a in actions))
    print(counts, "lines", len(lines), "actions", len(actions))


if __name__ == "__main__":
    main(*sys.argv[1:])
