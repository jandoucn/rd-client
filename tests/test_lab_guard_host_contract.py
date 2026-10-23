"""Repository contract: experimental/fault-injection tests gate on RUSTDESK_LAB_HOST only.

The destructive acceptance suites used to hard-code the lab machine names and the
VirtualBox SMBIOS UUID of each guest. That made every run impossible on the
physical Win10 host (``YAN``) even though it is reachable as SYSTEM there. The
guard is now: ``RUSTDESK_LAB_HOST`` must be non-empty and equal to the current
``COMPUTERNAME``. Machine names and VM UUIDs must not come back.
"""
import pathlib
import re
import subprocess
import unittest

REPO = pathlib.Path(__file__).resolve().parents[1]
# scripts/ holds the guest-side acceptance and fault-injection drivers that now run
# on win10-gs; they must use the same RUSTDESK_LAB_HOST gate as the Rust suites.
SOURCE_ROOTS = ('src', 'tests', 'libs', 'scripts')
EXTRA_SOURCES = ()
SOURCE_SUFFIXES = ('.rs', '.ps1', '.cpp', '.h', '.py', '.cmd', '.bat')
DOC_SUFFIXES = ('.zh.md', '.md')
# The guard contract test itself has to name the forbidden literals to ban them.
GUARD_TEST_NAME = 'test_lab_guard_host_contract.py'


def tracked_files():
    listed = subprocess.run(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=REPO,
        capture_output=True,
        check=True,
    ).stdout.decode('utf-8')
    return [p for p in listed.split('\0') if p]


def files_with_suffix(suffixes):
    for relative in tracked_files():
        path = REPO / relative
        in_scope = relative in EXTRA_SOURCES or any(
            relative.startswith(root + '/') for root in SOURCE_ROOTS
        )
        if in_scope and relative.endswith(suffixes) and path.is_file():
            yield relative, path


def matching_lines(path, patterns):
    hits = []
    text = path.read_text(encoding='utf-8')
    for number, line in enumerate(text.splitlines(), start=1):
        for pattern in patterns:
            if re.search(pattern, line):
                hits.append((number, pattern, line.strip()))
                break
    return hits


FORBIDDEN = [
    r'RDLAB-WIN10',
    r'RDLAB-WIN11',
    r'RDLAB-\*',
    r'd91a13d9-2bb7-40bf-b5e4-9d57870cd242',
    r'a6c9edff-6159-468f-bdba-08720a153424',
]


class SourceGuardContractTest(unittest.TestCase):
    def test_no_hard_coded_lab_machine_names_or_vm_uuid_in_code(self):
        failures = []
        for relative, path in files_with_suffix(SOURCE_SUFFIXES):
            if path.name == GUARD_TEST_NAME:
                continue
            for number, pattern, line in matching_lines(path, FORBIDDEN):
                failures.append(f'{relative}:{number}: {pattern} -> {line}')
        self.assertEqual(failures, [], 'hard-coded lab identity is back:\n' + '\n'.join(failures))

    def test_no_vm_uuid_guard_survives_in_docs_run_instructions(self):
        forbidden_doc = [
            r'(?i)COMPUTERNAME\s*=?\s*`?RDLAB',
            r'(?i)COMPUTERNAME[^\n]{0,40}RDLAB',
            r'(?i)UUID[^\n]{0,40}(d91a13d9|a6c9edff)',
        ]
        failures = []
        for relative, path in files_with_suffix(DOC_SUFFIXES):
            for number, pattern, line in matching_lines(path, forbidden_doc):
                failures.append(f'{relative}:{number}: {pattern} -> {line}')
        self.assertEqual(failures, [], 'docs still instruct running on a fixed VM identity:\n' + '\n'.join(failures))

    def test_unified_guard_exists_and_uses_lab_host_env(self):
        guard = REPO / 'libs/base/src/lab_guard.rs'
        self.assertTrue(guard.is_file(), 'missing unified guard libs/base/src/lab_guard.rs')
        text = guard.read_text(encoding='utf-8')
        self.assertIn('RUSTDESK_LAB_HOST', text)
        self.assertIn('COMPUTERNAME', text)
        self.assertIn('RUSTDESK_LAB_OS', text)
        self.assertNotIn('ComputerSystemProduct', text)

    def test_cpp_fixture_reads_the_same_environment_variable(self):
        text = (REPO / 'tests/msi_desktop_shortcut.cpp').read_text(encoding='utf-8')
        self.assertIn('L"RUSTDESK_LAB_HOST"', text)
        self.assertIn('_wcsicmp', text)

    def test_scripts_gate_on_the_same_environment_variable(self):
        literal_compare = r"(?i)\$env:COMPUTERNAME\s+-(c?ne|c?eq|notin|in|notlike|like)\s+['@(]"
        failures = []
        for relative, path in files_with_suffix(('.ps1',)):
            for number, _, line in matching_lines(path, [literal_compare]):
                failures.append(f'{relative}:{number}: {line}')
        self.assertEqual(failures, [], 'PowerShell guards still compare COMPUTERNAME to a literal:\n' + '\n'.join(failures))

    def test_script_computername_guards_require_lab_host_env(self):
        guard_on_computer = r'(?i)^\s*if\s*\(.*\$env:COMPUTERNAME'
        failures = []
        for relative, path in files_with_suffix(('.ps1',)):
            if not relative.startswith('scripts/'):
                continue
            for number, _, line in matching_lines(path, [guard_on_computer]):
                if 'RUSTDESK_LAB_HOST' not in line:
                    failures.append(f'{relative}:{number}: {line}')
        self.assertEqual(failures, [], 'script guards on COMPUTERNAME without RUSTDESK_LAB_HOST:\n' + '\n'.join(failures))

    def test_scripts_are_actually_scanned(self):
        scanned = [r for r, _ in files_with_suffix(SOURCE_SUFFIXES) if r.startswith('scripts/')]
        self.assertIn('scripts/guest-agent-acceptance-common.ps1', scanned)
        self.assertIn('scripts/guest-fault-matrix.ps1', scanned)
        self.assertGreaterEqual(len(scanned), 30)


if __name__ == '__main__':
    unittest.main()
