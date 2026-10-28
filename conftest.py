"""Pytest configuration for the babel-rust lane (mirror root).

* `collect_ignore` drops the real `tests/upstream/messages` tree:
  `tests/messages` is a symlink to it so the unmodified upstream
  `tests.messages.*` imports resolve; ignoring the real path avoids double
  collection (fixture data stays ignored either way).
* Doctests in the `babel/` package are collected exactly like upstream
  (verbatim upstream root-conftest behavior), so doc examples run against
  both the Rust kernel and the pure-Python fallback.
"""

import os
import sys
from pathlib import Path

from _pytest.doctest import DoctestModule

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# The unmodified upstream messages tests import `tests.messages.*` (sdist
# layout). Here the graded copy lives at `tests/upstream/messages`; alias
# the name to the real tree so `__file__`-derived paths (chdir, relpath in
# the exact-file tests) resolve exactly like upstream. No test file is
# modified for this.
try:
    import tests.upstream.messages as _upstream_messages  # noqa: F401
    sys.modules.setdefault("tests.messages", _upstream_messages)
except ImportError:
    pass

collect_ignore = [
    # Verbatim upstream root-conftest behavior (uncollectable modules).
    'babel/messages/setuptools_frontend.py',
    'tests/upstream/messages/data',
]

babel_path = Path(__file__).parent / 'babel'


def _is_relative(p1: Path, p2: Path) -> bool:
    try:
        p1.relative_to(p2)
        return True
    except ValueError:
        return False


def pytest_collect_file(file_path: Path, parent):
    if _is_relative(file_path, babel_path) and file_path.suffix == '.py':
        return DoctestModule.from_parent(parent, path=file_path)
