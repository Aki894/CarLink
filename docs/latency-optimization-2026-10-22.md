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

## 首轮硬件观察及修正

应用 `2636428` 经 CI 成功构建并远程部署。用户反馈画面更流畅，但声音断续。约 70 秒采样记录：

- Python 转发队列的五秒窗口最大值：中位数 19.88 ms，最高 23.15 ms；socket 写入最高 3.96 ms。
- Rust 侧视频年龄的窗口最大值：中位数 58 ms，最高 73 ms。
- USB 队列最高 94.53 ms，单次写入最高 94.51 ms；超时丢弃计数为 0。
- 源端出现最高 556.11 ms 的旧帧；已触发丢弃与关键帧恢复，日志观察到恢复约 68–90 ms。该时延基于发送时钟的最小偏移估计。
- 服务未自动重启。

定位到 Rust 中遗留的音频门控：视频等待关键帧时会丢音频块。修正为音频继续通过，并扩展回归测试，在等待关键帧状态下验证音频数据确实输出。压缩音频解码入口恢复 16 包缓冲并独立设置 500 ms 容忍窗口，适配 AirPlay 突发包；解码后的 PCM/live/USB 仍使用短队列及 120 ms 期限。

协议元数据另经真实上游 `oxp_wire::write` 检查：采用标准 stats 消息，`clockOffsetUs` 按规范编码为十进制字符串。

## 音乐播放时的画面延迟

`95d7147` 修正后，用户先确认声音流畅，随后明确复现：不播放音乐时操作较及时，播放音乐时画面操作约延迟一秒。曾出现短暂卡住并自行恢复。关闭 buffered-audio 声明的对照配置仍有该现象，因此恢复原声明。

临时时钟诊断记录到视频时间戳距当前约 4–49 ms，而 PCM 音频时间戳持续落后约 90–405 ms。压缩音频时间戳取自解码前输入，输出受解码缓冲及 PCM 节奏控制，两个时刻存在差距。

曾尝试在 headless 模式下把 PCM 时间戳锚定到实际输出时刻。用户对照反馈声音连续，但音乐播放期间画面仍有变化的延迟，最高 2 秒以上。该修改已通过 `2c47acc` 撤回，保留原音频时间线。

码率方面，固定上游实现提供帧率、分辨率协商，未发现直接设置视频目标码率的接口。动态画面日志约 60 视频包/秒，约 2–3 Mbit/s。CarLife 声明 30 帧，OXP 消费端偏好也为 30 帧，但上游会话初始帧率默认 60，偏好更新不直接改变已建立会话。

`d021e10` 在 headless 模式建立 AirPlay 会话时，把初始最高帧率限制到 30，保留 800×480 和 H.264 原码流转发。新增五秒窗口内实际视频 AU 帧数和有效载荷字节数统计，分别输出帧率和 Mbit/s，供硬件确认。

## 实车测试交接

用户要求先启用开机自启，并携带已有版本做实车稳定性测试。因此开发板固定保留 `95d71470ef0082b2ca0f983e94e5820295e057c8`，不部署 30 帧实验版。

- 已执行并确认 `carlink.service` 为 enabled，服务 active，运行环境只有正常配置，临时时钟与音频隔离配置均已移除。
- 开发板应用 SHA-256：`4a953c29d58adb763531942af59fabbbdd996fa894ed697d27e96cbb7cf54c3e`；运行时完整校验通过。
- 当前版本的[应用构建](https://github.com/Aki894/CarLink/actions/runs/39736044826)成功，59 项 Rust 应用测试通过；[完整固件构建](https://github.com/Aki894/CarLink/actions/runs/39736656220)成功，镜像结构、内核、模块、运行时和校验清单通过，镜像产物上传成功。
- 当前版本仍有用户报告的音乐播放期间画面延迟变化和偶发声音断续；本次实车重点比较真实车机上的表现，并验证 Lexus 触控板输入。

30 帧实验版 `fd19727` 的[应用构建](https://github.com/Aki894/CarLink/actions/runs/39740569851)与[配套完整固件](https://github.com/Aki894/CarLink/actions/runs/39741083235)均已成功，镜像检查通过且产物上传成功；未更新开发板。实际帧率、码率及音乐播放期间响应将在之后部署测试时记录。
