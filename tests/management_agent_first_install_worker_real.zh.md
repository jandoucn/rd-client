# T01 真实 worker→HTTP→producer→reader 分层验收

用例：`t01_real_worker_http_producer_reader_preserves_health_failure_and_waiting`。独立integration crate通过既有modules.rs导入完整生产模块图，直接调用first_install的scoped启动入口；没有源码提取，没有fake prepare。真实thread/channel、300秒节流、协调函数、reqwest loopback读取、正式公钥验签、producer文件发布、reader重新验证及state_store报告发布均实际执行。

测试边界仅cfg(test)：线程局部scope指定隔离root，MSI注册源使用正式原包路径/ProductCode绑定替身，健康调用记录后明确返回`isolated worker health unavailable`；协调闭包调用真实coordinate_approval/fetch_loopback_fixture/deliver_at。它替代生产固定root/SCM/注册查询/公网来源发现，不证明这些系统入口已验收；生产cfgnot(test)调用保持原样。线程局部scope在真实worker线程内建立，不影响其他test线程。

场景一：200正式精确manifest→sidecar存在→真实reader在其正式签名、MSI身份和锁定包校验之后到达健康边界一次，报告必须为preparation_refused且原错误精确保留。健康明确失败，禁止进一步MSI提取、服务或bundle操作。场景二：404→真实协调WaitingSource→sidecar缺失→真实reader waiting_approval，健康调用零次。两场均ready=false、无rescue/current、state/身份/config字节保持。

Win10串行执行，保持所有旧现场，测试只创建新t01-worker-real-UUID目录并打印供归档；不访问生产网络、不操作正式安装/服务。输入复用C:\RustDeskAgentLab\t01-delivery-input正式SOS包及resolved.json；环境RUSTDESK_T01_PRODUCER=red-formal-sos-1501。

```powershell
& '<新EXE>' --ignored --exact t01_real_worker_http_producer_reader_preserves_health_failure_and_waiting --test-threads=1 --nocapture
```

预期1个Rust case含两个场景。当前为待VM基线；已有路径若通过则记录baseline GREEN，不人为造RED。缺输入/编译失败/零匹配不算RED。完整资源成功准备、真实SCM/注册/健康、实际安装及在线升级E2E未覆盖；不标产品闭合或80%覆盖率。host仅no-run，无VM/CI/提交/子代理。
