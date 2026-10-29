#!/usr/bin/env bash
# 按 SHA-256 差量把本机源文件覆盖到 win10-gs 仓库。
# 默认同步 management_agent 终端相关路径；也可传文件列表。
#
# 用法:
#   scripts/win10-gs/sync-sources.sh
#   scripts/win10-gs/sync-sources.sh src/management_agent/terminal/runtime.rs
#   scripts/win10-gs/sync-sources.sh --from-git   # 同步 git status 里已改/未跟踪的相关文件
set -euo pipefail
# shellcheck source=common.sh
source "$(cd "$(dirname "$0")" && pwd)/common.sh"

DEFAULT_PATHS=(
  src/management_agent.rs
  src/management_agent/status.rs
  src/management_agent/terminal/mod.rs
  src/management_agent/terminal/runtime.rs
  src/management_agent/terminal/manager.rs
  src/management_agent/terminal/protocol.rs
  src/management_agent/terminal/transport.rs
  src/management_agent/terminal/session.rs
  src/management_agent/terminal/session_tests.rs
  src/management_agent/terminal/shell.rs
  src/management_agent/terminal/output_ring.rs
  src/management_agent/terminal/job_windows.rs
  src/management_agent/terminal/console_host.rs
  Cargo.toml
  Cargo.lock
)

collect_from_git() {
  git -C "$REPO_ROOT" status --porcelain -- \
    src/management_agent.rs \
    src/management_agent/ \
    Cargo.toml \
    Cargo.lock \
    | awk '{print $NF}'
}

PATHS=()
if [[ "${1:-}" == "--from-git" ]]; then
  while IFS= read -r line; do
    [[ -n "$line" ]] && PATHS+=("$line")
  done < <(collect_from_git)
  shift || true
fi
if [[ "$#" -gt 0 ]]; then
  PATHS+=("$@")
fi
if [[ "${#PATHS[@]}" -eq 0 ]]; then
  PATHS=("${DEFAULT_PATHS[@]}")
fi

# 目录展开成其下的普通文件（git status 会列出未跟踪目录）
EXPANDED=()
for rel in "${PATHS[@]}"; do
  local_file="$REPO_ROOT/$rel"
  if [[ -d "$local_file" ]]; then
    while IFS= read -r child; do
      EXPANDED+=("${child#"$REPO_ROOT"/}")
    done < <(find "$local_file" -type f ! -name '.DS_Store' | sort)
  else
    EXPANDED+=("$rel")
  fi
done
PATHS=("${EXPANDED[@]}")

uploaded=0
skipped=0
missing=0

for rel in "${PATHS[@]}"; do
  local_file="$REPO_ROOT/$rel"
  if [[ ! -f "$local_file" ]]; then
    echo "skip missing: $rel" >&2
    missing=$((missing + 1))
    continue
  fi
  local_hash="$(shasum -a 256 "$local_file" | awk '{print $1}')"
  remote_file="${WIN10_GS_REPO}/${rel}"
  remote_ps="$(to_win_path "$remote_file")"
  remote_hash="$(
    win10_exec "powershell -NoProfile -Command \"if (Test-Path -LiteralPath '$remote_ps') { (Get-FileHash -LiteralPath '$remote_ps' -Algorithm SHA256).Hash.ToLower() } else { 'missing' }\"" \
      | tr -d '\r' | tail -n 1 | tr '[:upper:]' '[:lower:]'
  )"
  if [[ "$remote_hash" == "$local_hash" ]]; then
    echo "ok same: $rel"
    skipped=$((skipped + 1))
    continue
  fi
  # 远端父目录可能不存在（新模块）
  remote_dir="$(dirname "$remote_file")"
  remote_dir_ps="$(to_win_path "$remote_dir")"
  win10_exec "powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path '$remote_dir_ps' | Out-Null\"" >/dev/null
  win10_upload "$local_file" "$remote_file"
  echo "uploaded: $rel ($local_hash)"
  uploaded=$((uploaded + 1))
done

echo "sync done: uploaded=$uploaded same=$skipped missing=$missing"
