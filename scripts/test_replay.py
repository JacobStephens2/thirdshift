"""Check the prompts emitted for reviews of historical repository snapshots."""
from pathlib import Path
import runpy
import unittest


REPLAY = Path(__file__).resolve().parent.parent / "docs/research/second-opinion-replays/replay.py"


class PlanReplay(unittest.TestCase):
    def test_historical_plan_review_names_its_existing_glossary(self):
        replay = runpy.run_path(str(REPLAY))
        prompt = replay["prompt_for"](
            {"kind": "plan", "harness": "claude"}, Path("replay-workspace")
        )
        # Plan #359's c8fcb03 snapshot predates the glossary rename.
        self.assertIn("`CONTEXT.md`", prompt)


if __name__ == "__main__":
    unittest.main()
