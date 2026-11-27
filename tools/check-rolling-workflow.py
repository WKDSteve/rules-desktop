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

In both, publish waits for the legs (`needs`), joins its guards with `&&` only, requires the Windows and the
Linux file with `test -f` before it counts, and the Linux leg is `experimental: false`.

The release (.github/workflows/release.yml) builds the Unix legs in a job of its own, build-unix: its publish
holds `!cancelled()`, `needs.check.result == 'success'`, `needs.build.result == 'success'` and
`needs.build-unix.result != 'failure'` (no `always()`), takes a macOS file of an earlier cut off the draft
(`delete-asset`; publish removes nothing from a release), and the grep -q rule holds there too. A release builds
the commit of its tag, so its artifacts need no commit in their names.

    python tools/check-rolling-workflow.py [PATH ...]

Without a path both workflows are checked; a path whose name is release.yml gets the release rules.
Exit 0: all guards are there. Exit 1: the missing ones are listed. Exit 2: a file cannot be read.
"""

import os
import re
import sys


def job(text: str, name: str) -> str:
    """The lines of one job: from `  <name>:` up to the next job or the end."""
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text, re.M | re.S)
    return match.group(1) if match else ""


def publish_guard(publish: str, unix_job: str, also_success: tuple[str, ...] = ()) -> list[str]:
    """The `if` of publish: never from a cancelled run, never over a failed required leg; publish waits for them."""
    found = []
    cond = re.search(r"^    if: (.+)$", publish, re.M)
    cond = cond.group(1) if cond else ""
    if "!cancelled()" not in cond:
        found.append("publish: its if lacks !cancelled()")
    if f"needs.{unix_job}.result != 'failure'" not in cond:
        found.append(f"publish: its if lacks needs.{unix_job}.result != 'failure'")
    for name in also_success:
        if f"needs.{name}.result == 'success'" not in cond:
            found.append(f"publish: its if lacks needs.{name}.result == 'success'")
    if "always()" in cond:
        found.append("publish: its if uses always() (that includes a cancelled run)")
    if "||" in cond:
        found.append("publish: its if joins with || (every guard must hold)")
    needs = re.search(r"^    needs: \[(.*)\]$", publish, re.M)
    listed = [n.strip() for n in needs.group(1).split(",")] if needs else []
    for name in (unix_job, *also_success):
        if name not in listed:
            found.append(f"publish: it does not wait for {name} (needs)")
    return found


def required_files(publish: str) -> list[str]:
    """The last guard: the Windows and the Linux file must be there before publish counts."""
    tests = [line for line in publish.splitlines() if re.search(r"\btest -f\b", line)]
    found = []
    for system, word in (("Windows", "windows"), ("Linux", "x86_64-unknown-linux-gnu")):
        if not any(word in line for line in tests):
            found.append(f"publish: no test -f requires the {system} file")
    return found


def linux_required(build: str, job_name: str) -> list[str]:
    """The Linux leg is no experimental one."""
    # One indentation, then a non-blank that is no `-` (the next leg): unambiguous, so a miss stays linear.
    if re.search(r"- name: linux\n(?:[ \t]+[^\s-][^\n]*\n)*?[ \t]+experimental: false\b", build):
        return []
    return [f"{job_name}: the linux leg is not experimental: false"]


def piped_listings(text: str) -> list[str]:
    return [f"line {number}: an archive listing is piped into grep -q"
            for number, line in enumerate(text.splitlines(), 1)
            if re.search(r"tar\s+-t\S*.*\|\s*grep\s+-\S*q", line)]


def release_problems(text: str) -> list[str]:
    publish = job(text, "publish")
    if not publish:
        return ["no publish job"]
    found = publish_guard(publish, "build-unix", ("check", "build"))
    build_unix = job(text, "build-unix")
    if not build_unix:
        found.append("no build-unix job")
    else:
        found += linux_required(build_unix, "build-unix")
    found += required_files(publish)
    if "delete-asset" not in publish:
        found.append("publish: a macOS file of an earlier cut is not taken off the draft (delete-asset)")
    return found + piped_listings(text)


def problems(text: str) -> list[str]:
    publish = job(text, "publish")
    if not publish:
        return ["no publish job"]
    found = publish_guard(publish, "build")
    build = job(text, "build")
    found += linux_required(build, "build") + required_files(publish)
    upload = re.search(r"^\s+name: (rolling-.*)$", build, re.M)
    if not upload or "needs.prepare.outputs.short" not in upload.group(1):
        found.append("build: the artifact name is not bound to needs.prepare.outputs.short")
    download = re.search(r"^\s+pattern: (rolling-.*)$", publish, re.M)
    if not download or "needs.prepare.outputs.short" not in download.group(1):
        found.append("publish: the artifact pattern is not bound to needs.prepare.outputs.short")
    return found + piped_listings(text)


def main() -> int:
    paths = sys.argv[1:] or [".github/workflows/rolling.yml", ".github/workflows/release.yml"]
    failed = unreadable = False
    for path in paths:
        try:
            with open(path, encoding="utf-8") as handle:
                text = handle.read()
        except OSError as error:
            print(f"check-rolling-workflow: cannot read {path}: {error}", file=sys.stderr)
            unreadable = True
            continue
        check = release_problems if os.path.basename(path) == "release.yml" else problems
        for problem in check(text):
            print(f"{path}: {problem}")
            failed = True
    return 2 if unreadable else 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
