#!/usr/bin/env bash
# Trigger win10-gs agent build from Mac.
# Default path: long-lived termark exec (keeps SSH open). Reliable.
# Optional: BUILD_VIA=schtasks for detached scheduled-task mode.
#
# Usage:
#   scripts/win10-gs/run-build.sh
#   scripts/win10-gs/run-build.sh release
#   scripts/win10-gs/run-build.sh debug 'terminal::'
#   BUILD_VIA=schtasks scripts/win10-gs/run-build.sh debug
set -euo pipefail
# shellcheck source=common.sh
source "$(cd "$(dirname "$0")" && pwd)/common.sh"

PROFILE="${1:-debug}"
FILTER="${2:-}"
BUILD_VIA="${BUILD_VIA:-direct}"
SCRIPT_DIR_LOCAL="$(cd "$(dirname "$0")" && pwd)"
REMOTE_SCRIPTS="${WIN10_GS_REPO}/scripts/win10-gs"
TIMEOUT_SEC="${BUILD_TIMEOUT_SEC:-2400}"

upload_ps1_bom() {
  local name="$1"
  local src="$SCRIPT_DIR_LOCAL/$name"
  local tmp
  tmp="$(mktemp /tmp/rd-ps1.XXXXXX.ps1)"
  python3 - "$src" "$tmp" <<'PY'
import sys
from pathlib import Path
src, dst = Path(sys.argv[1]), Path(sys.argv[2])
text = src.read_text(encoding="utf-8").replace("\r\n", "\n").replace("\n", "\r\n")
dst.write_bytes(b"\xef\xbb\xbf" + text.encode("utf-8"))
PY
  win10_upload "$tmp" "${REMOTE_SCRIPTS}/$name"
  rm -f "$tmp"
}

# Ensure remote dirs + scripts (PS1 with UTF-8 BOM for Windows PowerShell 5.1)
win10_out "powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path 'D:\\github\\rd-client\\scripts\\win10-gs','D:\\tmp\\rdtest\\agent-build' | Out-Null\"" >/dev/null
for f in build-agent.ps1 run-terminal-tests.ps1 replace-agent.ps1; do
  upload_ps1_bom "$f"
done
for f in common.sh sync-sources.sh run-build.sh README.zh.md; do
  if [[ -f "$SCRIPT_DIR_LOCAL/$f" ]]; then
    win10_upload "$SCRIPT_DIR_LOCAL/$f" "${REMOTE_SCRIPTS}/$f"
  fi
done

# Clear previous exit marker
win10_out "cmd /c del /f /q D:\\tmp\\rdtest\\agent-build\\build-agent-${PROFILE}-exit.txt 2>nul" >/dev/null 2>&1 || true

FILTER_ARG=()
if [[ -n "$FILTER" ]]; then
  FILTER_ARG=(-Filter "$FILTER")
fi

echo "build via=$BUILD_VIA profile=$PROFILE filter=${FILTER:-none} timeout=${TIMEOUT_SEC}s"

if [[ "$BUILD_VIA" == "direct" ]]; then
  # Keep SSH open for the whole cargo build.
  cmd="powershell -NoProfile -ExecutionPolicy Bypass -File D:\\github\\rd-client\\scripts\\win10-gs\\build-agent.ps1 -Profile ${PROFILE}"
  if [[ -n "$FILTER" ]]; then
    # Escape single quotes for remote PS
    f_esc="${FILTER//\'/\'\'}"
    cmd+=" -Filter '${f_esc}'"
  fi
  raw="$(termark exec "$WIN10_GS_ASSET_ID" --timeout "$TIMEOUT_SEC" "$cmd" 2>/dev/null || true)"
  python3 - "$raw" <<'PY'
import json, sys
raw = sys.argv[1]
start, end = raw.find("{"), raw.rfind("}")
if start < 0 or end <= start:
    print(raw)
    sys.exit(1)
data = json.loads(raw[start : end + 1])
out = data.get("output") or ""
print(out[-6000:] if len(out) > 6000 else out)
code = int(data.get("exitCode") or 1)
if code < 0:
    code &= 0xFFFFFFFF
sys.exit(0 if code == 0 else code if code < 256 else 1)
PY
  exit $?
fi

# ---- schtasks detached mode ----
TASK_NAME="rd-client-build-agent"
{
  echo '$ErrorActionPreference = "Stop"'
  echo "New-Item -ItemType Directory -Force -Path 'D:\\tmp\\rdtest\\agent-build' | Out-Null"
  echo "Remove-Item -LiteralPath 'D:\\tmp\\rdtest\\agent-build\\build-agent-${PROFILE}-exit.txt' -ErrorAction SilentlyContinue"
  if [[ -n "$FILTER" ]]; then
    f_esc="${FILTER//\'/\'\'}"
    echo "& 'D:\\github\\rd-client\\scripts\\win10-gs\\build-agent.ps1' -Profile '${PROFILE}' -Filter '${f_esc}'"
  else
    echo "& 'D:\\github\\rd-client\\scripts\\win10-gs\\build-agent.ps1' -Profile '${PROFILE}'"
  fi
  echo 'if ($null -eq $LASTEXITCODE) { exit 0 } else { exit $LASTEXITCODE }'
} > /tmp/rd-build-launcher.ps1
python3 - <<'PY'
from pathlib import Path
p = Path("/tmp/rd-build-launcher.ps1")
t = p.read_text(encoding="utf-8").replace("\r\n", "\n").replace("\n", "\r\n")
p.write_bytes(b"\xef\xbb\xbf" + t.encode("utf-8"))
PY
win10_upload /tmp/rd-build-launcher.ps1 "${REMOTE_SCRIPTS}/_launch-build.ps1"
rm -f /tmp/rd-build-launcher.ps1
{
  echo '@echo off'
  echo 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "D:\github\rd-client\scripts\win10-gs\_launch-build.ps1"'
} > /tmp/rd-build-launcher.bat
win10_upload /tmp/rd-build-launcher.bat "${REMOTE_SCRIPTS}/_launch-build.bat"
rm -f /tmp/rd-build-launcher.bat

win10_out "cmd /c schtasks /Delete /TN ${TASK_NAME} /F >nul 2>&1 & schtasks /Create /TN ${TASK_NAME} /SC ONCE /ST 23:59 /RL HIGHEST /F /TR D:\\github\\rd-client\\scripts\\win10-gs\\_launch-build.bat && schtasks /Run /TN ${TASK_NAME}"
echo "polling exit file..."
for _ in $(seq 1 180); do
  raw="$(win10_out "cmd /c if exist D:\\tmp\\rdtest\\agent-build\\build-agent-${PROFILE}-exit.txt (type D:\\tmp\\rdtest\\agent-build\\build-agent-${PROFILE}-exit.txt) else (echo pending)" | tr -d '\r' | tail -n 1 | tr -d '[:space:]')"
  if [[ "$raw" != "pending" && -n "$raw" ]]; then
    echo "build finished: exit=$raw"
    latest="$(win10_out "cmd /c dir /b /o-d D:\\tmp\\rdtest\\agent-build\\build-agent-${PROFILE}-*.log" | tr -d '\r' | head -n 1 || true)"
    if [[ -n "$latest" ]]; then
      win10_out "cmd /c type D:\\tmp\\rdtest\\agent-build\\${latest}" | tail -80 || true
    fi
    win10_out "cmd /c schtasks /Delete /TN ${TASK_NAME} /F" >/dev/null 2>&1 || true
    [[ "$raw" =~ ^[0-9]+$ ]] && exit "$raw"
    echo "non-numeric exit: [$raw]" >&2
    exit 1
  fi
  sleep 10
done
echo "timeout" >&2
win10_out "cmd /c schtasks /Delete /TN ${TASK_NAME} /F" >/dev/null 2>&1 || true
exit 124
