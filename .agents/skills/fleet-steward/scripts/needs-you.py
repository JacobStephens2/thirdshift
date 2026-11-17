"""Print the Console's Needs you rows from its overview page, read on stdin.

Usage: curl -s "$CONSOLE_URL/?section=overview" | python3 needs-you.py "$CONSOLE_URL"
"""
import html
import re
import sys

page = sys.stdin.read()
if not page:
    print(f"console unreachable at {sys.argv[1] if len(sys.argv) > 1 else 'its URL'}")
    sys.exit()

groups = [(m.start(), m.group(1)) for m in re.finditer(r'class="group" data-group="([^"]+)"', page)]
rows = []
for (start, name), (end, _) in zip(groups, groups[1:] + [(len(page), "")]):
    if name != "needs you":
        continue
    for row in re.split(r'class="row-head"', page[start:end])[1:]:
        text = html.unescape(re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", row))).strip()[2:200]
        if not text.startswith(("ended ", "live ")):
            rows.append(text)

print(len(rows), "rows")
for text in rows:
    print(" -", text)
