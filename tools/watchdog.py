#!/usr/bin/env python3
"""PageWeave watchdog (PRD 12 "Every-10-minute watchdog").

Deterministic, read-mostly health check run on a schedule. It inspects CI runs for the watched
branches, detects failures, stuck runs and missed watchdog intervals, and keeps ONE tracking issue
(label `watchdog`) in sync: opened/updated on a new failure signature, left untouched while the same
signature persists (no comment spam), commented and closed when everything recovers.

Safeguards: no AI calls, no auto-repair, no deletes, no releases; only issue create/edit/comment/close.
Every network call has a timeout; the decision logic (`evaluate`, `plan`) is pure and unit-tested.

Env: GITHUB_TOKEN, GITHUB_REPOSITORY (owner/repo), WATCH_BRANCHES (comma list), WATCHDOG_DRY_RUN=1 to
only print the plan. Writes a JSON report to WATCHDOG_REPORT (default watchdog-report.json).
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import sys
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass, field

API = "https://api.github.com"
LABEL = "watchdog"
STUCK_AFTER = dt.timedelta(minutes=90)
MISSED_AFTER = dt.timedelta(minutes=35)  # > 3 intervals of 10 min


@dataclass(frozen=True)
class Finding:
    kind: str  # ci_failed | ci_stuck | missed_checks
    severity: str  # error | warning
    key: str  # stable part used for deduplication
    detail: str


@dataclass
class Report:
    checked_at: str
    branches: list[str]
    findings: list[Finding] = field(default_factory=list)
    signature: str = ""
    action: str = ""
    notes: list[str] = field(default_factory=list)


def parse_time(s: str | None) -> dt.datetime | None:
    if not s:
        return None
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def evaluate(runs_by_branch: dict[str, list[dict]], jobs_by_run: dict[int, list[dict]],
             previous_watchdog_run: dt.datetime | None, now: dt.datetime) -> list[Finding]:
    """Pure: turn CI state into findings."""
    out: list[Finding] = []
    for branch, runs in sorted(runs_by_branch.items()):
        completed = [r for r in runs if r.get("status") == "completed" and r.get("conclusion") != "cancelled"]
        if completed:
            latest = max(completed, key=lambda r: r["created_at"])
            if latest.get("conclusion") not in ("success", "skipped"):
                failed = sorted(j["name"] for j in jobs_by_run.get(latest["id"], []) if j.get("conclusion") == "failure")
                names = ", ".join(failed) or latest.get("conclusion", "unknown")
                out.append(Finding("ci_failed", "error", f"{branch}|{names}",
                                   f"CI on `{branch}` is failing ({names}) — run {latest.get('html_url', latest['id'])}"))
        for r in runs:
            if r.get("status") in ("in_progress", "queued"):
                started = parse_time(r.get("run_started_at") or r.get("created_at"))
                if started and now - started > STUCK_AFTER:
                    out.append(Finding("ci_stuck", "warning", f"{branch}|stuck",
                                       f"CI run on `{branch}` has been {r['status']} for {int((now - started).total_seconds() // 60)} min — {r.get('html_url', r['id'])}"))
    if previous_watchdog_run and now - previous_watchdog_run > MISSED_AFTER:
        gap = int((now - previous_watchdog_run).total_seconds() // 60)
        out.append(Finding("missed_checks", "warning", "missed",
                           f"Previous watchdog run was {gap} min ago; scheduled checks were missed (GitHub schedules are best-effort)."))
    return out


def signature(findings: list[Finding]) -> str:
    """Stable hash of what is wrong (not when), so repeated runs with the same problem dedupe."""
    keys = sorted(f"{f.kind}:{f.key}" for f in findings if f.kind != "missed_checks")
    return hashlib.sha256("\n".join(keys).encode()).hexdigest()[:16] if keys else ""


SIG_MARK = "<!-- watchdog-signature:"


def plan(findings: list[Finding], open_issue: dict | None) -> str:
    """Pure: decide the single issue action. Returns one of: none, open, update, close."""
    sig = signature(findings)
    if not sig:
        return "close" if open_issue else "none"
    if not open_issue:
        return "open"
    return "none" if f"{SIG_MARK}{sig}" in (open_issue.get("body") or "") else "update"


def render_body(findings: list[Finding], sig: str, now: dt.datetime) -> str:
    lines = [
        "Automated watchdog report. This issue is updated when the set of problems changes and closed when they clear.",
        "",
        f"Last change detected: {now.strftime('%Y-%m-%d %H:%M UTC')}",
        "",
    ]
    lines += [f"- **{f.severity}** — {f.detail}" for f in findings]
    lines += ["", "No automatic repair is attempted. See `tools/watchdog.py`.", f"{SIG_MARK}{sig} -->"]
    return "\n".join(lines)


class GitHub:
    def __init__(self, token: str, repo: str):
        self.token = token
        self.repo = repo

    def call(self, method: str, path: str, body: dict | None = None):
        req = urllib.request.Request(
            f"{API}/repos/{self.repo}{path}",
            method=method,
            data=json.dumps(body).encode() if body is not None else None,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
                "User-Agent": "pageweave-watchdog",
            },
        )
        with urllib.request.urlopen(req, timeout=30) as r:
            data = r.read()
            return json.loads(data) if data else None


def main() -> int:
    now = dt.datetime.now(dt.timezone.utc)
    token = os.environ.get("GITHUB_TOKEN", "")
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    branches = [b.strip() for b in os.environ.get("WATCH_BRANCHES", "main").split(",") if b.strip()]
    dry = os.environ.get("WATCHDOG_DRY_RUN") == "1"
    report = Report(checked_at=now.isoformat(), branches=branches)
    if not token or not repo:
        print("error: GITHUB_TOKEN and GITHUB_REPOSITORY are required", file=sys.stderr)
        return 2
    gh = GitHub(token, repo)
    try:
        runs_by_branch: dict[str, list[dict]] = {}
        jobs_by_run: dict[int, list[dict]] = {}
        for b in branches:
            runs = gh.call("GET", f"/actions/workflows/ci.yml/runs?branch={urllib.request.quote(b)}&per_page=10")["workflow_runs"]
            runs_by_branch[b] = runs
            done = [r for r in runs if r.get("status") == "completed" and r.get("conclusion") not in ("success", "cancelled", "skipped")]
            for r in done[:1]:
                jobs_by_run[r["id"]] = gh.call("GET", f"/actions/runs/{r['id']}/jobs")["jobs"]
        try:
            wd_runs = gh.call("GET", "/actions/workflows/watchdog.yml/runs?status=completed&per_page=1").get("workflow_runs", [])
        except urllib.error.HTTPError as e:
            if e.code != 404:
                raise
            wd_runs = []
            report.notes.append("watchdog workflow not registered on the default branch yet; missed-check detection inactive")
        previous = parse_time(wd_runs[0]["created_at"]) if wd_runs else None
        findings = evaluate(runs_by_branch, jobs_by_run, previous, now)
        issues = gh.call("GET", f"/issues?labels={LABEL}&state=open&per_page=5") or []
        issue = issues[0] if issues else None
        action = plan(findings, issue)
        sig = signature(findings)
        report.findings, report.signature, report.action = findings, sig, action
        if not dry:
            if action == "open":
                gh.call("POST", "/issues", {"title": "Watchdog: CI health problems", "labels": [LABEL], "body": render_body(findings, sig, now)})
            elif action == "update":
                gh.call("PATCH", f"/issues/{issue['number']}", {"body": render_body(findings, sig, now)})
            elif action == "close":
                gh.call("POST", f"/issues/{issue['number']}/comments", {"body": f"All watched checks are healthy again as of {now.strftime('%Y-%m-%d %H:%M UTC')}. Closing."})
                gh.call("PATCH", f"/issues/{issue['number']}", {"state": "closed", "state_reason": "completed"})
    except (urllib.error.URLError, TimeoutError, KeyError, ValueError) as e:
        report.notes.append(f"check incomplete: {type(e).__name__}")
        print(json.dumps(asdict(report), indent=2))
        _write(report)
        return 3
    print(json.dumps(asdict(report), indent=2))
    _write(report)
    return 0


def _write(report: Report) -> None:
    path = os.environ.get("WATCHDOG_REPORT", "watchdog-report.json")
    try:
        with open(path, "w", encoding="utf-8") as f:
            json.dump(asdict(report), f, indent=2)
    except OSError:
        pass


if __name__ == "__main__":
    sys.exit(main())
