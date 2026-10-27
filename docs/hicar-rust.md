# HiCar Rust 迁移与 CarLink 集成

更新：2026-10-26。目标手机范围为鸿蒙 4.2。HiCar 的发现、认证、控制与媒体通路已接入 `carlink hicar run`；Python 实验代码保留为协议参考和差分测试基准，生产 HiCar 后端不再依赖 Python。

## 当前架构

```text
carlink manager
  ├─ 常驻 CarLife USB 输出与车机菜单
  ├─ DeviceStore / 目标选择 / 自动重连 / 单无线后端仲裁
  ├─ CarPlay：独立 OpenXPlay 接收端及其输出适配
  └─ carlink hicar run
       ├─ radio-worker：BlueZ、FE35/FE36、5 GHz 热点、RFCOMM fd
       ├─ session-worker：Nearby、HiChain/HUKS、DMSDP、HiSight RTSP
       └─ media-worker：H.264 直通、AAC 接收、时钟、UIBC
            └─ ARMv7 原生 carlink-audio：AAC → 44.1 kHz 双声道 PCM
```

三个 HiCar worker 都由同一 Rust 可执行文件启动，通过继承的私有 Unix socket 通信。控制与统计使用有界 JSON IPC，压缩视频和实时音频不经过 JSON IPC。radio 使用 `SOCK_SEQPACKET` 与 `SCM_RIGHTS` 传递 RFCOMM 描述符；session 通过已验证的 HiChain/HUKS C ABI 处理认证；media 直接把媒体送入 CarLink 源复用器，再由常驻 CarLife 进程输出。

默认 AAC 解码器为镜像自带的 `/usr/local/lib/carlink/carplay/bin/carlink-audio`。H.264 保持直通，不进行板上视频转码。CarPlay 保留独立 OpenXPlay 适配路径，连接策略由同一 manager 仲裁，任何时刻只由一个无线后端持有资源。

工具链固定为 Rust 1.90，依赖锁定在 `Cargo.lock`。本地修改源码，ARMv7 应用、原生库和固件均由 GitHub Actions 构建；源码归档、许可证、ABI 与 SHA256 检查随产物交付。

## 阶段状态

| 阶段 | 当前实现 | 验证进度 |
| --- | --- | --- |
| R1：协议与认证基础 | 有界分帧、严格 JSON、Nearby/TLV/CRC、DMSDP/HiSight、RTP/ADTS、AES-CTR、时钟、UIBC、HiChain/HUKS FFI | 已完成差分、异常输入、截断、持久化及双向认证测试 |
| R2：真实连接与媒体 | Rust 拥有 BlueZ、热点、TCP、认证、DMSDP/RTSP、音视频与输入生命周期；新增多源混音、麦克风及 modem 服务 | Rust 迁移及基础台架回归已完成；双向音频协议与 ARM 编解码已纳入自动回归。HiCar 独立 HU 通话及 Lexus 待验收，见[完成情况核对](status-audit.md) |
| R3：统一产品管理 | 常驻 CarLife、协议互斥、设备记录、开机重连、菜单切换、断线回菜单、启动恢复 | 已接入并完成台架操作与断线恢复测试；真实触控板和实车场景继续验收 |

源码检查、编译、镜像结构校验、开发板与实车验证分别记录。完整验收清单以 [验证记录](validation.md) 和 [连接管理设计](connection-management.md) 为准。

## 生命周期与恢复约束

- **身份与认证**：保留已有 schema-1 私有身份和 HUKS 持久化数据。HUKS 使用原硬件种子，签名身份按 model ID 和稳定 BR MAC 派生。认证失败或新认证代际撤销旧授权；正常 RFCOMM 认证通道 EOF 保留已建立的 TCP 投屏会话。
- **会话代际**：DMSDP/HiSight 使用独立 token，后端事件携带 `session_epoch`。迟到的旧关闭或旧就绪事件不改变新会话。视频结束立即发送失活状态；只有新会话送出可解码的真实 IDR 后才隐藏恢复菜单。
- **控制与媒体**：DMSDP 包含设备发现、扬声器服务、心跳和时钟协商；扬声器 SETUP 成功即进入音频就绪。RTSP 包含加密协商、能力、SETUP/PLAY、端口和 TEARDOWN。退出先停止媒体和控制，再清理认证与无线资源。
- **热点就绪**：NetworkManager 接受激活请求后，继续等待目标无线网卡同时进入 AP/5745 MHz，并取得 `192.168.49.1/24`。旧 AP 的模式/频率不足以触发 TCP 监听启动。此检查不依赖 `end0` 载波或电脑地址。
- **启动取消**：等待热点 IP 期间检查私有关闭请求或父进程 EOF。取消后删除临时 NetworkManager 连接、恢复此前网络，完成清理后才回复关闭成功，避免切换协议时遗留热点。
- **USB 初始握手**：从实际 AOA bulk ENABLE 起，等待首次真正的 `VIDEO_START`，启用时间上限 30 秒；USB suspend 不计时。超时经现有错误、清理和 manager 退避路径重新枚举。握手完成后，静态画面、暂停或源未播放不重新触发该超时。
- **USB 退避**：连续失败按 1、2、4、8 秒退避，最高 8 秒；成功握手并满足稳定条件后恢复正常重试间隔。USB 与无线重试时钟独立。
- **启动配置**：新镜像首次运行从随附示例创建私有 manager 配置。MFi 身份检查限于 CarPlay 启动，不阻止 HiCar 或车机菜单启动。认证、配对数据和厂商资料不提交到仓库。

## 协议行为与输入

DMSDP SSRC 用途按实测的 `ssrc & 0xfff` 区分。媒体队列有界，停止、格式变化、断线和协议切换都清理旧代际数据。CarLife 视频心跳使用独立期限，连续 PCM、静态画面或等待关键帧均不抑制心跳。

HiSight `left_move` 对应事件 2。确认、返回、主页及方向键保持按下/抬起配对；触摸释放和反馈连接中断有专门回归。Lexus 日常操作以焦点导航为主，物理触控板的返回键映射仍需实车确认。

新增设备使用保存在私有身份中的固定六位 PIN，适合无屏盒子。提交 `a47a9f0` 已加入独立配对页，顶部显示固定配对码、手机操作提示和取消入口。真实 CarLife 画面已验证“添加 → 确认停止旧投屏 → 显示 PIN → 取消”，取消后自动恢复 CarPlay，CarLife PID `968` 与 USB 连接保持不变。PIN 不进入普通日志、公开状态或设备列表；界面只读使用原身份文件。日常已配对重连沿用现有身份和密钥。

## 当前集成验证

当前交付提交为 `a47a9f0`：[应用构建 40281149701](https://github.com/Aki894/CarLink/actions/runs/40281149701) 与 [完整构建 40281831497](https://github.com/Aki894/CarLink/actions/runs/40281831497) 均成功。完整构建包含 native、Rust interop 和 firmware，镜像检查、产物上传、下载后 ZIP 与镜像 SHA256 校验均完成。运行时应用 SHA256 为 `c3dd04d3b79149e992333eca928586a24ed470b995af7f3488d9d1dd6816c3e4`。

`c79c4ba` 的 H3 媒体、无线、原生 IPC、后端四组回归全部通过；PIN 版本复用同一后端实现。真实连接持续 361.0 秒，HiCar 后端 PID `867`、CarLife PID `32183` 保持不变，并完成应用网格点击。另一轮强制停止真实 HiCar 源后，车机实际渲染出 CarLink 菜单，CarLife PID `29631` 保持不变；恢复菜单不需要重建 USB 输出进程。随后切换 CarPlay，HiCar worker 数量归零，用户确认画面、声音和操作正常。最终 PIN 版本断电重启后自动连回 CarPlay，CarLife 握手和菜单显示恢复。

当前台架使用同一台华为手机同时承担 HiCar 源与 CarLife HU。画面与输入可直接验证，音频存在同机 `AUDIO_DEVICE_OUT_PROXY` 路由影响。独立 HU 的可听音频以及 Lexus 曾出现的 80%/0% 连接失败保留为外部硬件验收项目。麦克风、modem 服务、AAC 上行与多音源分流已实现，协议与实时数据通路详见[双向通话与麦克风](calls.md)。

## 历史迁移证据

以下记录对应迁移中间版本，便于回溯问题与比较行为；当前生产入口采用上述完整 Rust 后端。

| 里程碑 | 证据与保留结论 |
| --- | --- |
| R1，`d047e42`，2026-10-25 | [应用 40110088101](https://github.com/Aki894/CarLink/actions/runs/40110088101)：73 项 Rust 测试、1,700 项 Python/Rust 对照；[完整构建 40110351064](https://github.com/Aki894/CarLink/actions/runs/40110351064) 成功。H3 通过绑定、持久化、STS、Rust 自互通和错误 PIN 拒绝；真实私有身份只读加载。 |
| R2 认证状态迁移 | [应用 40127257815](https://github.com/Aki894/CarLink/actions/runs/40127257815)：76 项 Rust 测试、1,700 项基础对照、184 项连接状态对照、3 组 IPC 测试；[完整构建 40127675093](https://github.com/Aki894/CarLink/actions/runs/40127675093) 成功。真实鸿蒙 4.2 消息允许 `version=14.2.0.400` 和省略 `discover_type`，回复仍使用 `A_0`/`discover_type=1`。 |
| 原生认证台架，`01bd418` | 127 秒窗口中约 115 秒媒体活动，记录 2,179 个视频访问单元、4,558 帧 AAC、4,861 个 PCM 块，音频丢包/重复/拒绝为 0。随后保持 CarLife 前台的对照恢复稳定，记录了测试 App 切后台造成 USB `ESHUTDOWN` 的影响。 |
| HiSight RTSP 迁移 | 修复正常 RFCOMM EOF 错误撤销投屏密钥的问题，保留观察到的 SETUP 报文长度兼容边界。私有 worker 在组 SIGTERM 时保留父进程完成 TEARDOWN 所需的 IPC 服务，父进程 EOF 后退出，超时由拥有者收尾。 |

台架测试期间保留 Ethernet SSH，部署使用独立版本目录和回滚记录；真实身份、配对数据与原始抓包留在私有目录。
