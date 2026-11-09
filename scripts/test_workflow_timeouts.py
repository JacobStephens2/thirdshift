"""Guard the job-level execution bounds consumed by GitHub Actions."""
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parent.parent


def job_blocks(workflow):
    # Workflow jobs use two-space indentation; step-level bounds do not count.
    jobs = workflow.read_text().split("\njobs:\n", 1)[1]
    starts = list(re.finditer(r"^  ([\w-]+):$", jobs, re.MULTILINE))
    blocks = {}
    for index, match in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(jobs)
        blocks[match[1]] = jobs[match.end():end]
    return blocks


class WorkflowTimeouts(unittest.TestCase):
    def assert_bounded(self, jobs):
        self.assertTrue(jobs, "expected workflow jobs")
        for name, block in jobs.items():
            with self.subTest(job=name):
                timeout = re.search(r"^    timeout-minutes: (\d+)$", block, re.MULTILINE)
                self.assertIsNotNone(timeout, f"{name} inherits GitHub's 360-minute default")
                self.assertGreater(int(timeout[1]), 0)
                self.assertLess(int(timeout[1]), 360)

    def test_ci_jobs_have_execution_bounds(self):
        self.assert_bounded(job_blocks(ROOT / ".github/workflows/ci.yml"))

    def test_pr_release_jobs_have_execution_bounds(self):
        jobs = job_blocks(ROOT / ".github/workflows/release.yml")
        self.assert_bounded({name: jobs[name] for name in (
            "plan", "build-local-artifacts", "build-global-artifacts",
        )})
        # Reusable-workflow callers cannot set timeout-minutes; their concrete
        # jobs must carry the bounds instead.
        self.assert_bounded(job_blocks(ROOT / ".github/workflows/release-smoke.yml"))


if __name__ == "__main__":
    unittest.main()
