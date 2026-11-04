"""Release notes from the commits since the last release: user-facing commits grouped by area, tooling and merges left out. A hand-written `packaging/release/<version>.md` goes on top as the highlights."""
import os
import re
import subprocess
import sys

USER_FACING = ("src/", "assets/", "packaging/", "Cargo.toml")
INTERNAL = re.compile(r"doc comments?", re.I)
SKIP = re.compile(r"^(Merge |Bump version|herd\b|Drive\b|Workflow\b|Test\b|Tests\b|CI\b|gate\b|Agent tools\b)", re.I)


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True, encoding="utf-8").stdout


def commits(since, until):
    span = f"{since}..{until}" if since else until
    out = git("log", "--no-merges", "--reverse", "--format=%x1e%s", "--name-only", span)
    for block in out.split("\x1e")[1:]:
        lines = [l for l in block.splitlines() if l.strip()]
        if lines:
            yield lines[0].strip(), lines[1:]


def split(subject):
    area, sep, rest = subject.partition(": ")
    if sep and len(area.split()) <= 3 and not area.endswith("."):
        area = re.sub(r"\s+(review fixes|review|fixes)$", "", area.strip(), flags=re.I)
        return area[:1].upper() + area[1:], rest.strip()
    return "", subject.strip()


def sentence(text):
    text = text.strip().rstrip(".")
    return text[:1].upper() + text[1:]


def main():
    version, since, until = sys.argv[1], sys.argv[2] or None, sys.argv[3] if len(sys.argv) > 3 else "HEAD"
    repo = sys.argv[4] if len(sys.argv) > 4 else ""
    groups = {}
    for subject, files in commits(since, until):
        if SKIP.match(subject) or INTERNAL.search(subject) or not any(f.startswith(USER_FACING) for f in files):
            continue
        area, text = split(subject)
        groups.setdefault(area or "General", []).append(text)
    lines = [f"## Schrumbi {version}", ""]
    highlights = os.path.join("packaging", "release", f"{version}.md")
    if os.path.exists(highlights):
        lines += [open(highlights, encoding="utf-8").read().strip(), "", "## All changes", ""]
    if not groups:
        lines += ["Small fixes and maintenance.", ""]
    general = groups.pop("General", [])
    for area, items in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0].lower())):
        lines += [f"### {area}", ""]
        lines += [f"- {sentence(i)}" for i in items]
        lines.append("")
    if general:
        lines += ["### More", ""] + [f"- {sentence(i)}" for i in general] + [""]
    lines += ["### Install", "", "- macOS: open the `.dmg`, drag Schrumbi to Applications.", "- Windows: run `Schrumbi-Setup-…-windows-x64.exe`.", "- Linux: `.deb` or `.AppImage`.", "- Already installed: Schrumbi offers the update by itself.", ""]
    if since and repo:
        lines.append(f"All commits: https://github.com/{repo}/compare/{since}...v{version}")
    print("\n".join(lines).rstrip() + "\n")


if __name__ == "__main__":
    main()
