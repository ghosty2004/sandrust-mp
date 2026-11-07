#!/usr/bin/env python3
"""Proves that a change to Rust sources touched only comments and whitespace.

  tools/check-code-only.py BASE [HEAD]      (git revisions; HEAD defaults to the working tree)

For every .rs file that differs between BASE and HEAD, the comments (`//`, `///`, `//!`, nested `/* */`)
are removed and whitespace runs are collapsed, string / char / raw-string literals kept verbatim; the
remaining code must be identical. Added, deleted or renamed .rs files fail the check. Exit 0 when every
changed file is comment/whitespace-only, 1 otherwise (each offending file is listed).
"""
import subprocess
import sys


def strip(src):
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            out.append(" ")
            continue
        if c == "r" and i + 1 < n and src[i + 1] in '#"' and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            j = i + 1
            while j < n and src[j] == "#":
                j += 1
            if j < n and src[j] == '"':
                hashes = j - i - 1
                end = src.find('"' + "#" * hashes, j + 1)
                end = n if end < 0 else end + 1 + hashes
                out.append(src[i:end])
                i = end
                continue
        if c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append(src[i:j + 1])
            i = j + 1
            continue
        if c == "'":
            # char literal ('a', '\n', '\u{..}') vs lifetime ('a): a literal closes within a few chars
            j = src.find("'", i + 1, i + 12)
            body = src[i + 1:j] if j > 0 else ""
            if j > 0 and (len(body) == 1 or body.startswith("\\")):
                out.append(src[i:j + 1])
                i = j + 1
                continue
        if c.isspace():
            if out and out[-1] != " ":
                out.append(" ")
            i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out).strip()


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def main():
    base = sys.argv[1]
    head = sys.argv[2] if len(sys.argv) > 2 else None
    rng = [base, head] if head else [base]
    changes = [l.split("\t") for l in git("diff", "--name-status", "-M", *rng, "--", "*.rs").splitlines()]
    bad, ok = [], 0
    for ch in changes:
        status, path = ch[0], ch[-1]
        if status != "M":
            bad.append(f"{status} {' -> '.join(ch[1:])} (not a modification)")
            continue
        old = git("show", f"{base}:{path}")
        new = git("show", f"{head}:{path}") if head else open(path, encoding="utf-8").read()
        if strip(old) == strip(new):
            ok += 1
        else:
            a, b = strip(old), strip(new)
            k = next((x for x in range(min(len(a), len(b))) if a[x] != b[x]), min(len(a), len(b)))
            bad.append(f"{path}: code differs near …{a[max(0, k - 60):k + 60]!r} vs …{b[max(0, k - 60):k + 60]!r}")
    print(f"comment/whitespace-only: {ok} file(s); code changes: {len(bad)}")
    for b in bad:
        print("  " + b)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
