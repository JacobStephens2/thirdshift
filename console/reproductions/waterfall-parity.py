import pathlib, re

source = (pathlib.Path(__file__).resolve().parents[2] / 'src/console.rs').read_text()
declaration = re.search(r'PageRoute\s*\{\s*path:\s*"/waterfall",(?P<fields>.*?)\n\s*\}', source, re.S)
assert declaration, 'Waterfall route must be registered in the shared parity table'
fields = declaration.group('fields')
print('Waterfall parity fields:', fields.strip(), flush=True)
assert re.search(r'thirdshift logs\b', fields), 'Waterfall parity declaration must include its logs CLI dependency alongside show --json'
