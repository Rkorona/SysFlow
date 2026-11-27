#!/usr/bin/env python3
"""Inventory and lint the SysFlow structured-event producers.

The event protocol is **open-world** (`docs/protocol.md` §3.10, `docs/event-system.md`): module,
category and event_type are stable identifiers, not a closed enum, so adding an event must never
require a Kotlin/Room/APK change. This script is therefore **not** an allow-list. It is a drift and
format gate for the producers that live in this repository, and it keeps `docs/events-catalog.md`
in sync with them.

It scans:
  * `core/**/*.rs` for literal `NewSystemEvent::new(module, category, event_type, severity, title)`
    calls outside `#[cfg(test)]` blocks;
  * `module/**/*.sh` and `tools/**/*.sh` for `event emit --module .. --category .. --type ..`,
    joining shell line continuations first;
  * `app/**` read-only, to prove the APK never emits events itself (it is a client: it may call
    `event list`, never `event emit`).

Fields that are not string literals (variables, `format!`, shell `$VAR`) are never ignored: they are
listed in a "dynamic producers" section so a refactor cannot silently drop a producer from the
inventory.

Field limits (mirrored from `core/sysflow-core/src/event.rs::validate_token`, and documented for
producers in `docs/event-system.md` → "Machine-checked event vocabulary"):
  module, category  1..=64 ASCII chars from [A-Za-z0-9._-]
  event_type        1..=96 ASCII chars from [A-Za-z0-9._-]
  severity          one of info, notice, warning, error, critical

Usage:
  tools/check-events.py                  # regenerate docs/events-catalog.md
  tools/check-events.py --check          # CI: fail when the catalog is stale or a producer is invalid
  tools/check-events.py --root /tmp/tree # run against another tree (self-test)
  tools/check-events.py --check --strict-labels   # also fail on a missing zh label in EventCopy.kt

Exit codes: 0 ok, 1 violations / stale catalog, 2 usage or I/O error.
"""

from __future__ import annotations

import argparse
import difflib
import re
import sys
from pathlib import Path

CATALOG_REL = "docs/events-catalog.md"
COPY_REL = "app/src/main/kotlin/io/sysflow/feature/home/EventCopy.kt"

# Mirrored from core/sysflow-core/src/event.rs. Keep in sync with that file; docs/event-system.md
# states the same numbers for producers.
TOKEN_LIMITS = {"module": 64, "category": 64, "event_type": 96}
TOKEN_RE = re.compile(r"[A-Za-z0-9._-]+\Z")
SEVERITIES = ("info", "notice", "warning", "error", "critical")

EVENT_CALL_RE = re.compile(r"\bNewSystemEvent\s*::\s*new\s*\(")
CFG_TEST_RE = re.compile(r"#\[\s*cfg\(\s*test\b")
EVENT_EMIT_RE = re.compile(r"\bevent\s+emit\b")
SHELL_FLAG_RE = re.compile(
    r"--(module|category|type|severity)(?:=|\s+)(?:'([^']*)'|\"([^\"]*)\"|(\S+))"
)
APP_EMIT_RE = re.compile(r"\bevent\s+emit\b|\"event\"\s*,\s*\"emit\"|'event'\s*,\s*'emit'")
RUST_STR_RE = re.compile(r'(?:b?r)(#*)"(.*)"\1\Z', re.S)
LABEL_RE = re.compile(r'"([^"]+)"\s+to\s+"')

ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
SKIP_DIRS = {"target", "build", ".git", ".gradle", ".idea", "intermediates", "node_modules"}


# --------------------------------------------------------------------------------------------
# Small Rust-aware scanners: strings/chars/comments must not confuse bracket matching.
# --------------------------------------------------------------------------------------------


def _literal_end(text: str, i: int) -> int | None:
    """Skip one string/raw-string/char literal starting at i; None when i is not one."""
    n = len(text)
    c = text[i]
    if c == '"':
        j = i + 1
        while j < n:
            if text[j] == "\\":
                j += 2
                continue
            if text[j] == '"':
                return j + 1
            j += 1
        return n
    if c == "r" and i + 1 < n and text[i + 1] in '#"':
        j = i + 1
        hashes = 0
        while j < n and text[j] == "#":
            hashes += 1
            j += 1
        if j >= n or text[j] != '"':
            return None
        end = text.find('"' + "#" * hashes, j + 1)
        return n if end < 0 else end + hashes + 1
    if c == "'":
        j = i + 1
        if j < n and text[j] == "\\":
            j += 2
        else:
            j += 1
        if j < n and text[j] == "'":
            return j + 1
        return None  # lifetime, not a char literal
    return None


def _skip_noncode(text: str, i: int) -> int | None:
    """Return the end of a comment/literal at i, or None when i is plain code."""
    n = len(text)
    c = text[i]
    if c == "/" and i + 1 < n:
        if text[i + 1] == "/":
            nl = text.find("\n", i)
            return n if nl < 0 else nl
        if text[i + 1] == "*":
            end = text.find("*/", i + 2)
            return n if end < 0 else end + 2
        return None
    return _literal_end(text, i)


def _find_matching(text: str, i: int, open_ch: str, close_ch: str) -> int:
    """Index of the bracket closing text[i], or -1. Skips strings, chars and comments."""
    depth = 0
    n = len(text)
    j = i
    while j < n:
        skip = _skip_noncode(text, j)
        if skip is not None:
            j = skip
            continue
        c = text[j]
        if c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return j
        j += 1
    return -1


def _next_code(text: str, i: int) -> int:
    n = len(text)
    j = i
    while j < n:
        if text[j] in " \t\r\n":
            j += 1
            continue
        skip = _skip_noncode(text, j)
        if skip is not None:
            j = skip
            continue
        return j
    return n


def _split_args(text: str, start: int, end: int) -> list[tuple[int, str]]:
    """Split a call argument list at top-level commas; returns (offset, raw argument) pairs."""
    args: list[tuple[int, str]] = []
    depth = 0
    seg = start
    j = start
    while j < end:
        skip = _skip_noncode(text, j)
        if skip is not None:
            j = skip
            continue
        c = text[j]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            args.append((seg, text[seg:j]))
            seg = j + 1
        j += 1
    tail = text[seg:end]
    if tail.strip():
        args.append((seg, tail))
    return args


def _blank(out: list[str], start: int, end: int) -> None:
    for k in range(start, min(end, len(out))):
        if out[k] != "\n":
            out[k] = " "


def _mask_comments(text: str) -> str:
    """Blank // and /* */ comments, preserving offsets and newlines."""
    out = list(text)
    n = len(text)
    j = 0
    while j < n:
        if text[j] == "/" and j + 1 < n and text[j + 1] in "/*":
            end = _skip_noncode(text, j)
            if end is not None:
                _blank(out, j, end)
                j = end
                continue
        end = _literal_end(text, j)
        j = end if end is not None else j + 1
    return "".join(out)


def _mask_cfg_test(text: str) -> str:
    """Blank whole `#[cfg(test)]` items so test fixtures never enter the catalog."""
    out = list(text)
    n = len(text)
    pos = 0
    while True:
        m = CFG_TEST_RE.search(text, pos)
        if not m:
            break
        bracket = text.find("[", m.start())
        attr_end = _find_matching(text, bracket, "[", "]") if bracket >= 0 else -1
        if attr_end < 0:
            pos = m.end()
            continue
        j = _next_code(text, attr_end + 1)
        while j < n and text[j] not in "{;":
            skip = _skip_noncode(text, j)
            j = skip if skip is not None else j + 1
        if j >= n:
            break
        end = _find_matching(text, j, "{", "}") if text[j] == "{" else j
        if end < 0:
            pos = m.end()
            continue
        _blank(out, m.start(), end + 1)
        pos = end + 1
    return "".join(out)


def _parse_rust_str(expr: str) -> str | None:
    """Return the value of a pure string literal, or None for anything dynamic."""
    s = expr.strip()
    if not s:
        return None
    m = RUST_STR_RE.match(s)
    if m:
        return m.group(2)
    if s[0] != '"':
        return None
    end = _literal_end(s, 0)
    if end != len(s) or end < 2:
        return None
    body = s[1 : end - 1]
    if "\\" not in body:
        return body
    return _decode_escapes(body)


def _decode_escapes(body: str) -> str | None:
    out: list[str] = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        if i + 1 >= len(body):
            return None
        e = body[i + 1]
        if e in ESCAPES:
            out.append(ESCAPES[e])
            i += 2
        elif e == "x" and i + 3 < len(body):
            try:
                out.append(chr(int(body[i + 2 : i + 4], 16)))
            except ValueError:
                return None
            i += 4
        elif e == "u" and i + 2 < len(body) and body[i + 2] == "{":
            close = body.find("}", i + 3)
            if close < 0:
                return None
            try:
                out.append(chr(int(body[i + 3 : close], 16)))
            except ValueError:
                return None
            i = close + 1
        else:
            return None
    return "".join(out)


def _line(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _preview(expr: str) -> str:
    flat = " ".join(expr.split())
    return flat if len(flat) <= 80 else flat[:77] + "..."


def _validate_token(rel: str, line: int, name: str, value: str, violations: list) -> None:
    limit = TOKEN_LIMITS[name]
    if not value:
        violations.append((f"{rel}:{line}", name, "must not be empty"))
    elif len(value) > limit:
        violations.append(
            (f"{rel}:{line}", name, f"{value!r} is {len(value)} chars, limit is {limit}")
        )
    elif not TOKEN_RE.match(value):
        violations.append(
            (f"{rel}:{line}", name, f"{value!r} has chars outside [A-Za-z0-9._-]")
        )


# --------------------------------------------------------------------------------------------
# Producer scans
# --------------------------------------------------------------------------------------------


def _walk(root: Path, sub: str, suffix: str) -> list[tuple[Path, str]]:
    base = root / sub
    if not base.is_dir():
        return []
    found = []
    for path in sorted(base.rglob(f"*{suffix}")):
        rel_parts = path.relative_to(root).parts
        if any(part in SKIP_DIRS for part in rel_parts):
            continue
        if path.is_file():
            found.append((path, path.relative_to(root).as_posix()))
    return found


def scan_rust(root: Path, violations: list, events: list, dynamic: list) -> None:
    for path, rel in _walk(root, "core", ".rs"):
        text = path.read_text(encoding="utf-8", errors="replace")
        # Fast path: most files have no event producer at all, so never pay for masking them.
        if "NewSystemEvent" not in text:
            continue
        code = _mask_cfg_test(_mask_comments(text))
        for m in EVENT_CALL_RE.finditer(code):
            open_paren = m.end() - 1
            close = _find_matching(code, open_paren, "(", ")")
            if close < 0:
                violations.append(
                    (f"{rel}:{_line(code, m.start())}", "call", "unterminated NewSystemEvent::new(...)")
                )
                continue
            call_line = _line(code, m.start())
            args = _split_args(code, open_paren + 1, close)
            fields: dict[str, str] = {}
            unresolved: list[str] = []
            exprs: list[str] = []
            for idx, (offset, raw) in enumerate(args[:4]):
                name = ("module", "category", "event_type", "severity")[idx]
                value = _parse_rust_str(raw)
                if value is None:
                    if idx < 3:
                        unresolved.append(name)
                        exprs.append(_preview(raw))
                    continue
                # `_split_args` offsets are absolute, so use them directly for the arg's line.
                line = _line(code, offset)
                if idx < 3:
                    fields[name] = value
                    _validate_token(rel, line, name, value, violations)
                elif value not in SEVERITIES:
                    violations.append(
                        (f"{rel}:{line}", "severity", f"{value!r} is not one of {', '.join(SEVERITIES)}")
                    )
            if unresolved:
                dynamic.append(
                    {
                        "file": rel,
                        "line": call_line,
                        "unresolved": unresolved,
                        "known": fields,
                        "expr": "; ".join(exprs),
                    }
                )
            if all(k in fields for k in ("module", "category", "event_type")):
                events.append(
                    {
                        "module": fields["module"],
                        "category": fields["category"],
                        "event_type": fields["event_type"],
                        "producer": f"{rel}:{call_line}",
                        "kind": "rust",
                    }
                )


def _shell_logical_lines(text: str):
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        start = i
        buf = lines[i]
        while buf.rstrip().endswith("\\") and i + 1 < len(lines):
            buf = buf.rstrip()[:-1] + " " + lines[i + 1]
            i += 1
        yield start + 1, buf
        i += 1


def _shell_literal(flags: dict[str, str], name: str) -> str | None:
    """A flag value that is a static string literal; None for a missing or interpolated value."""
    value = flags.get(name)
    if not value or "$" in value or "`" in value:
        return None
    return value


def scan_shell(root: Path, violations: list, events: list, dynamic: list) -> None:
    for sub in ("module", "tools"):
        for path, rel in _walk(root, sub, ".sh"):
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError as exc:  # pragma: no cover - unreadable scripts are not silent
                violations.append((rel, "io", str(exc)))
                continue
            for line_no, logical in _shell_logical_lines(text):
                if not EVENT_EMIT_RE.search(logical):
                    continue
                flags = {}
                for fm in SHELL_FLAG_RE.finditer(logical):
                    flags.setdefault(fm.group(1), fm.group(2) or fm.group(3) or fm.group(4))
                unresolved: list[str] = []
                known: dict[str, str] = {}
                for name in ("module", "category", "type"):
                    value = _shell_literal(flags, name)
                    if value is None:
                        unresolved.append("event_type" if name == "type" else name)
                    else:
                        known[name if name != "type" else "event_type"] = value
                severity = _shell_literal(flags, "severity")
                if severity is not None and severity not in SEVERITIES:
                    violations.append(
                        (f"{rel}:{line_no}", "severity", f"{severity!r} is not one of {', '.join(SEVERITIES)}")
                    )
                for name in ("module", "category"):
                    if name in known:
                        _validate_token(rel, line_no, name, known[name], violations)
                if "event_type" in known:
                    _validate_token(rel, line_no, "event_type", known["event_type"], violations)
                if unresolved:
                    dynamic.append(
                        {
                            "file": rel,
                            "line": line_no,
                            "unresolved": unresolved,
                            "known": known,
                            "expr": _preview(logical),
                        }
                    )
                elif all(k in known for k in ("module", "category", "event_type")):
                    events.append(
                        {
                            "module": known["module"],
                            "category": known["category"],
                            "event_type": known["event_type"],
                            "producer": f"{rel}:{line_no}",
                            "kind": "shell",
                        }
                    )


def scan_app(root: Path, violations: list) -> None:
    for path, rel in _walk(root, "app", ".kt"):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError as exc:  # pragma: no cover
            violations.append((rel, "io", str(exc)))
            continue
        for m in APP_EMIT_RE.finditer(text):
            violations.append(
                (
                    f"{rel}:{_line(text, m.start())}",
                    "app-emit",
                    "the APK is a client: it may read events (`event list`), never emit them",
                )
            )


# --------------------------------------------------------------------------------------------
# Catalog and label report
# --------------------------------------------------------------------------------------------


def _cell(value: str) -> str:
    return " ".join(value.split()).replace("|", "\\|")


def build_catalog(events: list, dynamic: list) -> str:
    triples: dict[tuple[str, str, str], set[str]] = {}
    for ev in events:
        triples.setdefault((ev["module"], ev["category"], ev["event_type"]), set()).add(
            ev["producer"]
        )
    rust = sum(1 for ev in events if ev["kind"] == "rust")
    shell = len(events) - rust
    lines = [
        "# SysFlow event catalog",
        "",
        "<!-- Generated by `tools/check-events.py`; do not edit by hand. -->",
        "",
        "The event protocol is **open-world** (see `docs/protocol.md` §3.10 and",
        "`docs/event-system.md`): this file is an inventory of the producers in this repository, not",
        "an allow-list. A new module or event type is always allowed — write the producer, run",
        "`tools/check-events.py`, and commit the regenerated catalog.",
        "",
        f"- Producer occurrences: {len(events)} ({rust} Rust, {shell} shell),"
        f" {len(triples)} unique `module/category/event_type` triples",
        f"- Dynamic producers (not statically resolvable): {len(dynamic)}",
        "",
        "## Literal producers",
        "",
        "| module | category | event_type | producer |",
        "|---|---|---|---|",
    ]
    for module, category, event_type in sorted(triples):
        producers = "<br>".join(f"`{p}`" for p in sorted(triples[(module, category, event_type)]))
        lines.append(f"| `{module}` | `{category}` | `{event_type}` | {producers} |")
    lines.extend(["", "## Dynamic producers", ""])
    if not dynamic:
        lines.append("_None._")
    else:
        lines.append("Fields built at run time (variables, `format!`, shell `$VAR`) cannot be")
        lines.append("validated statically. They are listed here so no producer is silently missed.")
        lines.append("")
        lines.append("| file:line | unresolved field(s) | known literals | expression |")
        lines.append("|---|---|---|---|")
        for item in sorted(dynamic, key=lambda d: (d["file"], d["line"])):
            unresolved = ", ".join(f"`{f}`" for f in item["unresolved"])
            known = ", ".join(
                f"`{k}`=`{v}`" for k, v in sorted(item["known"].items())
            ) or "—"
            lines.append(
                f"| `{item['file']}:{item['line']}` | {unresolved} | {known} | "
                f"`{_cell(item['expr'])}` |"
            )
    lines.append("")
    return "\n".join(lines)


def scan_labels(root: Path) -> set[str] | None:
    path = root / COPY_REL
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    start = text.find("private val eventLabels = mapOf(")
    if start < 0:
        return None
    end = text.find("\n)", start)
    block = text[start : end if end >= 0 else len(text)]
    return set(LABEL_RE.findall(block))


def report_lines(events: list, dynamic: list, violations: list) -> tuple[list[str], list[str]]:
    triples = {(ev["module"], ev["category"], ev["event_type"]) for ev in events}
    rust = sum(1 for ev in events if ev["kind"] == "rust")
    out = [
        f"producers: {len(events)} literal occurrences ({rust} Rust, {len(events) - rust} shell),"
        f" {len(triples)} unique triples, {len(dynamic)} dynamic call sites"
    ]
    errs = []
    for location, field, message in violations:
        errs.append(f"{location}: {field}: {message}")
    return out, errs


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Inventory and lint SysFlow event producers.")
    parser.add_argument("--root", help="tree to scan (default: the repo this script lives in)")
    parser.add_argument("--check", action="store_true", help="fail when the catalog is stale")
    parser.add_argument(
        "--strict-labels",
        action="store_true",
        help="also fail when EventCopy.kt has no Chinese label for a known event_type",
    )
    args = parser.parse_args(argv)

    root = Path(args.root).resolve() if args.root else Path(__file__).resolve().parent.parent
    if not (root / "core").is_dir():
        print(f"error: {root} does not look like the SysFlow tree (no core/)", file=sys.stderr)
        return 2

    violations: list = []
    events: list = []
    dynamic: list = []

    scan_rust(root, violations, events, dynamic)
    scan_shell(root, violations, events, dynamic)
    scan_app(root, violations)

    catalog = build_catalog(events, dynamic)
    catalog_path = root / CATALOG_REL

    stale = False
    if args.check:
        current = catalog_path.read_text(encoding="utf-8") if catalog_path.is_file() else None
        if current != catalog:
            stale = True
            if current is None:
                print(f"error: {CATALOG_REL} is missing", file=sys.stderr)
            else:
                print(f"error: {CATALOG_REL} is stale", file=sys.stderr)
                diff = difflib.unified_diff(
                    current.splitlines(),
                    catalog.splitlines(),
                    fromfile="on disk",
                    tofile="generated",
                    lineterm="",
                    n=1,
                )
                for idx, line in enumerate(diff):
                    if idx >= 40:
                        print("... (diff truncated)", file=sys.stderr)
                        break
                    print(line, file=sys.stderr)
    else:
        catalog_path.parent.mkdir(parents=True, exist_ok=True)
        old = catalog_path.read_text(encoding="utf-8") if catalog_path.is_file() else None
        if old != catalog:
            catalog_path.write_text(catalog, encoding="utf-8")
            print(f"wrote {CATALOG_REL}")
        else:
            print(f"{CATALOG_REL} already up to date")

    lines, errors = report_lines(events, dynamic, violations)
    for line in lines:
        print(line)

    labels = scan_labels(root)
    known_types = sorted({ev["event_type"] for ev in events})
    if labels is None:
        print(f"labels: {COPY_REL} not found; skipped the zh-label report")
        missing: list[str] = []
    else:
        missing = [t for t in known_types if t not in labels]
        print(f"labels: EventCopy.kt has {len(labels)} zh labels; missing: {len(missing)}")
        for name in missing:
            print(f"  missing label: {name}")
        if missing:
            print("  (report only: the event protocol is open-world and unknown events fall back"
                  " to the producer's own title/message)")

    for err in errors:
        print(f"error: {err}", file=sys.stderr)

    failed = bool(violations) or stale
    if args.strict_labels and missing:
        print(
            f"error: {len(missing)} event type(s) have no Chinese label in EventCopy.kt"
            " (--strict-labels)",
            file=sys.stderr,
        )
        failed = True
    if failed:
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
