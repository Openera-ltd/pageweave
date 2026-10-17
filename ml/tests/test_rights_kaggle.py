import json
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from pwml import corpus, kaggle_run
from pwml.rights import NotTrainable, check_trainable


class RightsGate(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp())
        corpus.generate(cls.tmp, {"train": 3, "blind": 2})

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp)

    def test_generated_train_split_is_trainable(self):
        self.assertEqual(check_trainable(self.tmp, "train")["items"], 3)

    def test_blind_split_is_sealed(self):
        with self.assertRaisesRegex(NotTrainable, "sealed"):
            check_trainable(self.tmp, "blind")

    def test_quarantined_or_tampered_items_block_training(self):
        work = Path(tempfile.mkdtemp())
        try:
            shutil.copytree(self.tmp / "train", work / "train")
            man = work / "train" / "manifest.jsonl"
            rows = [json.loads(l) for l in man.read_text().splitlines()]
            rows[0]["rights"]["status"] = "unknown"
            man.write_text("".join(json.dumps(r) + "\n" for r in rows))
            with self.assertRaisesRegex(NotTrainable, "quarantined"):
                check_trainable(work, "train")
            rows[0]["rights"]["status"] = "approved"
            man.write_text("".join(json.dumps(r) + "\n" for r in rows))
            img = work / rows[1]["image"]
            img.write_bytes(img.read_bytes() + b"x")
            with self.assertRaisesRegex(NotTrainable, "hash"):
                check_trainable(work, "train")
            (work / "train" / "manifest.jsonl").unlink()
            with self.assertRaisesRegex(NotTrainable, "no manifest"):
                check_trainable(work, "train")
        finally:
            shutil.rmtree(work)


class KaggleSafety(unittest.TestCase):
    def test_no_credentials_is_a_clear_refusal(self):
        with mock.patch.dict("os.environ", {"KAGGLE_CONFIG_DIR": tempfile.mkdtemp()}, clear=True):
            self.assertIsNone(kaggle_run.credentials_present())
            with self.assertRaisesRegex(kaggle_run.KaggleError, "environment secret"):
                kaggle_run.preflight()

    def test_credential_source_is_named_never_its_value(self):
        with mock.patch.dict("os.environ", {"KAGGLE_API_TOKEN": "s3cr3t-value"}, clear=True):
            self.assertEqual(kaggle_run.credentials_present(), "KAGGLE_API_TOKEN")

    def test_upload_refuses_the_sealed_split(self):
        tmp = Path(tempfile.mkdtemp())
        try:
            corpus.generate(tmp, {"blind": 1})
            with mock.patch.object(kaggle_run, "username", return_value="someone"):
                with self.assertRaisesRegex(kaggle_run.KaggleError, "sealed"):
                    kaggle_run.publish_dataset(tmp, "pageweave-test", "t")
        finally:
            shutil.rmtree(tmp)

    def test_upload_refuses_line_crops_without_a_rights_record(self):
        tmp = Path(tempfile.mkdtemp())
        try:
            d = tmp / "lines-x"
            d.mkdir()
            (d / "labels.jsonl").write_text('{"image": "crops/a.png", "text": "a"}\n')
            with mock.patch.object(kaggle_run, "kaggle", side_effect=AssertionError("must not call Kaggle")):
                with mock.patch.object(kaggle_run, "username", return_value="someone"):
                    with self.assertRaisesRegex(kaggle_run.KaggleError, "rights record"):
                        kaggle_run.publish_dataset(tmp, "pageweave-test", "t")
                    (d / "lines.json").write_text('{"split": "blind", "manifest_sha256": "x", "rights": "y"}')
                    with self.assertRaisesRegex(kaggle_run.KaggleError, "non-blind"):
                        kaggle_run.publish_dataset(tmp, "pageweave-test", "t")
        finally:
            shutil.rmtree(tmp)

    def test_supersede_only_replaces_the_same_kernel(self):
        other = {"run_id": "r1", "kernel": "u/pageweave-rec-finetune", "status": "queued"}
        same = {"run_id": "r2", "kernel": "u/pageweave-env-probe", "status": "queued"}
        saved = []
        with mock.patch.object(kaggle_run, "list_runs", return_value=[dict(other), dict(same)]), \
             mock.patch.object(kaggle_run, "save_run", side_effect=lambda r: saved.append(r)), \
             mock.patch.object(kaggle_run, "kaggle", side_effect=AssertionError("must not call Kaggle")):
            # Another kernel is still active, so the budget of one concurrent run refuses even with supersede.
            with self.assertRaisesRegex(kaggle_run.KaggleError, "already active"):
                kaggle_run.push(Path("kaggle/probe"), "test", None, supersede=True)
        self.assertEqual([r["run_id"] for r in saved], ["r2"])
        self.assertEqual(saved[0]["status"], "superseded")

    def test_gpu_ceiling_is_enforced_before_anything_is_sent(self):
        with mock.patch.object(kaggle_run, "kaggle", side_effect=AssertionError("must not call Kaggle")):
            with self.assertRaisesRegex(kaggle_run.KaggleError, "ceiling"):
                kaggle_run.push(Path("kaggle/probe"), "test", 99)


if __name__ == "__main__":
    unittest.main()
