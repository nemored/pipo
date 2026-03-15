#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path


def fail(msg: str) -> None:
    raise SystemExit(msg)


matrix_path = Path(__file__).with_name("compatibility_matrix.conf")
if not matrix_path.exists():
    fail(f"missing compatibility matrix: {matrix_path}")

with matrix_path.open("r", encoding="utf-8") as fh:
    payload = json.load(fh)

targets = payload.get("targets")
if not isinstance(targets, list) or not targets:
    fail("compatibility matrix must define a non-empty 'targets' list")

index = {}
for target in targets:
    if not isinstance(target, dict):
        fail("each target entry must be an object")
    triple = target.get("target_triple")
    if not isinstance(triple, str) or not triple.strip():
        fail("each target requires non-empty 'target_triple'")
    triple = triple.strip()
    if triple in index:
        fail(f"duplicate target_triple in compatibility matrix: {triple}")

    timeout = target.get("smoke_ready_timeout_seconds", 15)
    if not isinstance(timeout, int) or timeout < 1:
        fail(f"invalid smoke_ready_timeout_seconds for {triple}: {timeout}")

    known_issues = target.get("known_issues", [])
    if not isinstance(known_issues, list):
        fail(f"known_issues for {triple} must be a list")

    normalized_issues = []
    for issue in known_issues:
        if not isinstance(issue, dict):
            fail(f"known issue for {triple} must be an object")
        issue_id = issue.get("id")
        summary = issue.get("summary")
        mitigation = issue.get("mitigation")
        if not all(isinstance(v, str) and v.strip() for v in (issue_id, summary, mitigation)):
            fail(f"known issues for {triple} must include non-empty id, summary, and mitigation")
        normalized_issues.append({
            "id": issue_id.strip(),
            "summary": summary.strip(),
            "release_blocking": bool(issue.get("release_blocking", False)),
            "mitigation": mitigation.strip(),
        })

    index[triple] = {
        "target_triple": triple,
        "smoke_ready_timeout_seconds": timeout,
        "known_issues": normalized_issues,
    }

raw_supported = os.environ.get("SUPPORTED_TARGET_TRIPLES", "")
selected = [item.strip() for item in raw_supported.split(",") if item.strip()]
if not selected:
    fail("SUPPORTED_TARGET_TRIPLES must include at least one target")

resolved = []
for triple in selected:
    if triple not in index:
        fail(f"SUPPORTED_TARGET_TRIPLES includes {triple}, but no matrix entry exists")
    resolved.append(index[triple])

missing = sorted(set(index.keys()) - set(selected))
if missing:
    print(
        f"warning: matrix contains unmanaged targets not enabled in SUPPORTED_TARGET_TRIPLES: {', '.join(missing)}",
        file=sys.stderr,
    )

matrix = {"include": resolved}

if out_path := os.environ.get("GITHUB_OUTPUT"):
    with open(out_path, "a", encoding="utf-8") as out:
        print(f"target_matrix={json.dumps(matrix)}", file=out)
else:
    print(json.dumps(matrix))
