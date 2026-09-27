# /// script
# dependencies = ["mini-racer==0.14.1"]
# ///

import json
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from py_mini_racer import MiniRacer


ROOT = Path(__file__).resolve().parents[1]
NOW = datetime(2026, 9, 27, 12, tzinfo=timezone.utc)


def days_ago(days):
    return (NOW - timedelta(days=days)).isoformat().replace("+00:00", "Z")


class SourceVisibilityPolicyTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.context = MiniRacer()
        cls.context.eval("var window = this;")
        cls.context.eval((ROOT / "static" / "source-visibility.js").read_text())

    def partition(
        self,
        summaries,
        current_source_keys=None,
        selected_filter=None,
        preset="7d",
    ):
        arguments = ", ".join(
            json.dumps(argument)
            for argument in (
                summaries,
                current_source_keys,
                selected_filter,
                preset,
                int(NOW.timestamp() * 1000),
            )
        )
        result = self.context.eval(
            "JSON.stringify(window.SjbisSourceVisibility.partitionSourceKeys("
            f"{arguments}))"
        )
        return json.loads(result)

    def test_partitions_sources_using_the_selected_activity_window(self):
        result = self.partition(
            {
                "recent": {"last_activity_at": days_ago(2)},
                "at-cutoff": {"last_activity_at": days_ago(7)},
                "stale": {"last_activity_at": days_ago(8)},
                "never-active": {"last_activity_at": None},
            }
        )

        self.assertEqual(result["visible"], ["recent", "at-cutoff"])
        self.assertEqual(result["hidden"], ["stale", "never-active"])

    def test_current_and_open_notification_sources_override_staleness(self):
        result = self.partition(
            {
                "displayed-open": {"last_activity_at": days_ago(90)},
                "summary-open": {
                    "last_activity_at": None,
                    "has_open_notification": True,
                },
                "stale": {"last_activity_at": days_ago(90)},
            },
            current_source_keys=["displayed-open"],
        )

        self.assertEqual(result["visible"], ["displayed-open", "summary-open"])
        self.assertEqual(result["hidden"], ["stale"])

    def test_selected_filter_source_stays_visible(self):
        result = self.partition(
            {
                "selected": {"last_activity_at": None},
                "stale": {"last_activity_at": days_ago(60)},
            },
            selected_filter="selected",
        )

        self.assertEqual(result["visible"], ["selected"])
        self.assertEqual(result["hidden"], ["stale"])

    def test_presets_include_show_all_and_invalid_values_fall_back_to_seven_days(self):
        summaries = {
            "two-days-old": {"last_activity_at": days_ago(2)},
            "twenty-days-old": {"last_activity_at": days_ago(20)},
            "thirty-one-days-old": {"last_activity_at": days_ago(31)},
            "never-active": {"last_activity_at": None},
        }

        self.assertEqual(
            self.partition(summaries, preset="1d")["hidden"],
            [
                "two-days-old",
                "twenty-days-old",
                "thirty-one-days-old",
                "never-active",
            ],
        )
        self.assertEqual(
            self.partition(summaries, preset="30d"),
            {
                "visible": ["two-days-old", "twenty-days-old"],
                "hidden": ["thirty-one-days-old", "never-active"],
            },
        )
        self.assertEqual(
            self.partition(summaries, preset="all")["visible"],
            [
                "two-days-old",
                "twenty-days-old",
                "thirty-one-days-old",
                "never-active",
            ],
        )
        self.assertEqual(
            self.partition(summaries, preset="not-a-preset")["visible"],
            ["two-days-old"],
        )

    def test_accepts_array_summaries_and_exports_a_commonjs_api(self):
        result = self.partition(
            [{"name": "array-source", "last_activity_at": days_ago(1)}]
        )
        self.assertEqual(result["visible"], ["array-source"])

        node_context = MiniRacer()
        node_context.eval("var module = { exports: {} };")
        node_context.eval((ROOT / "static" / "source-visibility.js").read_text())
        self.assertEqual(
            node_context.eval("typeof module.exports.partitionSourceKeys"),
            "function",
        )

    def test_browser_loads_policy_before_jsx_consumers(self):
        index = (ROOT / "static" / "index.html").read_text()
        policy_position = index.index('src="source-visibility.js')
        for script in ("tweaks-panel.jsx", "data.jsx", "focus.jsx", "app.jsx"):
            self.assertLess(policy_position, index.index(f'src="{script}'))


if __name__ == "__main__":
    unittest.main()
