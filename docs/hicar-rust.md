# HiCar Rust 迁移与 CarLink 集成

2026-10-24 决策：第一版无线 HiCar 协议和 CarLife 桥接已有真实手机对照，下一阶段开始迁移 Rust。Python 接收器冻结为可运行的协议参考，产品能力逐步集中到 Rust 主程序。保留鸿蒙 4.2 范围与既有身份、密钥、MFi 和 Ethernet 管理方式。

## 目标结构

```text
carlink 主程序
  ├─ CarLife USB 输出、输入路由、本地设备菜单
  ├─ ConnectionManager / RadioManager / DeviceStore
  ├─ HiCar 后端：发现、认证、会话、媒体、输入
  └─ CarPlay 后端适配：管理现有 OpenXPlay 进程

原生库：现有 Linux HiChain / HUKS C 后端
音频：现有 ARMv7 原生 AAC → PCM 解码器
```

HiCar 原生接入主程序；CarPlay 保留独立 OpenXPlay，由统一后端接口管理。认证库通过 Rust FFI 复用既有 C ABI，检查结构大小、回调线程与对象生命周期。先保留现有音频子进程，优先去掉 Python 热路径和进程间媒体复制。

当前应用工具链与声明版本均为 Rust 1.90。R1 固定引入 serde、serde_json、libloading 和 Unix libc；Cargo.lock、依赖源码归档、许可证及 SHA256 清单随应用打包。加密通过按需加载的 OpenSSL EVP 和既有 HiChain/HUKS C ABI 调用。保留 H.264 直通和有界音频解码。

## 三个实施批次

| 批次 | 代码范围与目标 | 验收 |
| --- | --- | --- |
| R1：协议与认证适配 | 新增 `app/src/hicar/`；迁移 Nearby/TLV/CRC、认证 JSON、DMSDP/HiSight、RTP/ADTS、时钟、UIBC；封装 HiChain FFI | 将 Python 的正向、错误、截断、乱序和重连案例转为 Rust 测试；交叉认证和既有身份加载通过 |
| R2：实际会话与媒体 | BlueZ 发现及 FE35/FE36、认证、热点/TCP、保活和退出；视频直达 CarLife，AAC 经现有解码器输出 PCM，Remote Touch 回传 | 同一手机、同一身份对照 Python/Rust；首配、可信重连、音画、操作、暂停/恢复与退出；记录 CPU/RSS/队列年龄 |
| R3：统一产品运行 | 连接管理器、无线资源独占、设备记录、CarLife 长驻、CarPlay 后端适配、菜单、按钮、自启和固件 | 两手机同时在场、添加与双向切换、失败回退、常电待机、冷启动；独立 HU 音频和 Lexus 实车验收 |

R1 交付可独立验证的协议模块与 FFI 测试；R2 替换实验接收链路；R3 替换正式启动入口。每批保留可回滚版本，源码、完整镜像与硬件结果分别记录。

## 迁移必须保留的协议事实

- 认证身份按 model ID 与稳定 BR MAC 派生，继续加载原持久化身份/HUKS；PAKE 首配与 STS 重连分别处理。
- TCP 按记录边界拆包；HiSight 使用已验证的 AES-CTR 封装；控制、音频和视频分别管理代次。
- DMSDP 在 SSRC 高位携带用途信息，真实流编号为 `ssrc & 0xfff`。序号/乱序按真实编号维护；解码器按协商代次与格式恢复。
- CarLife 视频心跳独立调度，连续 PCM 和等待关键帧期间仍需发送。
- HiSight `left_move` 对应事件 2；确认、返回、主页和方向键保持按下/抬起成对。长时间直接触摸的点击识别保留专项回归，Lexus 以焦点导航为主。
- 停止、格式变化、重连和切换均清理旧队列，异步回调携带会话代号。

## 台架与待验收项

单台华为同时作为 HiCar 信源和 CarLife HU 已反馈桌面显示、点击和左右滑动。AudioFlinger 显示 CarLife 的 44.1 kHz 播放轨进入 `AUDIO_DEVICE_OUT_PROXY`；实际扬声器出声由独立 HU 或实车验证。

继续跟进：无人操作发现/重连、测试应用退出恢复、长时稳定性、独立 HU 音画同步、麦克风/通话，以及 Lexus USB 兼容性和真实触控板输入。按[连接管理设计](connection-management.md)统一收敛。

## R1 完成记录（2026-10-25）

R1 已完成，源码位于 `app/src/hicar/`：有界增量帧解析、严格 JSON、Nearby/TLV/CRC、DMSDP/HiSight、RTP/ADTS、时钟、UIBC、AES-CTR 与 HiChain/HUKS FFI。认证回调限定在线程内；私有文件拒绝符号链接并检查权限，密钥只输出长度和哈希。新增命令为 `carlink hicar codec`、`auth-worker`、`identity-check`，正常 CarLife 启动不会加载认证库。

- 提交 `d047e42` 的 [应用构建](https://github.com/Aki894/CarLink/actions/runs/40110088101)通过 73 项 Rust 单测和 1,700 项 Python/Rust 对照。
- [完整构建](https://github.com/Aki894/CarLink/actions/runs/40110351064)的 native、rust-interop、firmware 全部成功，镜像验证与产物上传完成。
- H3 上重复通过 1,700 项对照和 Rust/Python 双向混合绑定、持久化后认证、Rust 自互通与错误 PIN 拒绝。使用临时密钥目录，不覆盖真实配对。
- 实际已有身份文件已由 Rust 只读加载，硬件种子和派生身份与 Python 一致，文件校验值保持不变。

R2 接入真实 BlueZ、Wi-Fi/TCP 会话和媒体桥接；R3 再统一双协议连接管理。当前 HiCar 无线会话继续由 Python 实验接收器运行，R1 二进制用于协议与认证验证。

## R2 第一段：真实连接状态迁移（2026-10-25）

新增 `app/src/hicar/session.rs`，将 Nearby 通道协商、FE36 端点响应、ResourceManager、热点凭据协商、业务身份握手及 HiChain 会话接入 Rust。`hicar session-worker FD` 通过继承的私有 Unix socket 与过渡适配器通信，读取既有身份文件及 HUKS 库；使用原持久化硬件种子初始化 HUKS，再按 model/MAC 派生签名身份。认证完成前不发送热点凭据，认证失败撤销授权，过期会话代次不影响新会话。

临时适配器为 `experiments/hicar/rust_session.py`，由探针的 `--rust-session-binary` 显式启用。BlueZ 注册、socket 读写与帧边界、DMSDP/HiSight 控制及媒体目前仍在 Python；会话密钥只通过私有 IPC 交给现有媒体适配器，不进入 argv、stdout 或普通日志。正式开机服务保持原部署。

实机修正了一处 R1 测试样本盲点：鸿蒙 4.2 请求中的 `version` 是手机版本 `14.2.0.400`，且省略 `discover_type`；设备响应才使用 `A_0`/`discover_type=1`。Rust 现在区分两者，并增加相同字段形状的合成回归。

验证记录：

- [应用构建 40127257815](https://github.com/Aki894/CarLink/actions/runs/40127257815)：76 项 Rust 单测、1,700 项基础协议对照、184 组连接状态对照及 3 组损坏 IPC 测试通过。
- H3 与 CI QEMU 均通过私有 IPC 下的 Rust/Python 首配、持久化后 STS 重连、密钥一致、重复身份拒绝、认证前凭据隔离及过期代次检查；合成身份文件保持不变。
- 最终版本实机使用原配对完成操作 17/18 的 STS 认证，随后建立 TCP 4005/4006/4007、HiSight 视频和输入通道。用户确认 HiCar 桌面显示、画面操作正常。
- [完整构建 40127675093](https://github.com/Aki894/CarLink/actions/runs/40127675093)的 native、rust-interop、firmware 均成功，镜像校验与上传完成；artifact `12573769397` 已下载，ZIP SHA256 `7c8b6779482e0b56aed9d9c451ef39343c4bd40568744fd55834e04eb7484eb6`，包内镜像 SHA256 `e8c27a72f03d2a140fc8a44d2a5089cb2c1993ca817c1eba9ddf193f6c96c211`。

R2 后续先迁移 DMSDP/HiSight 控制与媒体数据通路、AAC/PCM 和输入，再移除过渡适配器的 BlueZ 与网络管理。新增真实首配、重复重连与独立 HU 音频验收；R3 继续负责双协议统一仲裁与开机集成。

### 最终台架采样与稳定性边界

第 127 轮使用最终 `01bd418` 应用同时承担 Rust 认证后端和 CarLife USB 输出，未使用临时首包规范化代码。约 115 秒媒体采样记录 2,179 个视频单元、4,558 帧 AAC、4,861 个 PCM 块（571 个含非零数据）和 442 次输入，音频丢包/重复/拒绝计数均为 0。用户确认保持 CarLife 前台时新版一分钟内稳定。日志中无 USB 写错误，记录一次退出码 0 的 CarLife 输出进程退出及自动拉起；长时间和多次重连回归继续保留。

此前第 125 轮在手机 App 切换过程中出现多次 USB bulk `ESHUTDOWN`；换旧输出做对照后，再用新版固定前台，连接恢复稳定。今后台架固定让 CarLife HU 留在前台，通过投屏界面或媒体会话操作源播放器。此次连续 AAC 与 PCM 建立已验证，同手机兼任 HiCar 源和 CarLife HU 的扬声器路由继续采用既有独立 HU 验收安排。

本轮测试后已恢复正式 `carlink` 服务，NRestarts=0、`end0` 管理可用，临时 Rust worker 已退出，手机亮屏超时设置已恢复。正式 USB 输出和开机路径继续保留原部署。

## R2 第二段：HiSight RTSP 状态机

`app/src/hicar/hisight.rs` 将 ANNOUNCE 加密协商、OPTIONS、能力应答、SETUP/PLAY、端口选择、保活回复和 TEARDOWN 迁入 Rust。生产 `session-worker` 复用已经认证的 HiChain 密钥，为每个 HiSight 实例分配独立代号；认证撤销会销毁对应状态，旧实例的清理请求不会关闭新实例。密钥和控制报文只经过现有私有 socketpair；普通日志只输出固定阶段名。视频继续在原通路中直通，不进入 JSON IPC。

真实回归发现并修复了认证通道交接边界：手机正常结束 RFCOMM 认证后，已经建立的 TCP 媒体会话继续持有自己的密钥；关闭认证通道只撤销新连接授权。原候选版在约 60 秒后的 HiSight 保活报文处报 `stale HiSight session` 并退出，新增的原生 IPC 测试已在旧候选版复现。新认证代号和显式认证失败仍会销毁旧媒体会话。

私有 Rust worker 在整个后端进程组收到 SIGTERM 时继续服务父进程的退出 IPC，让父进程先完成 RTSP TEARDOWN；父进程关闭接口或退出后，worker 读到 EOF 并清理密钥。父进程仍保留超时后的 SIGKILL 收尾。原生联测包含发送 SIGTERM 后继续处理加密请求及随后关闭接口的流程。

Python `HiSightSession` 现在只负责低频控制 IPC 的适配，socket、视频接收、AAC 解码与输入仍逐段迁移。Rust 协商保留源端 SETUP 响应缺少双 CRLF、Content-Length 计算异常等已观察到的兼容行为，并为记录长度、队列及序列号设置边界。认证先关闭时，TEARDOWN 错误会被收尾逻辑吸收，继续释放媒体和输入连接。

本轮同时把 CarLife 重试时钟与无线后端重连时钟分离。CarLife 进程连续短时退出时，以 1、2、4、8 秒退避，最长 8 秒；运行满 30 秒后恢复 500 ms 的普通恢复间隔。此策略给 USB 主机明确的脱离时间，减少快速重复枚举，保留当前无线会话。测试手机的旧 USB 会话曾需要重新打开 CarLife App，恢复结果另见[验证记录](validation.md)。

| R2 部分 | 当前实现 |
| --- | --- |
| Nearby、FE36 协商、HiChain 认证、热点凭据协商 | Rust 状态机，原有持久化身份 |
| HiSight RTSP 加密协商、端口、播放及退出 | 本轮迁入 Rust |
| BlueZ 注册、Wi-Fi 与 TCP socket IO | Python 适配层，待移除 |
| DMSDP 控制会话、音频服务控制、媒体与输入实时通路 | Python 在线实现；R1 Rust 编解码器待接入 |
| AAC 解码与 PCM 输出 | 原生解码子进程，生命周期待接入 Rust |

因此 R2 整体仍在进行。下一段先迁移 DMSDP 控制与音频服务状态，再把视频/音频/输入 IO 接到 Rust，最后移除 Python BlueZ 与进程胶水。每段以同一手机、同一身份和可回滚产物完成对照。
