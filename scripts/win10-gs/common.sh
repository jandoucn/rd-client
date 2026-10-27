#!/usr/bin/env bash
# win10-gs 共用变量。被本目录其它脚本 source。
set -euo pipefail

WIN10_GS_ASSET_ID="${WIN10_GS_ASSET_ID:-01a15c96-d93e-7ecc-ab71-6d5f1d90323d}"
WIN10_GS_REPO="${WIN10_GS_REPO:-D:/github/rd-client}"
WIN10_GS_TEST_ROOT="${WIN10_GS_TEST_ROOT:-D:/tmp/rdtest}"
WIN10_GS_BUILD_LOG_DIR="${WIN10_GS_BUILD_LOG_DIR:-D:/tmp/rdtest/agent-build}"

# 仓库根（本脚本位于 scripts/win10-gs/）
# BASH_SOURCE 在被 source 且 set -u 时可能未设置，做兜底。
_WIN10_GS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
REPO_ROOT="$(cd "$_WIN10_GS_DIR/../.." && pwd)"

# 捕获输出时 termark 默认吐 JSON；这里统一抽出 output 字段并透传 exitCode。
win10_exec() {
  local raw
  raw="$(termark exec "$WIN10_GS_ASSET_ID" "$@" 2>/dev/null || true)"
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$raw" <<'PY'
import json, sys
raw = sys.argv[1] if len(sys.argv) > 1 else ""
# termark 有时前后夹杂空白；找第一个 { 到最后一个 }
start = raw.find("{")
end = raw.rfind("}")
if start < 0 or end < start:
    sys.stdout.write(raw)
    sys.exit(0)
try:
    data = json.loads(raw[start : end + 1])
except json.JSONDecodeError:
    sys.stdout.write(raw)
    sys.exit(0)
sys.stdout.write(data.get("output") or "")
code = data.get("exitCode", 0)
try:
    code = int(code)
except (TypeError, ValueError):
    code = 1
# Windows 无符号 32 位失败码
if code < 0:
    code = code & 0xFFFFFFFF
sys.exit(0 if code == 0 else 1)
PY
  else
    printf '%s' "$raw"
  fi
}

# 只要远端命令的原始文本输出（忽略远端 exit，由调用方自己判）。
win10_out() {
  local raw
  raw="$(termark exec "$WIN10_GS_ASSET_ID" "$@" 2>/dev/null || true)"
  python3 - "$raw" <<'PY'
import json, sys
raw = sys.argv[1] if len(sys.argv) > 1 else ""
start = raw.find("{")
end = raw.rfind("}")
if start < 0 or end < start:
    sys.stdout.write(raw)
    raise SystemExit(0)
try:
    data = json.loads(raw[start : end + 1])
except json.JSONDecodeError:
    sys.stdout.write(raw)
    raise SystemExit(0)
sys.stdout.write(data.get("output") or "")
PY
}

win10_upload() {
  local local_path="$1"
  local remote_path="$2"
  termark upload "$WIN10_GS_ASSET_ID" "$local_path" "$remote_path" >/dev/null
}

# 把 Unix 路径转成远端 PowerShell 可用的反斜杠路径。
to_win_path() {
  printf '%s' "$1" | sed 's|/|\\|g'
}
