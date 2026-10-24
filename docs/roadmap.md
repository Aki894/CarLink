# 后续路线

当前基线和构建见[验证记录](validation.md)，各主题见[文档目录](README.md)。继续 Linux 主线：Rust CarLife MD、独立 OpenXPlay 接收端、Python 媒体桥接；Android 4.4 暂不并行维护。

## 优先级与验收

2026-10-24 调整：当前开发优先推进 [Linux 无线 HiCar](hicar.md)，范围限定鸿蒙 4.2。[双协议连接与设备菜单设计](connection-management.md)已整理，采用统一选择、单活动手机和长驻 CarLife。现有 CarPlay/CarLife 台架基线保留；下列 Lexus 项目在具备实车条件时优先复验，它同时影响 CarPlay 和 HiCar 两条输入路线。

HiCar 第 4 阶段及第一版 CarLife 桥接已完成台架验证：真实桌面、点击/滑动、连续 AAC 解码与 USB 播放轨建立。单手机回放进入 HiCar 虚拟音频设备，实际扬声器由独立 HU 验收。[Rust 迁移](hicar-rust.md)的 R1 已完成，当前进入 R2：

1. R1（完成）：纯协议与 HiChain FFI 已迁入 Rust；73 项单测、1,700 项 Python/Rust 对照及混合认证通过，已有身份只读兼容。
2. R2（进行中）：Rust 连接状态、既有配对认证与热点协商已接入实机，用户确认桌面及输入。下一段迁移 DMSDP/HiSight、视频、AAC/PCM 和触控板输入，随后移除过渡 BlueZ 适配器；继续做真实首配、重连与异常对照、独立 HU 音频验收。
3. R3：保持 CarLife 常驻，引入双协议连接管理、无线独占与设备记录；实现车机菜单和按钮，原车快捷键实车确定。继续隐藏品牌图标。
4. 完成无人操作发现/重连、多手机切换、冷启动和常电待机验证，再集成生产自启与固件。

原有产品收尾项保留如下：

1. **Lexus CarLife 复验**：保存从点火到连接、首次失败和一次重试的完整日志，定位 AOA、bulk、协议版本、STATISTIC_INFO、接受通知与 INIT/START。按阶段决定是否需要应用层握手超时恢复。
2. **Remote Touch 实车**：记录原始相对位移和按键，校准焦点方向、步长、双击和机械确认去重。当前触屏和合成触控板用例已通过。
3. **稳定性与启动**：三次冷启动、一小时音乐/导航、退出重连、断电日志保留、Wi-Fi/蓝牙共存、内存与 SD 稳定性；单独记录 bootloader、系统就绪、首帧和首音时间。
4. **电话与麦克风**：独立完成 CarLife 语音角色/输入协商，再验证 Siri、通话及媒体恢复。
5. **继续性能优化**：在现有 30 fps 基线上测量完整缓存和消费速度，再决定进程内音频解码或自适应负载方案；详细参考见[性能文档](performance.md)。

## 本地参考版本

| 项目 | 提交 | 用途 |
| --- | --- | --- |
| OpenXPlay Relay | `6a7cdab585113e29422f2306e90fdb5a789c318a` | 当前接收端；构建锁定以 sources.lock.json 为准 |
| OpenXPlay Stream | `356a3f56c0e0b4e839b91387bf2b8cfa0b0a8537` | OXP 输出 |
| DiPlay 用户分支 | `9b26935f36c32417811eb6319511c3050ba35252` | 媒体直通、音频和隐藏 OEM 入口 |
| CarProjection | `4241e52361af09fe58a82bf54ddde4903c46b543` | 旧协议、Lexus Remote Touch |
| HeadSwap | `99e888cf546ec5f040d4de377c59b478186afd5b` | 组帧、握手和输出负载 |
| Carlife_on_box | `d9397d90a1f32670f034eb07cdbe28d887a179d8` | MD/HU 台架和状态处理 |
| catplay | `bc6edae49554f2d0160b18ed18fc7b7376353115` | Linux 接收、时钟及音频架构 |

[AOA 1.0](https://source.android.com/docs/core/interaction/accessories/aoa)、[Linux FunctionFS](https://docs.kernel.org/usb/functionfs.html) 为 USB 实现依据。Armbian 框架与配置固定于 sources.lock.json；内核分支、APT 软件包的解析结果随当次构建记录保留。

参考仓库位于忽略的 research/repos/，厂商资料和原始媒体不进入公开仓库。
