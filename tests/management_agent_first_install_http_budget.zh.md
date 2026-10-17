# T01 blocking HTTP 整体预算：待 VM 验证

## RED 已证实，最小修复候选

已核冻结vm-stdout.txt：400ms预算下elapsed2050ms、20chunks、Delivered1，0passed/1failed，旧数据保全。不是输入或环境失败。

锁定源码进一步确认正确机制：blocking/request.rs:355明确RequestBuilder::timeout覆盖连接开始至body完成；:362写入inner请求timeout，:138 into_async保留该请求。async_impl/client.rs:2526创建total_timeout Sleep，:2968把同一个Sleep移交Response；async_impl/body.rs:317每个frame轮询同一个timeout，不重置。因此仅在client.get(endpoint)后增加`.timeout(budget)`；保留builder超时、redirect拒绝、长度与读取上限。无新增runtime/block_on/后台worker线程，无任意生产URL暴露。

新增`t01_http_fast_silent_headers_and_oversize_preserve_data`真实loopback覆盖：快速合法manifest交付一次；无headers等待；Content-Length超限；chunked实际字节超限；延迟headers后延迟body。所有负向要求WaitingSource、零交付、旧批准/身份/state/config保全；记录elapsed。与原slowchunks测试合计2个Rust case（另1case含5个子场景），不能把5个子场景冒充5个Rust case。HTTP来源真实、交付边界计数替身，不代表实际安装E2E。

VM命令：`--ignored t01_http_ --test-threads=1 --nocapture`，预期2项；环境RUSTDESK_T01_PRODUCER=red-formal-sos-1501，正式SOS输入不变。新GREEN待VM，不声明已通过。下文保留RED阶段依据。

审查本机锁定reqwest-0.12.24源码：blocking/client.rs:388仅设置blocking Timeout；:1438将timeout用于等待响应并在:1461传给blocking Response；blocking/response.rs:428的Read每次调用wait::timeout(body.read, timeout)；blocking/wait.rs:15每次计算Instant::now()+duration。async_impl/client.rs:272默认timeout=None。当前生产只调用blocking builder.timeout，没有另外设置async请求总预算，源码支持慢速分块反复重置读取计时的假设。

本次只将原HTTP算法原样移入fetch_manifest_http，共用读取函数；生产入口只能构造ManifestSource::Fixed，固定公开API地址和15秒预算不变。Loopback地址和短预算枚举分支以及fixture入口仅cfg(test)，没有生产任意endpoint参数。尚未引入deadline修复。

新增真实TCP loopback test `t01_http_slow_chunks_exhaust_total_budget_preserve_old_data`：合法原签名manifest切成约20块，每100ms写一个chunk，HTTP读取budget400ms，单次等待均低于400ms而整体约2秒。通过真实coordinate_approval→实际reqwest读取；交付边界只计数，不调用安装或真实producer。要求WaitingSource、零交付、旧批准/state/identity/config保持、实际至少发送2块、总耗时不超过1200ms（400ms预算加800ms调度容差）。服务器非阻塞accept有8秒上限、socket读写2秒上限，client返回后停止并join；不访问外部网络。elapsed只统计协调调用，不把服务器清理时间算为客户端预算。

预期旧算法读取完整正式manifest后Delivered、交付计数1，命中BUSINESS RED。若VM实际WaitingSource且耗时合格，则记录反证/既有行为通过，调查底层其他timeout，不能人为制造RED。输入失败、未接到请求、<2块或0项不算业务RED。host只--no-run，不执行测试。当前未修生产deadline，待真实VM结果。
