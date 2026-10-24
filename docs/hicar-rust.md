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

当前 `app/Cargo.toml` 无第三方依赖，MSRV 为 Rust 1.74。新增 D-Bus、事件调度和加密依赖先核对工具链与 ARMv7 hard-float 支持，再通过 Actions 固定版本构建。保留 H.264 直通和有界音频解码，避免 H3 软件视频转码。

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
