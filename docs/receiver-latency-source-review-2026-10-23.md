# 接收端源码对比与延迟优化路线

调研日期：2026-10-23（香港时间）。任务：针对 H3 / AIC8800D80 / CarPlay H.264 透传 / CarLife USB，查找类似 OpenXPlay 的实现和可复用的设计。此次只调查、读取运行统计和编写文档，未调整运行参数或构建流程。

## 结论

优先完成实际源帧率与车机消费帧率匹配，其次测量音频解码前后的全部缓存和接收端显示队列。中期参考 catplay，将音频处理改为进程内、可计量的环形缓冲，减少 Python/FFmpeg 管道中的隐藏排队。保留 H.264 压缩流透传。

当前最值得借鉴的材料是用户的 DiPlay/CarProjection 分支、catplay、HeadSwap，以及 DiPlay 上游近期合并的音频积压和时钟修复。浏览器方案可帮助观测，生产替换接收端应放在后面。

## 核对的版本

| 项目 | 本次查看的本地版本 | 适用方向 |
| --- | --- | --- |
| OpenXPlay-Relay | `6a7cdab585113e29422f2306e90fdb5a789c318a`，即 sources.lock.json 固定版本 | 当前 Linux 基础 |
| 用户 DiPlay 分支 | `Aki894/DiPlay`，`9b26935f36c32417811eb6319511c3050ba35252` | 已有 CarLife 转发设计 |
| 用户 CarProjection 分支 | `4241e52361af09fe58a82bf54ddde4903c46b543` | USB 分流和队列预算 |
| catplay | `bc6edae49554f2d0160b18ed18fc7b7376353115` | 原生嵌入式接收、时钟、音频 |
| HeadSwap | `99e888cf546ec5f040d4de377c59b478186afd5b` | CarLife 输出背压与码率控制 |

GitHub API 同时核对了最新上游：OpenXPlay-Relay `e4092032` 比固定版多 29 个提交；该比较没有修改 `rust/carplay-airplay/src` 或 `rust/direlay-linux/src` 的收流核心，主要有关 Android、打包与部署。catplay 最新 HEAD 与本地一致。DiPlay 上游 `143f4674` 与用户分支已分叉，采用单项设计分析，避免整体覆盖本地工作。

## 1. 用户 DiPlay / CarProjection：直接转发与独立发送

### 已有设计

- `DiPlay/shared/.../media/CarLifeVideoBridge.kt`：单独的 `carlife-h264-writer`；队列超过 6 帧或字节预算时清理并请求关键帧；发送前有 250 ms 年龄检查；重复相同编码配置不会反复关闭管道。
- `DiPlay/shared/.../media/VideoDecodeQueue.kt`：接收和解码分离，丢失参考链后等待随机访问帧；`VideoInputPump` 在等待输入缓冲时持续排空解码输出。
- `DiPlay/shared/.../media/AndroidMediaSink.kt`：收到编码帧时直接提交 CarLife bridge；解码音频优先写向 CarLife 路由，headless 转发不要求本地扬声器播放。
- `CarProjection/.../VideoQueueBudget.java`：最多 8 个待发送视频包和 2 MiB；`MsgProcess.java` 对视频来源和 USB 会话分别记录代次。

### 对我们的意义

这些已有实践支持继续走 H.264 透传，不在 H3 上增加视频解码、缩放和重编码。当前 Linux 已实现队列限额与 IDR 恢复，下一步要补的是源帧率匹配、隐藏音频缓存测量和显示端观测，不能把既有队列设计再次当作新优化重复部署。

## 2. DiPlay 上游：两项新的音频修复

- [PR #788：音频时钟漂移](https://github.com/shihabal3amri/DiPlay/pull/788)：维护者根据某车机实际写入样本数与播放头进度，分析出播放时钟约快 0.76%；音乐缓冲逐渐耗尽，周期性重新缓冲。其设计对音乐使用缓冲水位反馈和有限速率微调。
- [PR #814：旧音乐包积压和错误降速](https://github.com/shihabal3amri/DiPlay/pull/814)：把解码前压缩音频队列按 RTP 时间跨度计入积压；对持续过量积压进行恢复，短暂无线突发保留；压缩包积压时暂停学习硬件时钟漂移。作者的确定性测试复现了约 4 秒压缩音频积压，实车效果仍待报告者验证。

适合借鉴的是“同时测量解码前、解码后、设备消费进度”，以及按时间而非包数定义预算。`AudioTrack.setPlaybackRate` 属于 Android 本地播放接口；我们的声音最终由车机播放，移植时需要 CarLife 可用反馈或独立测量，不能直接套用该 API 与控制参数。

当前 `receiver/forward.py::AudioDecoder` 使用压缩包队列、FFmpeg stdin/stdout、PCM 20 ms 节拍。stdout 读取线程同时负责等待播放时刻，等待期间停止读取；缓存可滞留在子进程或管道中，而下游统计仍然很小。需要记录输入样本时间跨度、解码耗时、管道可读字节、输出样本数及节拍落后量。

此前直接重定 PCM 时间戳的实验使用户观察到延迟加重，已回退。新的时钟实验必须先测量多阶段积压，并保留独立回退开关。

## 3. catplay：最相关的原生 Linux 架构参考

[catplay](https://github.com/catplay-labs/catplay) 面向嵌入式无线 CarPlay 盒子；README 列出的一个主要平台也使用 AIC8800D80。其目标板、无线连接方式与本项目仍需分别适配。

重点源码：

- `carplay/catplay_carplay/src/screen/rx/screen_rx_session.rs`：保留帧目标时间与本地到达时间，统计显示时间差、负提前量和晚到帧，再交给 sink。
- `.../clock/media_clock.rs`：明确本地和远端时间域之间的映射；`media_pll.rs` 有相位/频率校正结构，但当前 `DISABLE_PLL=true`。
- `.../audio/api_rx.rs`：音频 source/sink 分离，围绕样本读写和消费进度工作。
- `.../rtp/play/rtp_player.rs` 与 `carplay/catplay_audio`：RTP 处理、进程内解码和 SPSC 环形缓冲。

适合迁移的是进程内音频解码、按样本数计量的有界 PCM 环和分离的时钟观测。保留现有 FFmpeg 解码器库也是一种实现路径，不必为了借鉴架构而整体切换 codec 库。

替换整个接收端还涉及认证接口、蓝牙引导、热点、输入反馈和 ARMv7 构建集成，先在现有 OpenXPlay 上改良数据路径更便于对照。

## 4. HeadSwap：根据实际输出拥塞控制负载

`aa/BitratePolicy.kt` 把 backlog 毫秒数进行平滑，结合实际 drain 速率，拥塞时降低码率，稳定一段时间后逐步提高。当前代码下降参考点 300 ms、上升参考点 100 ms、平滑时间常数 1 秒。

这比只看 Wi-Fi 协商速率或平均 Mbps 更接近瓶颈。我们可以先引入相同的观测量：视频 AU 大小分布、USB write 分布、队列时间跨度、关键帧恢复频率。控制 iPhone 码率的接口需要单独确认：当前 OpenXPlay `AirPlayDisplayConfig` / display plist 暴露分辨率和 maxFPS，没有现成的视频 bitrate 字段。HeadSwap 能直接调 Android 编码器，这部分能力与 H.264 透传不同。

## 5. PlayPort、UxPlay 和社区经验

- [PlayPort](https://github.com/youcci/playport)：DiPlay 协议移植到 JVM 服务端，编码媒体通过 WebSocket 到浏览器，用 WebCodecs 解码并显示 FPS、码率、队列和延迟。适合作为诊断界面参考；其蓝牙引导当前以 macOS 为验证平台，H3 生产链路不需要加入浏览器和额外媒体转发。
- [UxPlay 文档](https://github.com/FDH2/UxPlay/blob/master/README.txt) 和 [RPiPlay #265](https://github.com/FD-/RPiPlay/issues/265)：延迟累积需要区分按时间戳播放与立即显示，以及消费能力落后于输入的情形。普通 AirPlay 的两秒音频模式与 CarPlay 交互不是同一配置，参数不能直接照搬。
- [个人 AirPlay 同步实践](https://gioele.dev/blog/latency-free-airplay-audio-streaming/) 展示了主动缓冲以维持音视频同步的另一种目标，适用于理解缓冲取舍。
- [vivo 智能车载官方社区](https://bbs.vivo.com.cn/newbbs/thread/37036311?show_title=1) 对帧率优先产生卡顿/延迟的车机建议降到 30 帧，支持先做兼容帧率实验。
- [GStreamer queue 文档](https://gstreamer.freedesktop.org/documentation/coreelements/queue.html) 清楚区分线程解耦、时间/字节/数量限额和阻塞行为。应借其有界队列观念；本项目视频链路并未使用 GStreamer。

## 本机代码和现场统计带来的两个优先线索

### A. 源头帧率与测试端的消费速度

OpenXPlay 固定版本默认 display fps 为 60；CarLink CarLife 输出固定协商 800×480×30。源头 30 fps 补丁 `receiver/patches/0006-headless-30fps.patch` 已经随完整固件构建成功，但接收端仍是原运行版本，本次 CarLife 握手修复只换了 Rust 程序。

华为 APK `b/a/a/a/m/j/a.java` 的 `FrameDecoder` 构造时计算 `m=1000/frameRate`；循环每次提交后按此最小间隔等待。队列是 `LinkedBlockingQueue`，并有大于 60 个消息才进入的积压处理分支。提交 MediaCodec 的 PTS 是本机 `System.nanoTime()/1000`，输出立即 render；这条已检查的显示路径没有按 CarLife 视频头里的 PTS 安排显示。

因此，应首先实测实际 H.264 AU 帧率，并检查其是否高于测试端持续消费能力。60 个访问单元在 30 fps 下相当于约两秒内容，量级与用户观察接近。这里是可检验的机制假设，实际源帧率和终端队列深度尚需采集。不能将网络包数或配置里的 FPS 直接代替实际帧率。

补充：上游 `AirPlayDisplaySettings::sanitize_fps` 当前最小值为 30。实验 20 fps 需要同时处理该约束及前后两端协商，单改 fps 参数可能仍上报 30。H.264 参考帧不能随意隔帧丢弃，优先控制 iPhone 输出。

### B. 本地很快，屏幕仍可能落后

2026-10-23 00:01 左右读取当前服务统计：Python 队列窗口峰值约 20–25 ms，source age 峰值约 41–48 ms，Rust live age 峰值约 2–67 ms；USB write 某些窗口峰值约 61–82 ms。该样本说明需要把观测延伸到华为接收/解码/显示，当前没有完整覆盖用户看到的一到两秒。

这些指标的窗口和统计起点不同，不能简单相加；源时钟映射也需要与实际接收时间交叉核对。

## 建议执行顺序

| 顺序 | 工作 | 观测结果如何指导下一步 |
| --- | --- | --- |
| 1 | 保留当前 CarLife 修复；部署已构建的源头 30 fps 接收端和实际 FPS/Mbps 统计，记录版本与开关 | 若实际 AU 输入率下降、显示积压随之消失，优先解决帧率不匹配 |
| 2 | 固定 800×480，播放/暂停音乐、静态页/连续切页做相同操作；观测手机 MediaCodec/解码线程队列 | 区分上游发送慢、USB 阻塞、终端解码慢 |
| 3 | 参考 DiPlay #814，补齐压缩音频时间跨度、解码后 PCM 水位、管道字节和节拍落后统计 | 识别隐藏音频 backlog，区分突发与持续拥塞 |
| 4 | 参考 catplay，改为进程内解码或独立持续排空解码输出，再由有界 PCM 环控制发送节奏 | 缩短链路并让缓存总量可计量 |
| 5 | 参考 HeadSwap，以发送耗时/积压驱动源头 FPS 或可验证的码率控制 | 调低真实生成负载，避免 H3 重编码 |
| 6 | 正确处理 AU、IDR、时钟切换后，比较更短但有上限的视频缓存 | 以点击到屏幕的 P50/P95/P99 和断音次数验收，保留画质与恢复时间比较 |

20 fps、1.5–2 Mbps 等数值只能作为后续实验档位；当前没有验证可直接控制 iPhone 码率的接收端接口。优先测量和部署已经准备好的源头 30 fps 改动。
