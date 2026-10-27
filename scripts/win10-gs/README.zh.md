# win10-gs 本机编译与验收脚本

给 Mac 侧用的固定入口。目标机：`win10-gs`（termark asset `01a15c96-d93e-7ecc-ab71-6d5f1d90323d`，`10.0.0.100`）。

## 前置

- 本机 Termark 桌面端在跑，且能连 win10-gs
- 远端仓库在 `D:\github\rd-client`（与 AGENTS.md 一致）
- 工具链在 `D:\build-env`（不要动）

## 常用流程

```bash
# 1) 差量上传本机改过的 agent/终端源文件
scripts/win10-gs/sync-sources.sh
# 或只推 git 工作区相关改动（目录会自动展开）
scripts/win10-gs/sync-sources.sh --from-git
# 或指定文件
scripts/win10-gs/sync-sources.sh src/management_agent/terminal/runtime.rs

# 2) 远端编译 agent（默认直连 termark，保持 SSH；编译完返回退出码）
scripts/win10-gs/run-build.sh              # debug
scripts/win10-gs/run-build.sh release
# 编译完顺便跑终端过滤测试
scripts/win10-gs/run-build.sh debug 'terminal::'
# 可选：脱离式计划任务（BUILD_VIA=schtasks）
BUILD_VIA=schtasks scripts/win10-gs/run-build.sh debug

# 3) 需要替换现网 agent 时（破坏性，会停服务；在 win10-gs 上以管理员执行）
#   powershell -NoProfile -ExecutionPolicy Bypass -File D:\github\rd-client\scripts\win10-gs\replace-agent.ps1
```

## 文件

| 文件 | 作用 |
|---|---|
| `common.sh` | asset id、远端路径、termark JSON 解包 |
| `sync-sources.sh` | SHA-256 差量 `termark upload`；目录自动展开 |
| `build-agent.ps1` | 远端编译（vcvars + 锁定工具链，UTF-8 BOM） |
| `run-build.sh` | Mac 触发编译；默认 direct 长连接，可选 schtasks |
| `run-terminal-tests.ps1` | 远端 `cargo test … terminal::` |
| `replace-agent.ps1` | 安全替换已安装 agent（System 属性 + SCM） |

## 注意

- win10-gs 上 `git pull` 不可用；只靠 `sync-sources.sh` 覆盖源文件
- **Windows PowerShell 5.1 读无 BOM 的 UTF-8 会按系统代码页解析**，`run-build.sh` 上传 `.ps1` 时会自动加 UTF-8 BOM
- 日志在 `D:\tmp\rdtest\agent-build\`
- 不要把 `D:\build-env\cargo\bin` 代理放到 PATH 前面
- `termark exec` 在管道/捕获时输出 JSON，`common.sh` 的 `win10_out` / `win10_exec` 已解包 `output`
