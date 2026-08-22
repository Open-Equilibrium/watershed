from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


SCHEMA: dict[str, Any] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "findings",
        "overall_correctness",
        "overall_explanation",
        "overall_confidence",
    ],
    "properties": {
        "findings": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "title",
                    "body",
                    "priority",
                    "confidence",
                    "category",
                    "code_location",
                ],
                "properties": {
                    "title": {"type": "string", "minLength": 1, "maxLength": 140},
                    "body": {"type": "string", "minLength": 1, "maxLength": 2000},
                    "priority": {"type": "string", "enum": ["P0", "P1", "P2", "P3"]},
                    "confidence": {"type": "number", "minimum": 0, "maximum": 1},
                    "category": {
                        "type": "string",
                        "enum": ["bug", "security", "regression", "test_gap", "maintainability"],
                    },
                    "code_location": {
                        "type": "object",
                        "additionalProperties": False,
                        "required": ["file_path", "line"],
                        "properties": {
                            "file_path": {"type": "string", "minLength": 1},
                            "line": {"type": "integer", "minimum": 1},
                        },
                    },
                },
            },
        },
        "overall_correctness": {
            "type": "string",
            "enum": ["patch is correct", "patch is incorrect"],
        },
        "overall_explanation": {"type": "string", "minLength": 1, "maxLength": 3000},
        "overall_confidence": {"type": "number", "minimum": 0, "maximum": 1},
    },
}

def bounded_field(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    suffix = "\n\n[truncated]"
    return text[: max(0, limit - len(suffix))] + suffix

def extract_json(text: str) -> dict[str, Any]:
    stripped = text.strip()
    if not stripped:
        raise SystemExit("review engine returned empty output")
    try:
        parsed = json.loads(stripped)
    except json.JSONDecodeError as exc:
        fenced_report = parse_json_candidate(stripped)
        if isinstance(fenced_report, dict) and "findings" in fenced_report:
            return fenced_report
        jsonl_report = extract_json_from_jsonl(stripped)
        if jsonl_report:
            return jsonl_report
        raise SystemExit(f"review engine returned non-JSON output: {exc}\n{stripped[:2000]}")
    if isinstance(parsed, dict) and "findings" in parsed:
        return parsed
    if isinstance(parsed, dict) and isinstance(parsed.get("structured_output"), dict):
        return parsed["structured_output"]
    if isinstance(parsed, dict) and isinstance(parsed.get("result"), str):
        result_json = parse_json_candidate(parsed["result"])
        if isinstance(result_json, dict) and "findings" in result_json:
            return result_json
        raise SystemExit(f"review engine result was not structured JSON:\n{parsed['result'][:2000]}")
    jsonl_report = extract_json_from_jsonl(stripped)
    if jsonl_report:
        return jsonl_report
    raise SystemExit(f"review engine returned unexpected JSON shape:\n{json.dumps(parsed)[:2000]}")


def extract_json_from_jsonl(text: str) -> dict[str, Any] | None:
    candidates: list[str | dict[str, Any]] = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict):
            continue
        part = event.get("part")
        if isinstance(part, dict) and isinstance(part.get("text"), str):
            candidates.append(part["text"])
        data = event.get("data")
        if isinstance(data, dict) and isinstance(data.get("content"), str):
            candidates.append(data["content"])
        if isinstance(event.get("result"), str):
            candidates.append(event["result"])
        if isinstance(event.get("structured_output"), dict):
            candidates.append(event["structured_output"])
    for candidate in reversed(candidates):
        if isinstance(candidate, dict):
            if "findings" in candidate:
                return candidate
            continue
        parsed = parse_json_candidate(candidate)
        if isinstance(parsed, dict) and "findings" in parsed:
            return parsed
    return None


def parse_json_candidate(text: str) -> Any | None:
    stripped = text.strip()
    if stripped.startswith("```"):
        lines = stripped.splitlines()
        if lines and lines[0].startswith("```") and lines[-1].strip() == "```":
            stripped = "\n".join(lines[1:-1]).strip()
    try:
        parsed = json.loads(stripped)
    except json.JSONDecodeError:
        return None
    if isinstance(parsed, str) and parsed != text:
        nested = parse_json_candidate(parsed)
        return nested if nested is not None else parsed
    return parsed


def normalize_review_path(path: str) -> str:
    normalized = path.strip().replace("\\", "/")
    while True:
        previous = normalized
        while normalized.startswith("./"):
            normalized = normalized[2:]
        if normalized.startswith(("a/", "b/")):
            normalized = normalized[2:]
        if normalized == previous:
            return normalized


def validate_report(report: dict[str, Any], repo: Path, changed_paths: set[str], required: list[str]) -> None:
    normalized_changed_paths = {normalize_review_path(path) for path in changed_paths}
    allowed_top = {"findings", "overall_correctness", "overall_explanation", "overall_confidence"}
    extra_top = set(report) - allowed_top
    if extra_top:
        raise SystemExit(f"review JSON has unexpected top-level keys: {sorted(extra_top)}")
    for key in SCHEMA["required"]:
        if key not in report:
            raise SystemExit(f"review JSON missing required key: {key}")
    if not isinstance(report["findings"], list):
        raise SystemExit("review JSON findings must be an array")
    if report.get("overall_correctness") not in {"patch is correct", "patch is incorrect"}:
        raise SystemExit(f"review JSON has invalid overall_correctness: {report.get('overall_correctness')}")
    if not isinstance(report.get("overall_explanation"), str) or not report["overall_explanation"]:
        raise SystemExit("review JSON overall_explanation must be a non-empty string")
    if len(report["overall_explanation"]) > 3000:
        raise SystemExit("review JSON overall_explanation is too long")
    if not number_in_range(report.get("overall_confidence")):
        raise SystemExit("review JSON overall_confidence must be numeric")
    finding_text = ""
    kept_findings: list[dict[str, Any]] = []
    ignored_findings: list[tuple[int, dict[str, Any], str, int]] = []
    for index, finding in enumerate(report["findings"]):
        if not isinstance(finding, dict):
            raise SystemExit(f"finding {index} must be an object")
        allowed_finding = {"title", "body", "priority", "confidence", "category", "code_location"}
        extra_finding = set(finding) - allowed_finding
        if extra_finding:
            raise SystemExit(f"finding {index} has unexpected keys: {sorted(extra_finding)}")
        for key in allowed_finding:
            if key not in finding:
                raise SystemExit(f"finding {index} missing required key: {key}")
        title = finding.get("title")
        if not isinstance(title, str) or not title or len(title) > 140:
            raise SystemExit(f"finding {index} has invalid title")
        body = finding.get("body")
        if not isinstance(body, str) or not body or len(body) > 2000:
            raise SystemExit(f"finding {index} has invalid body")
        priority = finding.get("priority")
        if priority not in {"P0", "P1", "P2", "P3"}:
            raise SystemExit(f"finding {index} has invalid priority: {priority}")
        if not number_in_range(finding.get("confidence")):
            raise SystemExit(f"finding {index} has invalid confidence")
        category = finding.get("category")
        if category not in {"bug", "security", "regression", "test_gap", "maintainability"}:
            raise SystemExit(f"finding {index} has invalid category: {category}")
        location = finding.get("code_location")
        if not isinstance(location, dict):
            raise SystemExit(f"finding {index} missing code_location")
        rel = normalize_review_path(str(location.get("file_path", "")).strip())
        line = location.get("line")
        if not rel or not isinstance(line, int) or line < 1:
            raise SystemExit(f"finding {index} has invalid location: {location}")
        if Path(rel).is_absolute() or ".." in Path(rel).parts:
            raise SystemExit(f"finding {index} uses invalid file path: {rel}")
        location["file_path"] = rel
        if rel not in normalized_changed_paths:
            ignored_findings.append((index, finding, rel, line))
            continue
        kept_findings.append(finding)
        finding_text += "\n" + json.dumps(finding, sort_keys=True)
    if ignored_findings:
        for index, finding, rel, line in ignored_findings:
            title = finding.get("title", "<untitled>")
            print(
                f"autoreview ignored out-of-scope finding {index}: {title} ({rel}:{line})",
                file=sys.stderr,
            )
            print(bounded_field(str(finding.get("body", "")), 500), file=sys.stderr)
        report["findings"] = kept_findings
        if not kept_findings and report["overall_correctness"] == "patch is incorrect":
            note = f"Ignored {len(ignored_findings)} out-of-scope finding(s) outside the reviewed change."
            explanation = report["overall_explanation"].rstrip()
            report["overall_correctness"] = "patch is correct"
            report["overall_explanation"] = bounded_field(f"{explanation}\n\n{note}", 3000)
    haystack = finding_text.lower()
    for needle in required:
        if needle.lower() not in haystack:
            raise SystemExit(f"required finding text not found: {needle}")


def number_in_range(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and 0 <= value <= 1


def print_report(report: dict[str, Any], *, label: str = "autoreview") -> None:
    findings = report["findings"]
    if findings:
        print(f"{label} findings: {len(findings)}")
    elif report["overall_correctness"] == "patch is incorrect":
        print(f"{label} verdict: patch is incorrect without discrete findings")
    else:
        print(f"{label} clean: no accepted/actionable findings reported")
    for finding in findings:
        loc = finding["code_location"]
        print(f"[{finding['priority']}] {finding['title']}")
        print(f"{loc['file_path']}:{loc['line']}")
        print(f"{finding['body']}")
        print()
    print(f"overall: {report['overall_correctness']} ({report['overall_confidence']})")
    print(report["overall_explanation"])
