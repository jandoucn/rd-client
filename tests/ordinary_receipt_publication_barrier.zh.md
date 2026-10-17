# T02 普通 MSI 回执发布屏障测试

当前源码入口为 `tests/fixtures/ordinary_publication/cases.rs`，显式构建：
`cargo test --manifest-path tests/fixtures/ordinary_publication/Cargo.toml --tests --no-run`。
该独立 package 的 build.rs 只为自身设置既有 `t02_receipts` cfg，避免引入 updater runtime；根 Cargo 不再自动发现此入口。两个父测试及 ignored child 全部保留，运行仍只在 VM。

同一清单现显式注册 `ordinary_update_terminal_contract`，编译其完整 persistence 用例。该文件默认根入口仅保留原 projection 提示，三处 receipt/module cfg 要求显式 t02_receipts，避免无 updater 上下文时编译 runtime。2026-10-17 两个独立 target no-run exit 0；根 `--offline --locked --tests --no-run` 最终 exit 0；均未执行测试。

新增独立integration target，只编译并执行生产`ordinary_update_receipts.rs`中的`Outbox::fixture/fixture_open/record_outcome/write_once/read_raw_file/workspace`；不包含updater runtime、installer、API或服务操作。

父测试先独占创建真实fixture目录和immutable intent.json，再启动两个同binary child。两个child只`fixture_open`该intent，不竞争初始化；各自序列化同事务/不同outcome并写完各自staging后才创建ready文件。父等待A/B ready，写release-a并wait/join确认A退出，再写release-b并join B。没有sleep竞速排序；短sleep只用于轮询显式barrier文件。测试断言winner字节、原intent字节、无残留staging和read-only reconciliation结果。

Win10 owner精确分别运行：

```powershell
$env:RUSTDESK_T02_VM='ordinary-terminal'
& '<EXE>' --exact same_outcome_second_writer_is_idempotent_after_first_exit --test-threads=1 --nocapture
& '<EXE>' --exact conflicting_second_writer_fails_after_first_exit_and_preserves_winner --test-threads=1 --nocapture
```

每条选择1项，child `writer_child`由parent显式--ignored启动。测试自身用unique TEMP目录，Drop清理本任务目录；不触碰产品/VM安装路径或共享资源。
