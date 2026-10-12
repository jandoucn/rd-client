import argparse
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if not args.output.parent.is_dir():
    parser.error('output parent must exist')
source = args.source.read_text(encoding='utf-8')
delimiter = 'RDDEFENDER'
if ')' + delimiter + '"' in source:
    parser.error('raw string delimiter conflicts')
chunks = [source[index:index + 6000] for index in range(0, len(source), 6000)]
args.output.write_text(
    ''.join('LR"' + delimiter + '(' + chunk + ')' + delimiter + '"\n' for chunk in chunks),
    encoding='utf-8',
)
