# Win11 0510 SOS MSI→1501：真实注册源与首次初始化准备

本阶段只读调查和构建准备，不运行VM、不重跑已有GREEN。Win10旧EXE0510不适用于本MSI场景。Win11窗口由主协调安排，执行前重新核旧ProductCode、edition、build、实例、无会话、门禁及旧资源，不根据历史标签假定当前基线。

## 当前源码证据与冲突

1. 旧普通路径updater.rs:update_new_version调用update_me_msi成功后remove_download_artifact；该函数删除原文件并尝试删除UPDATE_DOWNLOAD_DIR_PREFIX临时目录。失败路径也删除。windows.rs:update_me_msi直接用传入路径msiexec /i，没有永久cache移交。
2. **当前owner工作区已改变MSI入口**：updater.rs:1348转ordinary_update_receipts::runtime::install并return；不能再把上条旧清理路径当当前必经路径。runtime.rs:91复制到受保护outbox/package.msi，:114及:119从该package.msi执行安装。本轮读到的新路径没有成功后删除该包的调用，不能断言当前安装源一定已删。
3. msi_metadata::installed_source真实读取MsiSourceListGetInfo的PackageName/LastUsedSource/LastUsedType，再打开注册源MSI。producer要求PackageName及文件basename等于签名target.primary原basename。因此新路径的package.msi可能形成**注册basename冲突**；必须在真实升级后读取，不能改注册表/改名/补拷官方包来令检查通过。
4. reader先锁验注册包，再copy_registered；复制完成前必须保留可用源。旧清理与worker时序有潜在竞争；仅凭代码不能确认具体VM上安装后源寿命或其是否被OS缓存替代。Windows Installer LocalPackage不是该reader查询的LastUsedSource，不能混作同一来源。
5. reader在rescue_first_install.rs:747从正式MSI提取agent后，把其size/hash用于校验已安装ManagementAgent及RecoveryBootstrap。当前候选agent含新协调逻辑，**不同于官方1501 MSI内agent**。只把官方MSI升级后换本地候选agent，不能满足完整资源首次初始化；build同1501不等于字节身份相同。这一拒绝必须保留，不能绕过核验或重新签名伪装官方包。

## 可直接执行的第一窗口

输入仍为原正式SOS1501：MSI SHA256=17f8ee985719936177c7b8c3cf4078e6879882dfde7bbd3d6f9ca37af15ad519，resolved.json=2ca7049c9dabda9899182184184cf4c6eeee6cfeb7604bbf54f1af71df42da45，originalcatalog=acb6f930b3384d5576d60351f3d18de08716ab614b28162e5253e6b95d1d974c。发布checkout=9d8d430c01811965c7585d3b57530481eaa15174。本地candidate必须另记源码/EXE摘要，绝不标official。

主Win11 owner按独立已审查正常升级driver执行0510→原官方1501，保留真实安装日志、原始安装路径及退出码；若选在线updater入口，还要绑定执行的旧/候选updater源码，不用直接msiexec冒充在线升级。安装结束即采样原始注册值、包路径是否存在/大小/hash、安装返回UTC和worker报告mtime，再执行新增**只读**测试：

```powershell
$env:RUSTDESK_T01_REGISTERED='observe-installed-1501'
& '<registered-source测试EXE>' --ignored --exact t01_installed_1501_real_registered_source_matches_signed_release --test-threads=1 --nocapture
```

必须1项。该用例只读正式input与真实安装注册源，正式验签、ProductCode/basename/锁定包hash；比较state/identity/config原bytes，不写批准或注册源，不安装、不启停服务、不访问网络。源缺失/不同名记录实际失败和前置，不自动补源。旧0510上执行不算1501产品RED。

## 下一首次初始化窗口的合法边界

可以在仅fixture构建中绑定loopback origin，驱动真实worker、production注册查询及健康链，原正式manifest逐字返回；不得覆盖生产固定API常量或注入任意域名/URL。先完成严格来源检查且正式1501已健康后才启动候选。该混合场景可验producer真实注册positive及reader准确拒绝候选agent不匹配，但**不能**验完整初始化成功；不得把降级/拒绝报告当资源准备成功。

完整成功需要一个明确标记为laboratory的受审候选MSI，其实际payload包括同一候选agent/bootstrap，并有仅fixture构建接受的测试签名清单；不能以原正式1501签名批准修改后的包，也不能让production key gate接受testkey。该候选包构建/签名输入尚未准备，本轮未产出也未运行mixed worker。因此本轮仅交付注册只读probe，不把未完成的端到端driver标成可验收包。

通过条件分层：注册positive→producer真实入口交付→reader真实资源校验→离线资源完整性；ready常量保持现有语义，不要求ready=true。失败保留当前正常应用、原身份、注册/包、旧批准及旧bundle。每个窗口结束由owner审查新增资源再清理，保留既有RED现场。覆盖率和完整E2E未闭合。
