# T01 worker 接线 RED 候选与薄 hook 草案

当前显式入口为 `tests/fixtures/worker_delivery/Cargo.toml`，测试源码 `cases.rs`；根 Cargo 不自动发现它。
先执行 `python tests/fixtures/worker_delivery/prepare.py` 将根锁中的固定修订复制到独立 package（不覆盖已有锁），再执行 `python tests/management_agent_first_install_worker_extract.py <新的提取目录>`，设置 `T01_WORKER_EXTRACT=<提取目录>/worker.rs`，执行 `cargo test --manifest-path tests/fixtures/worker_delivery/Cargo.toml --tests --no-run`。首次 Cargo 仅调整隔离锁的 package 图，正式 Cargo.lock 不变。
缺提取输入时显式 target 编译失败是准备错误；默认根测试编译不需要该环境变量。运行仍限独占 VM，使用原测试名与 `--ignored --exact`。

已核上一阶段vm-result.json：9 passed，exit=0，services_match=true，changed=[]；模块a5904090的ACL修复已通过。本阶段未改该模块及共享调用点。

## 实际调用链和来源

- management_agent.rs:1606启动first_install::start，:1766真实notify；rescue_first_install.rs:897的start_checked以容量1通道接收，300秒节流，fixed_layout/current检查后直接prepare，没有manifest获取和producer交付。
- management_agent.rs:1472现有恢复下载分支在normal/fallback存在时直接返回，不是正常安装完成的批准来源，不能挪用它的latest下载行为。
- 固定API为https://rdapi.yan.life/rd/update/v1/manifest/stable.json；API sqlite/index.php:1934路由只调用public_update_manifest(db,channel)，不读取历史build参数。:580函数虽有targetBuild形参，路由并未暴露，而且比较为上界筛选，不是精确批准。不能虚构?build_seq协议。
- 发布器publish-release-to-oss.py:283定义manifest映射，:626发布release_prefix/catalog.json。已证明1501固定历史catalog URL与原始签名，但通用当前安装→可信tag尚未接好；health响应仅version/build_seq，没有source_tag。不能从latest或不可信URL倒推历史批准。

最小第一版可仅请求固定API，严格禁止redirect，限制body<=262144、连接/总超时；使用正式签名gate后要求选中MSI target的build==本机可信当前build。API不是当前build则维持waiting_approval并记录来源不匹配，不下载latest安装包、不猜历史tag。后续确需历史catalog时，从受验证的当前安装元数据取得版本/tag后再制定唯一固定下载域路径与publisher等价映射测试；没有可信tag则继续等待。

## 本次可执行候选边界

management_agent_first_install_worker_extract.py逐字提取当前生产Worker/start_checked/start尾段（保存源摘要及片段摘要），不修改任何worker语句。真实线程、SyncSender notify、节流、prepare调用和state_store发布执行。fixed_layout/prepare/workspace为隔离替身，正式manifest前置用真实生产公钥独立验签，身份/config保全后断言worker交付sidecar。

这是生产worker调度的接线缺口候选，不是完整安装E2E：available-manifest.json是测试提供的来源边界，现有产品尚无读取它的契约。本次不将该文件包装成合法生产入口，也不能仅凭其存在把缺批准认作生产缺陷。若Win10失败，可证明当前调度未交付；正式接线RED还需在主确认下把来源边界绑定拟接固定API/协调入口。不能以该候选替代真实HTTP、注册源及产品入口验收。

## 薄 hook 草案（未应用）

management_agent.rs在同级Windows模块声明处增加`mod first_install_approval_delivery;`。
rescue_first_install.rs的worker在fixed_layout/current检查之后、prepare之前调用新模块的`coordinate_current_installation()`；该函数自行读取固定state、核健康当前build与固定MSI身份、固定API取manifest、正式验证后调用已通过producer。该调用应在reader获取InstallGuard之前，不能持reader互斥等待HTTP。

建议只将来源失败记为限流诊断后继续原prepare，保留waiting_approval；不得修改正常应用ready/registered，成功也不直接写ready。原有sidecar由reader继续复验。updater由其owner交接签名manifest；远程事务继续after_commit。

本次仅新增独占tests及提取驱动；coordination尚未实现，主可据该草案决定共享hook落点。host只编译；VM/CI/commit/subagents均未执行，覆盖率未知。
