import argparse
import importlib.util
import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / ".github/scripts/release-metadata.py"


def load_module():
    spec = importlib.util.spec_from_file_location("release_metadata", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BeijingBuildDateTest(unittest.TestCase):
    """构建序号按当天北京时间归日，同一天从 01 起递增。

    规则来源：发版 tag 的日期段取当天北京时间，序号在同一自然日内递增，失败
    的构建也占用序号，版本号不在本次发布中改就沿用上一版。
    """

    @classmethod
    def setUpClass(cls):
        cls.metadata = load_module()

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def _write_metadata(self, value):
        path = self.tmp / "version.json"
        path.write_text(json.dumps(value))
        return path

    def _write_tags(self, tags):
        path = self.tmp / "tags.txt"
        path.write_text("\n".join(tags) + "\n")
        return path

    def test_midnight_beijing_counts_as_that_day(self):
        # UTC 16:00 已经是北京时间次日 00:00，必须记到新的一天。
        moment = datetime(2026, 10, 24, 16, 0, 0, tzinfo=timezone.utc)
        self.assertEqual(self.metadata.beijing_build_date(moment), "20261025")

    def test_just_before_beijing_midnight_stays_on_the_previous_day(self):
        moment = datetime(2026, 10, 24, 15, 59, 59, tzinfo=timezone.utc)
        self.assertEqual(self.metadata.beijing_build_date(moment), "20261024")

    def test_beijing_morning_is_not_rolled_back_to_utc_date(self):
        # UTC 00:30 是北京时间 08:30，按 UTC 会错记成前一天。
        moment = datetime(2026, 10, 25, 0, 30, 0, tzinfo=timezone.utc)
        self.assertEqual(self.metadata.beijing_build_date(moment), "20261025")

    def test_first_build_of_the_day_starts_at_one(self):
        resolved = self.metadata.resolve_release_metadata(
            version="1.5.4",
            build_date="20261025",
            tags=["v1.5.4-build-2026.10.24-04"],
        )
        self.assertEqual(resolved["build_number"], "20261025.1")
        self.assertEqual(resolved["build_seq"], 2026102501)

    def test_sequence_continues_within_the_same_day(self):
        resolved = self.metadata.resolve_release_metadata(
            version="1.5.4",
            build_date="20261025",
            tags=[
                "v1.5.4-build-2026.10.25-01",
                "v1.5.4-build-2026.10.25-02",
            ],
        )
        self.assertEqual(resolved["build_number"], "20261025.3")
        self.assertEqual(resolved["build_seq"], 2026102503)

    def test_version_keeps_previous_value_when_not_bumped(self):
        # 没有打新版本 tag 时，版本号沿用 metadata 里的值，只有日期与序号推进。
        args = argparse.Namespace(
            metadata=self._write_metadata(
                {"product": "rustdesk-yan", "edition": "custom", "version": "1.5.4",
                 "build_number": "20261024.4", "build_seq": 2026102404,
                 "android_version_code": 2026102404, "channel": "stable"}
            ),
            tags_file=self._write_tags(["v1.5.4-build-2026.10.24-04"]),
            output=self.tmp / "resolved.json",
            build_date="20261025",
        )
        self.metadata._resolve(args)
        resolved = json.loads(args.output.read_text())
        self.assertEqual(resolved["version"], "1.5.4")
        self.assertEqual(resolved["build_number"], "20261025.1")
        self.assertEqual(resolved["build_seq"], 2026102501)


class FailedBuildStillConsumesSequenceTest(unittest.TestCase):
    """失败的构建也占用序号：下一次发布必须拿到更大的序号。

    实现依据：每次打 tag 前先把 build_number / build_seq 推进并提交，标记位
    metadata_sequence_floor 会把 version.json 里的序号当作当日下限，即使当天
    之前的构建失败、没有留下 tag，下一次也不会复用同一个序号。
    """

    @classmethod
    def setUpClass(cls):
        cls.metadata = load_module()

    def test_floor_keeps_the_sequence_when_no_tag_was_created(self):
        floor = self.metadata.metadata_sequence_floor(
            version="1.5.4",
            build_date="20261025",
            tags=["v1.5.4-build-2026.10.25-01"],
            build_seq=2026102502,
        )
        self.assertEqual(floor, 2)

    def test_floor_starts_at_one_on_a_fresh_day(self):
        floor = self.metadata.metadata_sequence_floor(
            version="1.5.4",
            build_date="20261025",
            tags=["v1.5.4-build-2026.10.24-04"],
            build_seq=2026102404,
        )
        self.assertEqual(floor, 1)

    def test_floor_ignores_a_sequence_from_another_day(self):
        floor = self.metadata.metadata_sequence_floor(
            version="1.5.4",
            build_date="20261025",
            tags=["v1.5.4-build-2026.10.25-03"],
            build_seq=2026102409,
        )
        self.assertEqual(floor, 1)


if __name__ == "__main__":
    unittest.main()
