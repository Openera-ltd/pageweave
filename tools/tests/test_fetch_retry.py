"""Downloads of pinned resources retry network and server errors, not client errors."""
import io
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import fetch_ocr  # noqa: E402
import fetch_pdfium  # noqa: E402


class Resp(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()


def http(code):
    return urllib.error.HTTPError("u", code, "x", {}, None)


class RetryTests(unittest.TestCase):
    def check(self, mod, outcomes, expect_calls, ok=True):
        it = iter(outcomes)

        def urlopen(req, timeout):
            o = next(it)
            if isinstance(o, BaseException):
                raise o
            return Resp(o)

        waits = []
        with tempfile.TemporaryDirectory() as d, mock.patch("urllib.request.urlopen", urlopen):
            target = Path(d) / "a.bin"
            if ok:
                mod.download("https://example.invalid/a", target, wait=waits.append)
                self.assertEqual(target.read_bytes(), b"data")
            else:
                with self.assertRaises(Exception):
                    mod.download("https://example.invalid/a", target, wait=waits.append)
        self.assertEqual(len(waits), expect_calls - 1)
        return waits

    def test_timeouts_and_server_errors_are_retried(self):
        for mod in (fetch_pdfium, fetch_ocr):
            waits = self.check(mod, [TimeoutError("t"), urllib.error.URLError("refused"), http(503), b"data"], 4)
            self.assertEqual(waits, [2, 4, 8])

    def test_client_errors_fail_at_once_and_retries_are_bounded(self):
        for mod in (fetch_pdfium, fetch_ocr):
            self.check(mod, [http(404)], 1, ok=False)
            self.check(mod, [TimeoutError("t")] * 4, 4, ok=False)


if __name__ == "__main__":
    unittest.main()
