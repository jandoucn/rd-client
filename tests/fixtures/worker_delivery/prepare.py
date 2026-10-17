"""Seed the isolated dependency lock from the root's pinned revisions."""
from pathlib import Path
import shutil

here = Path(__file__).resolve().parent
lock = here / 'Cargo.lock'
if lock.exists():
    raise SystemExit('已有隔离锁，拒绝覆盖')
shutil.copyfile(here.parents[2] / 'Cargo.lock', lock)
