"""Print the Console's Needs you rows from its overview page, read on stdin.

Usage: curl -fsS --cookie "$CONSOLE_COOKIE_FILE" "$CONSOLE_URL/?section=overview" |
       python3 needs-you.py "$CONSOLE_URL"
"""
from html.parser import HTMLParser
import re
import sys


class Inbox(HTMLParser):
    """Read row identities from the Fleet page's semantic data attributes."""

    def __init__(self):
        super().__init__()
        self.groups = []
        self.found = False
        self.row = None
        self.row_depth = 0
        self.rows = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "details":
            group = attrs.get("data-group", self.groups[-1] if self.groups else "")
            self.groups.append(group)
            self.found |= group == "needs you"
        if tag == "li":
            if self.row is not None:
                self.row_depth += 1
            elif self.groups and self.groups[-1] == "needs you" and attrs.get("data-command"):
                self.row = (attrs.get("data-machine", ""), attrs["data-command"], [])
                self.row_depth = 1

    def handle_data(self, data):
        if self.row is not None:
            self.row[2].append(data)

    def handle_endtag(self, tag):
        if tag == "details" and self.groups:
            self.groups.pop()
        if tag == "li" and self.row is not None:
            self.row_depth -= 1
            if self.row_depth == 0:
                self.rows.append(self.row)
                self.row = None


if __name__ == "__main__":
    inbox = Inbox()
    inbox.feed(sys.stdin.read())
    inbox.close()
    if not inbox.found or inbox.row is not None:
        url = sys.argv[1] if len(sys.argv) > 1 else "its URL"
        print(f"console unreadable at {url}: expected Inbox; check connectivity and sign-in cookies", file=sys.stderr)
        sys.exit(1)
    print(len(inbox.rows), "rows")
    for machine, selector, fragments in inbox.rows:
        text = re.sub(r"\s+", " ", " ".join(fragments)).strip()[:200]
        print(f" - {machine or 'Console host'} [{selector}] {text}")
