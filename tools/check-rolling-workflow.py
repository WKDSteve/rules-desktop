#!/usr/bin/env python3
"""Fail when the rolling build loses one of the guards of its Linux and macOS legs.

The rolling build (.github/workflows/rolling.yml) builds Windows on the self-hosted runner and
Linux and macOS on hosted runners, the macOS leg experimental. Three things went wrong or nearly so
and are checked here as text, since a workflow cannot be run locally:

  * publish must run when the experimental leg was cancelled (a timeout makes the result of the
    matrix `cancelled`) but not when a required leg failed, and never from a cancelled run:
    its `if` holds `!cancelled()` and `needs.build.result != 'failure'`, and no `always()`;
  * the artifacts are named by the commit (`needs.prepare.outputs.short`) on upload and download:
    a re-run whose prepare picks a newer commit must not publish an older leg's file;
  * an archive listing is never piped into `grep -q`: GNU tar dies of SIGPIPE when a line follows
    the match, and pipefail turns that into a red Linux leg (the first run failed so).

The release (.github/workflows/release.yml) builds the Unix legs in a job of its own, build-unix: its publish
holds `!cancelled()` and `needs.build-unix.result != 'failure'` (no `always()`), and the grep -q rule holds
there too. A release builds the commit of its tag, so its artifacts need no commit in their names.

    python tools/check-rolling-workflow.py [PATH ...]

Without a path both workflows are checked; a path whose name is release.yml gets the release rules.
Exit 0: all guards are there. Exit 1: the missing ones are listed. Exit 2: a file cannot be read.
"""

import re
import sys


def job(text: str, name: str) -> str:
    """The lines of one job: from `  <name>:` up to the next job or the end."""
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text, re.M | re.S)
    return match.group(1) if match else ""


def publish_guard(publish: str, unix_job: str) -> list[str]:
    """The `if` of publish: never from a cancelled run, never over a failed required Unix leg."""
    found = []
    cond = re.search(r"^    if: (.+)$", publish, re.M)
    cond = cond.group(1) if cond else ""
    if "!cancelled()" not in cond:
        found.append("publish: its if lacks !cancelled()")
    if f"needs.{unix_job}.result != 'failure'" not in cond:
        found.append(f"publish: its if lacks needs.{unix_job}.result != 'failure'")
    if "always()" in cond:
        found.append("publish: its if uses always() (that includes a cancelled run)")
    return found


def piped_listings(text: str) -> list[str]:
    return [f"line {number}: an archive listing is piped into grep -q"
            for number, line in enumerate(text.splitlines(), 1)
            if re.search(r"tar\s+-t\S*.*\|\s*grep\s+-\S*q", line)]


def release_problems(text: str) -> list[str]:
    publish = job(text, "publish")
    if not publish:
        return ["no publish job"]
    found = publish_guard(publish, "build-unix")
    if not job(text, "build-unix"):
        found.append("no build-unix job")
    return found + piped_listings(text)


def problems(text: str) -> list[str]:
    publish = job(text, "publish")
    if not publish:
        return ["no publish job"]
    found = publish_guard(publish, "build")
    build = job(text, "build")
    upload = re.search(r"^\s+name: (rolling-.*)$", build, re.M)
    if not upload or "needs.prepare.outputs.short" not in upload.group(1):
        found.append("build: the artifact name is not bound to needs.prepare.outputs.short")
    download = re.search(r"^\s+pattern: (rolling-.*)$", publish, re.M)
    if not download or "needs.prepare.outputs.short" not in download.group(1):
        found.append("publish: the artifact pattern is not bound to needs.prepare.outputs.short")
    return found + piped_listings(text)


def main() -> int:
    paths = sys.argv[1:] or [".github/workflows/rolling.yml", ".github/workflows/release.yml"]
    failed = False
    for path in paths:
        try:
            with open(path, encoding="utf-8") as handle:
                text = handle.read()
        except OSError as error:
            print(f"check-rolling-workflow: cannot read {path}: {error}", file=sys.stderr)
            return 2
        check = release_problems if path.replace("\\", "/").endswith("release.yml") else problems
        for problem in check(text):
            print(f"{path}: {problem}")
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
