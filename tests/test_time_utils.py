# /// script
# dependencies = ["mini-racer==0.14.1"]
# ///

import json
import unittest
from pathlib import Path

from py_mini_racer import MiniRacer


ROOT = Path(__file__).resolve().parents[1]


class TimeUtilitiesTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.context = MiniRacer()
        cls.context.eval("var window = this;")
        cls.context.eval((ROOT / "static" / "time.js").read_text())

    def call(self, function_name, *args):
        arguments = ", ".join(json.dumps(arg) for arg in args)
        return self.context.eval(
            f"window.SjbisTime.{function_name}({arguments})"
        )

    def test_formats_iso_and_legacy_ages_consistently(self):
        now_ms = 1_758_641_560_000
        self.assertEqual(
            self.call(
                "formatAge",
                "2025-09-23T15:31:10.000Z",
                {"nowMs": now_ms},
            ),
            "1m 30s ago",
        )
        self.assertEqual(
            self.call("formatAge", "-00:01:30", {"nowMs": now_ms}),
            "1m 30s ago",
        )

    def test_formats_long_ages_with_a_custom_suffix(self):
        self.assertEqual(
            self.call(
                "formatAge",
                "2025-09-21T12:00:00.000Z",
                {"nowMs": 1_758_641_560_000, "suffix": "old"},
            ),
            "2d 3h old",
        )

    def test_age_falls_back_for_invalid_or_future_values(self):
        now_ms = 1_758_641_560_000
        self.assertEqual(
            self.call("formatAge", "not-a-date", {"nowMs": now_ms}),
            "just now",
        )
        self.assertEqual(
            self.call(
                "formatAge",
                "2025-09-23T15:33:00.000Z",
                {"nowMs": now_ms},
            ),
            "just now",
        )

    def test_formats_pacific_date_time_across_dst(self):
        self.assertEqual(
            self.call("formatPacificDateTime", "2026-01-15T20:34:56Z"),
            "Jan 15, 2026, 12:34:56 PM PST",
        )
        self.assertEqual(
            self.call("formatPacificDateTime", "2026-07-15T19:34:56Z"),
            "Jul 15, 2026, 12:34:56 PM PDT",
        )

    def test_canonicalizes_iso_timestamp_and_rejects_non_absolute_values(self):
        self.assertEqual(
            self.call("toIsoTimestamp", "2026-09-23T13:14:07-07:00"),
            "2026-09-23T20:14:07.000Z",
        )
        self.assertEqual(self.call("formatPacificDateTime", "-00:01:30"), "")
        self.assertEqual(self.call("toIsoTimestamp", "not-a-date"), "")

    def test_script_loads_before_jsx_consumers(self):
        index = (ROOT / "static" / "index.html").read_text()
        time_position = index.index('src="time.js')
        for script in ("tweaks-panel.jsx", "data.jsx", "focus.jsx", "app.jsx"):
            self.assertLess(time_position, index.index(f'src="{script}'))

    def test_direct_deploy_includes_plain_javascript_assets(self):
        deploy_script = (ROOT / "build-and-deploy.sh").read_text()
        self.assertIn("static/*.js static/*.jsx", deploy_script)


if __name__ == "__main__":
    unittest.main()
