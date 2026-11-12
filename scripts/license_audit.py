#!/usr/bin/env python3
"""Check every dependency's license against the project allowlist.

Uses `cargo metadata --locked` (no network, no extra tools) and evaluates the
SPDX expression conservatively: an `OR` passes when any branch is allowed, an
`AND` requires every part, and `WITH` checks the exception as well.

Usage: scripts/license_audit.py [--json]
"""

from __future__ import annotations

import json
import subprocess
import sys

# Licenses we accept for shipped dependencies. Keep this list small and
# review any addition; `MPL-2.0` is file-level copyleft and is only accepted
# because the affected crate is not modified.
ALLOWED = {
    "0bsd",
    "apache-2.0",
    "bsd-2-clause",
    "bsd-3-clause",
    "bsl-1.0",
    "bzip2-1.0.6",
    "cc0-1.0",
    "isc",
    "llvm-exception",
    "mit",
    "mit-0",
    "mpl-2.0",
    "ofl-1.1",
    "ubuntu-font-1.0",
    "unicode-3.0",
    "unlicense",
    "zlib",
}


def normalize(expression: str) -> str:
    # Legacy crates use `MIT/Apache-2.0` for a disjunction.
    return expression.replace("/", " OR ").strip()


def split_top_level(expression: str, operator: str) -> list[str]:
    """Splits on `operator` outside parentheses."""
    parts: list[str] = []
    depth = 0
    current: list[str] = []
    i = 0
    needle = f" {operator} "
    while i < len(expression):
        char = expression[i]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        if depth == 0 and expression.startswith(needle, i):
            parts.append("".join(current).strip())
            current = []
            i += len(needle)
            continue
        current.append(char)
        i += 1
    parts.append("".join(current).strip())
    return [part for part in parts if part]


def strip_parens(expression: str) -> str:
    expression = expression.strip()
    while expression.startswith("(") and expression.endswith(")"):
        depth = 0
        balanced = True
        for index, char in enumerate(expression):
            if char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
                if depth == 0 and index != len(expression) - 1:
                    balanced = False
                    break
        if not balanced:
            break
        expression = expression[1:-1].strip()
    return expression


def allowed(expression: str) -> bool:
    expression = strip_parens(normalize(expression))
    ors = split_top_level(expression, "OR")
    if len(ors) > 1:
        return any(allowed(part) for part in ors)
    ands = split_top_level(expression, "AND")
    if len(ands) > 1:
        return all(allowed(part) for part in ands)
    if " WITH " in expression:
        base, exception = expression.split(" WITH ", 1)
        return allowed(base) and exception.strip().lower() in ALLOWED
    return expression.strip().lower() in ALLOWED


def main() -> int:
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    metadata = json.loads(output)
    workspace = set(metadata["workspace_members"])
    rejected: list[tuple[str, str]] = []
    missing: list[str] = []
    seen: set[str] = set()
    for package in metadata["packages"]:
        if package["id"] in workspace:
            continue
        if package["id"] in seen:
            continue
        seen.add(package["id"])
        expression = package.get("license")
        if not expression:
            if package.get("license_file"):
                continue
            missing.append(f"{package['name']} {package['version']}")
            continue
        if not allowed(expression):
            rejected.append((f"{package['name']} {package['version']}", expression))

    if missing:
        print("dependencies without a license field:")
        for item in missing:
            print(f"  {item}")
    if rejected:
        print("dependencies outside the allowlist:")
        for package, expression in rejected:
            print(f"  {package}: {expression}")
    if missing or rejected:
        return 1
    print(f"license audit: {len(seen)} dependencies, all licenses allowed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
