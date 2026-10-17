# T01 正式首次协调契约：RED阶段

## 最新：协调实现与生产薄接线候选

已核t02green-t01coord-coord.stdout/stderr：2failed，正向Err(CoordinationUnimplemented)而期望Delivered；四类负向均stub，保全断言通过。现已实现coordinate_approval，测试字节保持c291456e474d91027d75cda5b8878db62967bbd7f417c57c1d3eeca80c20570b。

新冻结目录：`C:\Users\Administrator\AppData\Local\Temp\opencode\t01-coordination-green-freeze-20261016`。
测试EXE：`management_agent_first_install_approval_delivery-77c31a89923d1b9b.exe`。
EXE SHA-256：`09140d2110042a45e0eaf838e049e56941c8c8a868f1ac9c63eedd2d6fcac8c6`。
模块SHA-256：`ab254a4bcecf3438599bb63435bc5172e68cf053ad966dbb9e584cdafd80721a`。

Win10 owner沿用原正式SOS输入与RUSTDESK_T01_PRODUCER=red-formal-sos-1501，分别执行：

```powershell
& '<新EXE>' --ignored t01_coordination_ --test-threads=1 --nocapture
& '<新EXE>' --ignored t01_producer_ --test-threads=1 --nocapture
```

应分别2项与9项。当前仅host编译，未宣称新GREEN；JSON build-finished.success=true，且production rustdesk-management-agent compiler-artifact fresh=false/test=false，wrapper和worker生产路径实际编译成功，不仅cfgtest。

固定GET仅rdapi既有公开manifest地址；禁止redirect，连接5秒/总15秒，Content-Length和读取均限制262144。MSI枚举仅machine context、最多4096，唯一RustDesk且InstallLocation为固定Program Files目录；不是任意ProductCode输入。当前build取Agent内嵌version.json并要求固定主程序健康IPC精确同build及无会话，不一致则等待。固定profile/state和本地gates预查，producer在安装互斥下重新验证state/签名/build/真实注册源/锁定包/发布门禁。HTTP前没有持安装互斥。

shared变更仅management_agent.rs的Windows模块声明，以及rescue_first_install.rs worker在无current且缺sidecar时调用wrapper；错误限流后继续原prepare，waiting报告仍由原prepare发布。不写新报告、不改ready或正常application状态；management_agent.rs原extra_fields及测试mod修改原样保留。worker网络hook在cfg(not(test))，避免既有资产单测意外访问公网；生产编译已覆盖，不能拿当前单测代替worker运行验收。

未覆盖：固定HTTP真实fixture的200/404/redirect/超限/超时、真实MSI枚举/注册正向、生产worker调度→HTTP→交付→prepare、普通安装/在线升级E2E及分支覆盖率。此前worker片段fixture仅基线调度，不计该生产hookGREEN。还未closed。以下为此前RED阶段契约。

用户明确授权公开固定API的正式signedmanifest为普通首次批准来源。无需command resolve、历史tag或新服务器协议。此契约取代前一worker片段测试中未绑定的available-manifest文件假设。

## 入口与边界

`coordinate_approval(CurrentInstallation, fetch_fixed_source, deliver)` 是将供固定生产wrapper调用的协调逻辑。CurrentInstallation为当前本机installation/固定MSItarget/精确build/ProductCode，不提供URL。fetch/deliver为内部依赖边界；生产wrapper固定绑定公开API和deliver_installed_msi_approval，测试注入原始正式manifest和隔离文件producer。缺实现明确返回CoordinationUnimplemented，待VM真实RED才实现。

两个新增测试从协调入口进入，不直接以producer作为测试起点：

- `t01_coordination_exact_public_manifest_delivers_before_return`：获取一次原正式manifest，经真实签名gate前置确认；期望协调函数只调用一次真实文件producer、输出schema1批准、身份/config/state字节保全。
- `t01_coordination_unavailable_or_nonexact_keeps_waiting_and_old_bytes`：网络失败、签名损坏、两个方向build不等；要求WaitingSource/WaitingSignature/WaitingExactBuild，零次producer调用及旧批准/身份/config/state字节保全。每个子场景检查保全后汇总错误，不以无崩溃代替错误断言。

这是协调逻辑/文件集成层，网络为替身、注册源为cfgtest注入，不声称真实HTTP/生产注册源/普通安装E2E。现有9项模块GREEN仍独立保留。

## RED之后最小生产接线

1. 固定wrapper从现有可信state和本机MSI注册查询建立CurrentInstallation，当前build必须结合固定主程序健康身份校验。不能单凭API响应决定当前build。
2. reqwest blocking固定GET `https://rdapi.yan.life/rd/update/v1/manifest/stable.json`，禁redirect，限制connect/总时间与body大小；无外部URL参数，不用command resolve。
3. 正式key验签、固定MSI target、精确current build相等后调用现有production producer；production producer再核真实ProductCode/registered_source/basename/锁定包及本地门禁。缺失、网络错误或不匹配保持等待，不修改正常application状态。
4. management_agent.rs仅增加Windows模块声明；rescue_first_install worker在current不存在检查后、prepare前薄调用固定wrapper，失败限流诊断后继续既有prepare。reader互斥尚未取得时调用，禁止持锁HTTP等待。updater不修改；ready常量不修改。

本轮只增加协调stub和tests；共享源码未接线。后续收到真实RED再写协调实现及上述薄hook。host仅--no-run，noVM/noCI/no提交/no子代理。
