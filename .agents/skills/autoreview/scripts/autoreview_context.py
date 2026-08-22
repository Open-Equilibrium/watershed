from __future__ import annotations

import argparse
import json
import os
import stat
import subprocess
import textwrap
from pathlib import Path

from autoreview_engines import find_command, is_within, resolve_command
from autoreview_report import SCHEMA


def run(
    args: list[str],
    cwd: Path,
    *,
    input_text: str | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        args,
        cwd=cwd,
        input=input_text,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if check and result.returncode != 0:
        cmd = " ".join(args)
        raise SystemExit(
            f"command failed ({result.returncode}): {cmd}\n{result.stderr or result.stdout}"
        )
    return result


def git(repo: Path, *args: str, check: bool = True) -> str:
    return run(
        [resolve_command("git", repo), *args], repo, check=check
    ).stdout


def repo_root() -> Path:
    start = Path.cwd().resolve()
    unsafe_root = discover_repo_root(start) or start
    git_bin = find_command("git", unsafe_root)
    if not git_bin:
        raise SystemExit("git executable not found. Install Git or add it to PATH.")
    result = subprocess.run(
        [git_bin, "rev-parse", "--show-toplevel"],
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise SystemExit("autoreview must run inside a git repository")
    return Path(result.stdout.strip()).resolve()


def discover_repo_root(start: Path) -> Path | None:
    current = start
    while True:
        if (current / ".git").exists():
            return current
        if current.parent == current:
            return None
        current = current.parent


def current_branch(repo: Path) -> str:
    return git(repo, "branch", "--show-current", check=False).strip() or "detached"


def is_dirty(repo: Path) -> bool:
    return bool(git(repo, "status", "--porcelain").strip())


def choose_target(
    repo: Path, mode: str, base_ref: str | None
) -> tuple[str, str | None]:
    mode = "local" if mode == "uncommitted" else mode
    branch = current_branch(repo)
    if mode == "local" or (mode == "auto" and is_dirty(repo)):
        return "local", None
    if mode == "commit":
        return "commit", None
    if mode == "branch" or (mode == "auto" and branch != "main"):
        return "branch", base_ref or detect_pr_base(repo) or "origin/main"
    raise SystemExit("no review target: clean main checkout and no forced mode")


def detect_pr_base(repo: Path) -> str | None:
    gh_bin = find_command("gh", repo)
    if not gh_bin:
        return None
    result = run(
        [
            gh_bin,
            "pr",
            "view",
            "--json",
            "baseRefName",
            "--jq",
            ".baseRefName",
        ],
        repo,
        check=False,
    )
    base = result.stdout.strip()
    return f"origin/{base}" if result.returncode == 0 and base else None


def bounded(text: str, limit: int = 180_000) -> str:
    if len(text) <= limit:
        return text
    return text[:limit] + f"\n\n[truncated at {limit} characters]\n"


def read_text(path: Path, limit: int = 40_000) -> str:
    try:
        data = path.read_bytes()
    except OSError as exc:
        return f"[unreadable: {exc}]"
    if b"\0" in data:
        return "[binary file omitted]"
    text = data.decode("utf-8", errors="replace")
    return bounded(text, limit)


def read_untracked_text(repo: Path, path: Path, limit: int = 40_000) -> str:
    try:
        before = path.lstat()
        if not stat.S_ISREG(before.st_mode) or (
            getattr(before, "st_file_attributes", 0)
            & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
        ):
            return "[non-regular file omitted]"
        if before.st_nlink != 1:
            return "[hard-linked file omitted]"
        if not is_within(path.resolve(strict=True), repo.resolve(strict=True)):
            return "[outside repository omitted]"
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_BINARY", 0))
    except OSError as exc:
        return f"[unreadable: {exc}]"
    try:
        after = os.fstat(descriptor)

        def identity(value: os.stat_result) -> tuple[int, int, int, int, int]:
            return (
                value.st_dev,
                value.st_ino,
                stat.S_IFMT(value.st_mode),
                value.st_size,
                value.st_mtime_ns,
            )

        if after.st_nlink != 1:
            return "[hard-linked file omitted]"
        if not stat.S_ISREG(after.st_mode) or identity(before) != identity(after):
            return "[file changed during read; omitted]"
        with os.fdopen(descriptor, "rb") as stream:
            descriptor = -1
            data = stream.read()
            final = os.fstat(stream.fileno())
            if final.st_nlink != 1:
                return "[hard-linked file omitted]"
            if identity(after) != identity(final):
                return "[file changed during read; omitted]"
    except OSError as exc:
        return f"[unreadable: {exc}]"
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    if b"\0" in data:
        return "[binary file omitted]"
    return bounded(data.decode("utf-8", errors="replace"), limit)


def local_bundle(repo: Path) -> str:
    parts = [
        "# Git Status",
        git(repo, "status", "--short"),
        "# Staged Diff",
        git(repo, "diff", "--cached", "--stat"),
        bounded(git(repo, "diff", "--cached", "--patch", "--find-renames")),
        "# Unstaged Diff",
        git(repo, "diff", "--stat"),
        bounded(git(repo, "diff", "--patch", "--find-renames")),
    ]
    untracked = [
        line
        for line in git(repo, "ls-files", "--others", "--exclude-standard").splitlines()
        if line
    ]
    if untracked:
        parts.append("# Untracked Files")
        for rel in untracked:
            path = repo / rel
            parts.append(f"## {rel}\n{read_untracked_text(repo, path)}")
    return "\n\n".join(parts)


def branch_bundle(repo: Path, base_ref: str) -> str:
    return "\n\n".join(
        [
            "# Branch Diff",
            f"base: {base_ref}",
            git(repo, "diff", "--stat", f"{base_ref}...HEAD"),
            bounded(
                git(
                    repo,
                    "diff",
                    "--patch",
                    "--find-renames",
                    f"{base_ref}...HEAD",
                )
            ),
        ]
    )


def commit_bundle(repo: Path, commit_ref: str) -> str:
    return "\n\n".join(
        [
            "# Commit Diff",
            f"commit: {commit_ref}",
            git(repo, "show", "--stat", "--format=fuller", commit_ref),
            bounded(
                git(
                    repo,
                    "show",
                    "--patch",
                    "--find-renames",
                    "--format=fuller",
                    commit_ref,
                )
            ),
        ]
    )


def review_paths(
    repo: Path, target: str, target_ref: str | None, commit_ref: str
) -> set[str]:
    names: set[str] = set()
    if target == "local":
        sources = [
            git(repo, "diff", "--name-only", "--cached"),
            git(repo, "diff", "--name-only"),
            git(repo, "ls-files", "--others", "--exclude-standard"),
        ]
    elif target == "branch":
        assert target_ref
        sources = [git(repo, "diff", "--name-only", f"{target_ref}...HEAD")]
    else:
        sources = [git(repo, "show", "--name-only", "--format=", commit_ref)]
    for source in sources:
        for line in source.splitlines():
            path = line.strip()
            if path:
                names.add(path)
    return names


def load_extra_prompt(args: argparse.Namespace) -> str:
    chunks: list[str] = []
    for value in args.prompt or []:
        chunks.append(value)
    for path in args.prompt_file or []:
        chunks.append(Path(path).read_text(encoding="utf-8"))
    return "\n\n".join(chunks)


def load_datasets(args: argparse.Namespace) -> str:
    chunks: list[str] = []
    for spec in args.dataset or []:
        path = Path(spec)
        if path.is_dir():
            raise SystemExit(f"--dataset must be a file, got directory: {path}")
        chunks.append(f"# Dataset: {path}\n{read_text(path)}")
    return "\n\n".join(chunks)


def build_prompt(
    repo: Path,
    target: str,
    target_ref: str | None,
    bundle: str,
    extra_prompt: str,
    datasets: str,
) -> str:
    target_line = f"{target} {target_ref}" if target_ref else target
    return textwrap.dedent(
        f"""
        You are a senior code reviewer. Review the provided git change bundle only.

        Hard rules:
        - Return exactly one JSON object and nothing else. Do not wrap it in Markdown.
        - The JSON object must match this schema exactly:
        {json.dumps(SCHEMA, indent=2)}
        - Do not modify files.
        - Do not invoke nested reviewers or review tools.
        - Forbidden nested review commands include: codex review, autoreview, claude review, oracle review.
        - You may use read-only tools and web search to inspect files, dependency contracts, upstream docs, current behavior, and security implications.
        - Shell commands, if available, must be read-only inspection commands. Do not run tests, formatters, package installs, generators, network mutation commands, git mutation commands, or commands that write files.
        - Report only actionable defects introduced or exposed by this change.
        - Prefer high-signal findings over style feedback.
        - Include security findings: injection, secret leaks, authz/authn bypass, path traversal, unsafe deserialization, unsafe filesystem or shell use, privacy leaks, and credential handling.
        - Do not reject legitimate functionality merely because it touches shell, filesystem, network, auth, or sensitive data. Report a security finding only when the patch creates a concrete exploitable risk, removes an important safety check, or lacks validation at a trust boundary.
        - For each finding, use the smallest file/line location that demonstrates the issue.
        - If there are no actionable findings, return an empty findings array and mark the patch correct.

        Review target: {target_line}
        Repository: {repo}

        {extra_prompt}

        {datasets}

        # Change Bundle
        {bundle}
        """
    ).strip()
