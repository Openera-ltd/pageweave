"""Unit tests for the watchdog's pure decision logic with injected failures (PRD 12 acceptance)."""
import datetime as dt
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import watchdog as w  # noqa: E402

NOW = dt.datetime(2026, 10, 16, 23, 0, tzinfo=dt.timezone.utc)


def run(id_, conclusion, minutes_ago, status="completed"):
    t = (NOW - dt.timedelta(minutes=minutes_ago)).isoformat()
    return {"id": id_, "status": status, "conclusion": conclusion, "created_at": t, "run_started_at": t, "html_url": f"u/{id_}"}


class EvaluateTests(unittest.TestCase):
    def test_green_branch_has_no_findings(self):
        self.assertEqual(w.evaluate({"main": [run(1, "success", 5)]}, {}, NOW - dt.timedelta(minutes=10), NOW), [])

    def test_injected_failure_is_detected_with_failed_job_names(self):
        f = w.evaluate({"main": [run(2, "failure", 3), run(1, "success", 30)]},
                       {2: [{"name": "Windows", "conclusion": "failure"}, {"name": "Linux", "conclusion": "success"}]}, None, NOW)
        self.assertEqual([x.kind for x in f], ["ci_failed"])
        self.assertIn("Windows", f[0].detail)

    def test_recovery_uses_latest_completed_run(self):
        f = w.evaluate({"main": [run(3, "success", 1), run(2, "failure", 20)]}, {}, None, NOW)
        self.assertEqual(f, [])

    def test_cancelled_runs_are_ignored(self):
        f = w.evaluate({"main": [run(3, "cancelled", 1), run(2, "success", 20)]}, {}, None, NOW)
        self.assertEqual(f, [])

    def test_stuck_run_detected(self):
        f = w.evaluate({"main": [run(4, None, 120, status="in_progress")]}, {}, None, NOW)
        self.assertEqual([x.kind for x in f], ["ci_stuck"])

    def test_missed_interval_detected_but_not_part_of_signature(self):
        f = w.evaluate({"main": []}, {}, NOW - dt.timedelta(minutes=60), NOW)
        self.assertEqual([x.kind for x in f], ["missed_checks"])
        self.assertEqual(w.signature(f), "")


class PlanTests(unittest.TestCase):
    def failing(self, jobs=("Windows",)):
        return w.evaluate({"main": [run(2, "failure", 3)]}, {2: [{"name": j, "conclusion": "failure"} for j in jobs]}, None, NOW)

    def test_open_then_dedupe_then_update_then_close(self):
        f = self.failing()
        self.assertEqual(w.plan(f, None), "open")
        issue = {"number": 7, "body": w.render_body(f, w.signature(f), NOW)}
        # Same problem on the next run (different run id) → no new noise.
        again = w.evaluate({"main": [run(9, "failure", 1)]}, {9: [{"name": "Windows", "conclusion": "failure"}]}, None, NOW)
        self.assertEqual(w.plan(again, issue), "none")
        # A different problem → update the same issue, never a second one.
        self.assertEqual(w.plan(self.failing(("Linux",)), issue), "update")
        # Healthy → close.
        self.assertEqual(w.plan([], issue), "close")
        self.assertEqual(w.plan([], None), "none")

    def test_body_contains_no_tokens_and_has_signature(self):
        f = self.failing()
        body = w.render_body(f, w.signature(f), NOW)
        self.assertIn(w.SIG_MARK, body)
        self.assertNotIn("Bearer", body)


if __name__ == "__main__":
    unittest.main()
