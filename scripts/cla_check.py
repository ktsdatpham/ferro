#!/usr/bin/env python3
"""CLA check for pull requests (ADR 0010).

Passes when the pull request's author is listed under "Exempt" in
docs/legal/CLA.md, or when the description contains the CLA line. Reads
AUTHOR and BODY from the environment. Exit 1 with a hint otherwise.
"""
import os
import re
import sys

LINE = "I have read the Ferro CLA (docs/legal/CLA.md) and I agree to it."


def exempt(cla_text: str) -> list:
    tail = cla_text.split("## Exempt", 1)[-1]
    return re.findall(r"^- (\S+)\s*$", tail, re.M)


def ok(author: str, body: str, cla_text: str) -> bool:
    return author in exempt(cla_text) or LINE in (body or "")


def main() -> int:
    here = os.path.dirname(os.path.abspath(__file__))
    path = os.path.join(here, "..", "docs", "legal", "CLA.md")
    with open(path, encoding="utf-8") as f:
        text = f.read()
    author = os.environ.get("AUTHOR", "")
    if ok(author, os.environ.get("BODY", ""), text):
        print(f"CLA ok for {author}")
        return 0
    print("Missing CLA line. Add this to the pull request description:")
    print(f"{LINE} - <your GitHub login>")
    return 1


if __name__ == "__main__":
    sys.exit(main())
