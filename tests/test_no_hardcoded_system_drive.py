"""生产代码里不能再写死系统盘。

系统盘不一定是 C 盘：系统盘在 D 盘的机器上，Program Files、ProgramData 和 Windows 目录都在 D 盘，
写死 `C:\\` 会让服务校验、救援、修复和升级预检全部失效。路径统一从
`src/platform/windows_system_dirs.rs` 向 Windows 查询；这里扫描所有生产 Rust 代码（跳过测试代码和
注释），发现还有 `C:` 开头的路径字面量就失败。
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCANNED = [ROOT / "src", ROOT / "libs" / "base" / "src"]
# 唯一允许出现旧写法的地方：系统目录模块自身的回退值与旧标记。
ALLOWED_FILES = {ROOT / "src" / "platform" / "windows_system_dirs.rs"}
# `findstr /C:"..."` 这类命令行开关不是盘符，前面带 `/` 的排除。
DRIVE_C = re.compile(r"(?i)(?<![A-Za-z/])c:[\\/]")
# 实验目录只在破坏性测试的守卫下使用，不属于生产路径。
LAB_MARKERS = ("RustDeskAgentLab",)


def lex(source):
    """返回 (字符串字面量列表[(起点, 内容)], 需要跳过的区间列表)。"""
    strings, skipped = [], []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = n if end < 0 else end
            skipped.append((i, end))
            i = end
            continue
        if source.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif source.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            skipped.append((i, j))
            i = j
            continue
        raw = re.match(r'b?r(#*)"', source[i:i + 40])
        if raw and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
            hashes = raw.group(1)
            start = i + raw.end()
            end = source.find('"' + hashes, start)
            end = n if end < 0 else end
            strings.append((i, source[start:end]))
            i = end + 1 + len(hashes)
            continue
        if c == '"' or (c == "b" and source.startswith('b"', i)):
            j = i + (2 if c == "b" else 1)
            buf = []
            while j < n and source[j] != '"':
                if source[j] == "\\" and j + 1 < n:
                    buf.append(source[j:j + 2])
                    j += 2
                else:
                    buf.append(source[j])
                    j += 1
            strings.append((i, "".join(buf)))
            i = j + 1
            continue
        if c == "'":
            m = re.match(r"'(\\.|\\u\{[0-9a-fA-F]+\}|[^\\'])'", source[i:i + 12])
            if m:
                skipped.append((i, i + m.end()))
                i += m.end()
                continue
        i += 1
    return strings, skipped


def test_item_ranges(source):
    """`#[cfg(test)]` 以及 `#[cfg(all(test, ...))]` 修饰的条目所占的区间。"""
    strings, comments = lex(source)
    blocked = sorted(comments + [(p, p + len(s) + 2) for p, s in strings])

    def in_blocked(pos):
        return any(a <= pos < b for a, b in blocked)

    ranges = []
    for m in re.finditer(r"#\[cfg\((?:test|all\(\s*test\b[^\]]*)\)\]", source):
        if in_blocked(m.start()):
            continue
        j = m.end()
        depth = 0
        while j < len(source):
            if in_blocked(j):
                j += 1
                continue
            ch = source[j]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    j += 1
                    break
            elif ch == ";" and depth == 0:
                j += 1
                break
            j += 1
        ranges.append((m.start(), j))
    return ranges


def violations():
    found = []
    for base in SCANNED:
        for path in sorted(base.rglob("*.rs")):
            if path in ALLOWED_FILES or "tests" in path.relative_to(ROOT).parts[1:]:
                continue
            if path.name.endswith("_tests.rs") or path.name == "tests.rs":
                continue
            source = path.read_text(encoding="utf-8")
            strings, _ = lex(source)
            tests = test_item_ranges(source)
            for pos, value in strings:
                if not DRIVE_C.search(value) or any(m in value for m in LAB_MARKERS):
                    continue
                if any(a <= pos < b for a, b in tests):
                    continue
                line = source.count("\n", 0, pos) + 1
                found.append(f"{path.relative_to(ROOT)}:{line}: {value[:90]}")
    return found


class NoHardcodedSystemDrive(unittest.TestCase):
    def test_production_code_resolves_system_folders_at_runtime(self):
        found = violations()
        self.assertEqual(found, [], "\n" + "\n".join(found))


class LexerSelfCheck(unittest.TestCase):
    def test_lexer_skips_comments_and_test_items(self):
        source = '''
// "C:\\\\in comment"
fn prod() { let a = r"C:\\Program Files\\RustDesk"; }
#[cfg(test)]
mod tests { fn t() { let b = "C:\\\\Windows"; let c = '{'; } }
#[cfg(test)]
const T: &str = r"C:\\ProgramData";
fn lab() { let d = r"C:\\RustDeskAgentLab\\x"; let e = r#"{"a":"C:/x"}"#; }
'''
        strings, _ = lex(source)
        tests = test_item_ranges(source)
        live = [
            v for p, v in strings
            if DRIVE_C.search(v) and not any(a <= p < b for a, b in tests)
            and not any(m in v for m in LAB_MARKERS)
        ]
        self.assertEqual(live, [r"C:\Program Files\RustDesk", '{"a":"C:/x"}'])


if __name__ == "__main__":
    unittest.main()
