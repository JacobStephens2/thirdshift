import json, os, pathlib, re, subprocess, tempfile, urllib.request
binary = str(pathlib.Path(os.environ.get('THIRDSHIFT_BINARY', 'target/debug/thirdshift')).resolve())
other = 'https://other.example.test'
with tempfile.TemporaryDirectory(prefix='console-header-review-') as home:
    root = pathlib.Path(home) / '.thirdshift'
    root.mkdir()
    (root / 'config.toml').write_text('[console]\nport = 0\nother_consoles = ["' + other + '"]\n')
    env = dict(os.environ, HOME=home)
    hostname = json.loads(subprocess.check_output([binary, 'status', '--json'], env=env))['hostname']
    server = subprocess.Popen([binary, 'console', '--no-sign-in'], env=env, stdout=subprocess.PIPE, text=True)
    try:
        origin = server.stdout.readline().strip()
        missing = []
        for page in ['inbox', 'lanes', 'stages', 'waterfall', 'history', 'logs', 'security']:
            html = urllib.request.urlopen(origin + '/?page=' + page, timeout=5).read().decode()
            header = re.search(r'<header\b[^>]*>(.*?)</header>', html, re.S).group(1)
            if hostname not in header or 'href="' + other + '"' not in header:
                missing.append(page)
        assert not missing, 'Every Console page header should name this machine and link other Consoles; missing on: ' + ', '.join(missing)
    finally:
        server.terminate(); server.wait(timeout=5)
