import ast
import re
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
BUILD_PY = REPO_ROOT / "build.py"
WINDOWS_CC = REPO_ROOT / "src/platform/windows.cc"


def _extract_function_source(path: Path, name: str) -> str:
    source = path.read_text(encoding="utf-8")
    tree = ast.parse(source)
    for node in tree.body:
        if isinstance(node, ast.FunctionDef) and node.name == name:
            segment = ast.get_source_segment(source, node)
            if segment is None:
                break
            return segment
    raise AssertionError(f"{path.name} 中找不到函数 {name}")


def _extract_launch_process_win() -> str:
    source = WINDOWS_CC.read_text(encoding="utf-8")
    start = source.index("HANDLE LaunchProcessWin(")
    body_start = source.index("{", start)
    depth = 0
    for index in range(body_start, len(source)):
        char = source[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[start:index + 1]
    raise AssertionError("LaunchProcessWin 函数体未找到闭合花括号")


class _FakeOS:
    def __init__(self, environ):
        self.environ = environ
        self.chdir_calls = []

    def chdir(self, path):
        self.chdir_calls.append(path)


class _FakeShutil:
    def __init__(self):
        self.copy_calls = []

    def copy2(self, src, dst):
        self.copy_calls.append((src, dst))


def _run_windows_flutter_build(environ):
    function_source = _extract_function_source(BUILD_PY, "build_flutter_windows")
    commands = []
    namespace = {
        "os": _FakeOS(dict(environ)),
        "shutil": _FakeShutil(),
        "skip_cargo": True,
        "flutter_build_dir_2": "flutter/build/windows/x64/runner/Release",
        "system2": commands.append,
    }
    exec(compile(function_source, str(BUILD_PY), "exec"), namespace)
    namespace["build_flutter_windows"]("1.5.4", "flutter", True)
    return commands


class WindowsFlutterVersionArgsTest(unittest.TestCase):
    def test_windows_build_passes_version_and_build_number(self) -> None:
        commands = _run_windows_flutter_build(
            {"VERSION": "1.5.4", "BUILD_SEQ": "2026102405"})
        flutter_commands = [c for c in commands if c.startswith("flutter build windows")]

        self.assertEqual(len(flutter_commands), 1)
        command = flutter_commands[0]
        self.assertIn('--build-name "1.5.4"', command)
        self.assertIn('--build-number "2026102405"', command)
        self.assertNotEqual(command, "flutter build windows --release")
        self.assertLess(command.index("--build-name"), command.index("--build-number"))

    def test_windows_build_without_env_adds_no_empty_args(self) -> None:
        for environ in ({}, {"VERSION": "1.5.4"}, {"BUILD_SEQ": "2026102405"}):
            with self.subTest(environ=environ):
                commands = _run_windows_flutter_build(environ)
                flutter_commands = [
                    c for c in commands if c.startswith("flutter build windows")]
                self.assertEqual(flutter_commands, ["flutter build windows --release"])

    def test_windows_build_source_has_no_bare_release_command(self) -> None:
        function_source = _extract_function_source(BUILD_PY, "build_flutter_windows")

        self.assertNotIn("'flutter build windows --release'", function_source)
        self.assertIn("flutter_version_args", function_source)
        self.assertIn("--build-name", function_source)
        self.assertIn("--build-number", function_source)

    def test_macos_build_keeps_existing_version_argument_block(self) -> None:
        function_source = _extract_function_source(BUILD_PY, "build_flutter_dmg")

        self.assertIn("flutter_version_args = ''", function_source)
        self.assertIn(
            "if os.environ.get('VERSION') and os.environ.get('BUILD_SEQ'):",
            function_source)
        self.assertIn('f\' --build-name "{os.environ["VERSION"]}"\'', function_source)
        self.assertIn('f\' --build-number "{os.environ["BUILD_SEQ"]}"\'', function_source)
        self.assertIn(
            'f\'flutter build macos --release{flutter_version_args}\')', function_source)


class LaunchProcessWinBufferTest(unittest.TestCase):
    def test_uses_wcslen_sized_buffer_instead_of_max_path(self) -> None:
        body = _extract_launch_process_win()

        self.assertNotIn("wchar_t buf[MAX_PATH]", body)
        self.assertNotIn("wcscpy_s(buf, MAX_PATH", body)
        self.assertIn("wcslen(cmd)", body)

    def test_buffer_is_allocated_from_command_length(self) -> None:
        body = _extract_launch_process_win()

        self.assertRegex(body, r"std::vector<wchar_t>\s+buf\s*\(\s*cmd\s*,")
        self.assertIn("buf.data()", body)

    def test_null_command_is_guarded_before_dereference(self) -> None:
        body = _extract_launch_process_win()

        guard = re.search(r"if\s*\(\s*!\s*cmd\s*\)", body)
        self.assertIsNotNone(guard, "LaunchProcessWin 缺少 cmd 空指针保护")
        self.assertLess(guard.start(), body.index("wcslen(cmd)"))

    def test_startup_behaviour_is_preserved(self) -> None:
        body = _extract_launch_process_win()

        self.assertIn(
            "HANDLE LaunchProcessWin(LPCWSTR cmd, DWORD dwSessionId, BOOL as_user, "
            "BOOL show, DWORD *pDwTokenPid)",
            body)
        self.assertIn("si.lpDesktop = (LPWSTR)L\"winsta0\\\\default\"", body)
        self.assertIn("si.wShowWindow = SW_SHOW", body)
        self.assertIn("DWORD dwCreationFlags = DETACHED_PROCESS;", body)
        self.assertIn("dwCreationFlags |= CREATE_UNICODE_ENVIRONMENT;", body)
        self.assertIn("CreateProcessAsUserW(hToken, NULL, buf.data()", body)

    def test_vector_header_is_available(self) -> None:
        source = WINDOWS_CC.read_text(encoding="utf-8")

        self.assertIn("#include <vector>", source)


if __name__ == "__main__":
    unittest.main()
