"""Tests of tools/check-rolling-workflow.py: each guard, taken out of a good workflow, is reported."""

import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("check_rolling_workflow", HERE / "check-rolling-workflow.py")
check = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check)

GOOD = """jobs:
  prepare:
    runs-on: ubuntu-latest
  build:
    steps:
      - run: |
          tar -tzf "dist/$ASSET" > stage.list
          grep -qx "./$PROGRAM" stage.list
      - uses: actions/upload-artifact@x
        with:
          name: rolling-${{ matrix.name }}-${{ needs.prepare.outputs.short }}
  publish:
    needs: [prepare, build]
    if: ${{ !cancelled() && needs.prepare.outputs.current == 'true' && needs.build.result != 'failure' }}
    steps:
      - uses: actions/download-artifact@x
        with:
          pattern: rolling-*-${{ needs.prepare.outputs.short }}
"""


class CheckRollingWorkflow(unittest.TestCase):
    def test_the_good_workflow_passes(self):
        self.assertEqual(check.problems(GOOD), [])

    def test_the_real_workflow_passes(self):
        text = (HERE.parent / ".github" / "workflows" / "rolling.yml").read_text(encoding="utf-8")
        self.assertEqual(check.problems(text), [])

    def assert_reported(self, text, words):
        found = check.problems(text)
        self.assertTrue(any(words in line for line in found), found)

    def test_always_instead_of_not_cancelled(self):
        self.assert_reported(GOOD.replace("!cancelled()", "always()"), "lacks !cancelled()")
        self.assert_reported(GOOD.replace("!cancelled()", "always()"), "always()")

    def test_publish_without_the_failure_guard(self):
        self.assert_reported(GOOD.replace(" && needs.build.result != 'failure'", ""), "result != 'failure'")

    def test_artifact_names_not_bound_to_the_commit(self):
        self.assert_reported(GOOD.replace("-${{ needs.prepare.outputs.short }}\n  publish", "\n  publish"),
                             "artifact name")
        self.assert_reported(GOOD.replace("pattern: rolling-*-${{ needs.prepare.outputs.short }}",
                                          "pattern: rolling-*"), "artifact pattern")

    def test_a_listing_piped_into_grep_q(self):
        piped = GOOD.replace('tar -tzf "dist/$ASSET" > stage.list',
                             'tar -tzf "dist/$ASSET" | grep -qx "./$PROGRAM"')
        self.assert_reported(piped, "piped into grep -q")

    def test_no_publish_job(self):
        self.assertEqual(check.problems("jobs:\n  build:\n    x: 1\n"), ["no publish job"])


RELEASE = """jobs:
  build:
    runs-on: [self-hosted, windows]
  build-unix:
    steps:
      - run: |
          tar -tzf "dist/$name" > stage.list
          grep -qx "./rules-hub" stage.list
  publish:
    needs: [check, build, build-unix]
    if: ${{ !cancelled() && needs.check.result == 'success' && needs.build.result == 'success' && needs.build-unix.result != 'failure' }}
"""


class CheckReleaseWorkflow(unittest.TestCase):
    def test_the_good_release_passes(self):
        self.assertEqual(check.release_problems(RELEASE), [])

    def test_the_real_release_passes(self):
        text = (HERE.parent / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertEqual(check.release_problems(text), [])

    def test_release_publish_without_a_guard(self):
        found = check.release_problems(RELEASE.replace("!cancelled() && ", ""))
        self.assertIn("publish: its if lacks !cancelled()", found)
        found = check.release_problems(RELEASE.replace(" && needs.build-unix.result != 'failure'", ""))
        self.assertIn("publish: its if lacks needs.build-unix.result != 'failure'", found)

    def test_release_without_unix_legs(self):
        text = RELEASE.replace("  build-unix:\n", "  other:\n")
        self.assertIn("no build-unix job", check.release_problems(text))

    def test_release_with_a_piped_listing(self):
        piped = RELEASE.replace('tar -tzf "dist/$name" > stage.list', 'tar -tzf "dist/$name" | grep -q x')
        self.assertTrue(any("piped into grep -q" in p for p in check.release_problems(piped)))


if __name__ == "__main__":
    unittest.main()
