# H3 实时转发延迟调研与优化

2026-10-22。用户复现：声音和触控基本正常，画面偶尔落后 1–2 秒，再加速追上。目标是防止补播过期画面，同时保留声音和 Lexus 触控板操作。

## 现场基线

- H3，内核 `6.18.57-current-sunxi`，800×480 H.264 透传，音频转 44.1 kHz 双声道 PCM；无视频软件解码、缩放和重编码。
- AIC8800D80，5 GHz 信道 149、20 MHz，信号约 -60 dBm，接收协商速率 86.7 Mbit/s，Wi-Fi 省电已关闭。TCP 采样 RTT 约 2–4 ms。
- USB gadget 为 high-speed。板温约 44–46°C；操作期间 CPU 已升至 1.296 GHz，CPU/IO 压力较低；CarLink 常驻约 18 MiB。
- 操作时本地 socket 采样出现约 8–11 KiB 短队列，也有清空时刻。上游收到静态画面时约每秒一个心跳，动态画面会密集到达；不能把静态时的 1 秒读取间隔当成丢包或排队时间。

## 一手资料与结论

1. 固定版本 [OpenXPlay stream_connection](https://github.com/OpenXPlay/OpenXPlay-Relay/blob/6a7cdab585113e29422f2306e90fdb5a789c318a/rust/direlay-linux/src/stream_connection.rs) 原有 128 项发送队列，只按数量限制；[stream_bridge_sink](https://github.com/OpenXPlay/OpenXPlay-Relay/blob/6a7cdab585113e29422f2306e90fdb5a789c318a/rust/direlay-linux/src/stream_bridge_sink.rs) 的时间戳默认来自进程内部纪元。跨进程诊断应明确时钟，而不是直接将该值与系统 uptime 相减。
2. [Linux socket(7)](https://man7.org/linux/man-pages/man7/socket.7.html) 说明发送缓冲与应用队列是两个独立层次，SO_SNDBUF 还有内核记账开销。只缩小 Python deque 不能覆盖内核、上游 Rust 和 USB 等候。
3. [Linux FunctionFS](https://docs.kernel.org/usb/functionfs.html) 与 [USB gadget API](https://docs.kernel.org/driver-api/usb/gadget.html) 要求正确处理端点和传输完成。CarLife 外层头与正文保持独立发送，开始传输后完成整个帧；过期丢弃应放在写入之前，避免截断协议。
4. [FFmpeg formats](https://ffmpeg.org/ffmpeg-formats.html) 将 nobuffer 描述为减少初始分析缓冲；[pipe 协议](https://ffmpeg.org/ffmpeg-protocols.html#pipe) 的 blocksize 限制 IO 操作大小。它们不是消除全链路视频积压的开关。当前视频不经过 FFmpeg，已有 probesize/analyzeduration/flush 配置保留；音频单独限制排队并平滑输出。
5. [CPUFreq 文档](https://docs.kernel.org/admin-guide/pm/cpufreq.html) 说明性能、功耗和热限制之间的关系；[无线 PM QoS](https://wireless.docs.kernel.org/en/latest/en/developers/documentation/pm-qos.html) 适用范围与驱动有关。当前 CPU 已到高频，外置 fullmac 网卡也已关闭省电，本轮优先修复可观察的软件积压，保留现有散热管理和无线参数。

## 实现

- 仅 headless forward 模式启用 `CARLINK_LOW_LATENCY=1`。上游利用 iPhone screen header 的发送时刻与板上 CLOCK_MONOTONIC 建立最小传输偏移估计，允许小幅时钟漂移，标记延迟到达的视频；流连接另发进程纪元与系统单调时钟的偏移，音频时间线独立对齐。未提供发送时钟的帧退回板上接收时刻。
- 上游发送队列由 128 降至 32 项；发送前跳过超过 120 ms 的旧视频，限频请求关键帧。
- Python 最多排队四个视频帧、每角色六个音频块；视频/音频排队预算 120 ms，live socket 发送缓冲单独设为 32 KiB。旧视频清除后等待新 IDR，不随机丢弃参考 P 帧后继续发送依赖帧。
- 本地协议新增 kind 6 视频恢复标记；video flags bit 1 表示 CLOCK_MONOTONIC 采集时刻。恢复视频不再发送全局 inactive/active，不重启音频播放对象。
- Rust 读取队列及 USB 写队列均增加时限；视频与音频使用独立代次；过期视频触发 IDR 恢复，过期音频块丢弃。USB 已开始传输的帧保持完整；控制响应不按媒体时限丢弃。
- 协议循环等待由 10 ms 缩短到 2 ms。关键帧请求限频到每 250 ms，并在等待关键帧期间重试。
- 每五秒输出 `forward_latency`、`live_latency`、`usb_tx_latency`：源到转发年龄、本地队列年龄、socket 发送时间、USB 排队和写入时间、丢弃量。仅记录统计，不保存媒体。

这些预算约束各段软件排队，实际画面延迟还包含 iPhone 编码、无线接收和车机解码显示。复测按相同页面切换和音乐操作观察，统计数据与用户看到的结果分别记录。

## 回归检查

- 合成源故意先发送两秒前的 IDR，再发新画面；旧画面泄漏到 CarLife 即失败。
- 上游纯函数检查：手机时间只前进 100 ms、接收时间前进两秒，映射后仍能识别约 1.9 秒额外等待；随后新帧恢复正确时刻。
- 检查源时钟标记跨真实 Python、Rust 会话可用，视频与音频时间戳仍单调。
- 检查视频恢复保留音频状态和音频代次、关键帧请求限频、完整按键脉冲和 Lexus 输入回归。
- 检查过期 USB 媒体丢弃而控制响应保留；完整协议帧边界继续校验。

构建、完整固件与硬件复测结果在执行完成后补充。
