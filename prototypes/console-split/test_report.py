"""Keep the published suite count consistent with the recorded Cargo results."""
import json
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ReportTests(unittest.TestCase):
    def test_suite_count_excludes_nested_fixture_runs(self):
        report = (ROOT / "docs/research/console-core-split.md").read_text()
        reported = int(re.search(
            r'\| `cargo test --no-fail-fast` \| [^|]+ \| (\d+) passed', report).group(1))
        data = json.loads((ROOT / "prototypes/console-split/results.json").read_text())
        run = next(row for row in data["runs"] if row["label"] == "validation-test")
        counts = [tuple(map(int, re.search(
            r'(\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out',
            line).groups())) for line in run["results"]]
        actual = sum(row[0] for row in counts if row[4] == 0)
        self.assertEqual(reported, actual, "nested subprocess summaries must not be counted twice")


if __name__ == "__main__":
    unittest.main()
