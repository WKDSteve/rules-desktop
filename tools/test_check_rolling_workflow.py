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
    strategy:
      matrix:
        include:
          - name: linux
            runs-on: ubuntu-22.04
            experimental: false
          - name: macos
            experimental: true
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
      - run: |
          test -f dist/rules-hub-windows-x86_64.exe
          test -f dist/rules-hub-x86_64-unknown-linux-gnu.tar.gz
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
    strategy:
      matrix:
        include:
          - name: linux
            triple: x86_64-unknown-linux-gnu
            experimental: false
    steps:
      - run: |
          tar -tzf "dist/$name" > stage.list
          grep -qx "./rules-hub" stage.list
  publish:
    needs: [check, build, build-unix]
    if: ${{ !cancelled() && needs.check.result == 'success' && needs.build.result == 'success' && needs.build-unix.result != 'failure' }}
    steps:
      - run: |
          test -f "dist/rules-hub-${VERSION}-x86_64-pc-windows-msvc.zip" || exit 1
          test -f "dist/rules-hub-${VERSION}-x86_64-unknown-linux-gnu.tar.gz" || exit 1
          gh release delete-asset "$GITHUB_REF_NAME" "$mac" --yes || true
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

    def test_release_publish_not_waiting_for_the_unix_legs(self):
        found = check.release_problems(RELEASE.replace("needs: [check, build, build-unix]", "needs: [check, build]"))
        self.assertIn("publish: it does not wait for build-unix (needs)", found)

    def test_release_publish_without_a_green_windows_build(self):
        found = check.release_problems(RELEASE.replace("needs.build.result == 'success' && ", ""))
        self.assertIn("publish: its if lacks needs.build.result == 'success'", found)

    def test_release_publish_joined_with_or(self):
        found = check.release_problems(RELEASE.replace("!cancelled() && needs.check", "!cancelled() || needs.check"))
        self.assertIn("publish: its if joins with || (every guard must hold)", found)

    def test_release_without_the_linux_file_check(self):
        text = RELEASE.replace('          test -f "dist/rules-hub-${VERSION}-x86_64-unknown-linux-gnu.tar.gz" || exit 1\n', "")
        self.assertIn("publish: no test -f requires the Linux file", check.release_problems(text))

    def test_release_with_an_experimental_linux_leg(self):
        text = RELEASE.replace("triple: x86_64-unknown-linux-gnu\n            experimental: false",
                               "triple: x86_64-unknown-linux-gnu\n            experimental: true")
        self.assertIn("build-unix: the linux leg is not experimental: false", check.release_problems(text))

    def test_release_keeping_a_stale_macos_file(self):
        text = RELEASE.replace('          gh release delete-asset "$GITHUB_REF_NAME" "$mac" --yes || true\n', "")
        self.assertTrue(any("delete-asset" in p for p in check.release_problems(text)))


class CheckRollingRequiredLegs(unittest.TestCase):
    def test_rolling_without_the_windows_file_check(self):
        text = GOOD.replace("          test -f dist/rules-hub-windows-x86_64.exe\n", "")
        self.assertIn("publish: no test -f requires the Windows file", check.problems(text))

    def test_rolling_with_an_experimental_linux_leg(self):
        text = GOOD.replace("runs-on: ubuntu-22.04\n            experimental: false",
                            "runs-on: ubuntu-22.04\n            experimental: true")
        self.assertIn("build: the linux leg is not experimental: false", check.problems(text))


class CheckMain(unittest.TestCase):
    def test_a_name_ending_in_release_yml_gets_the_rolling_rules(self):
        import io, contextlib, tempfile, pathlib as p
        with tempfile.TemporaryDirectory() as folder:
            path = p.Path(folder) / "prerelease.yml"
            path.write_text(GOOD, encoding="utf-8")
            argv = check.sys.argv
            check.sys.argv = ["x", str(path)]
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(check.main(), 0)
            finally:
                check.sys.argv = argv

    def test_an_unreadable_path_does_not_hide_the_others(self):
        import io, contextlib, tempfile, pathlib as p
        with tempfile.TemporaryDirectory() as folder:
            bad = p.Path(folder) / "rolling.yml"
            bad.write_text(GOOD.replace("!cancelled() && ", ""), encoding="utf-8")
            argv = check.sys.argv
            check.sys.argv = ["x", str(p.Path(folder) / "missing.yml"), str(bad)]
            out = io.StringIO()
            try:
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(check.main(), 2)
            finally:
                check.sys.argv = argv
            self.assertIn("lacks !cancelled()", out.getvalue())


if __name__ == "__main__":
    unittest.main()
