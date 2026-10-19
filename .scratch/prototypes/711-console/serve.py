"""PROTOTYPE: serve this folder on :8711 with caching off, so a plain reload always shows the latest files."""
import functools, http.server, pathlib

class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

handler = functools.partial(NoCache, directory=str(pathlib.Path(__file__).parent))
print("Console prototype: http://localhost:8711/?variant=garden")
http.server.ThreadingHTTPServer(("", 8711), handler).serve_forever()
