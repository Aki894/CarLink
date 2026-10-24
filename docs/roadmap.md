# 后续路线

当前基线和构建见[验证记录](validation.md)，各主题见[文档目录](README.md)。继续 Linux 主线：Rust CarLife MD、独立 OpenXPlay 接收端、Python 媒体桥接；Android 4.4 暂不并行维护。

## 优先级与验收

2026-10-23 调整：当前开发优先推进 [Linux 无线 HiCar](hicar.md)，范围限定鸿蒙 4.2。现有 CarPlay/CarLife 台架基线保留；下列 Lexus 项目在具备实车条件时优先复验，它同时影响 CarPlay 和 HiCar 两条输入路线。

HiCar 当前处于原计划第 4 阶段：发现、首次认证、热点/TCP、设备/显示服务、连续 H.264 和基础 UIBC 输入已实测，可操作真实 HiCar 桌面。音频及可信重连继续实施。近期顺序为：

1. 扩大 UIBC 输入与断线恢复验证；解决真实手机的 STS 签名兼容，减少重复首配。
2. 接入音频和周期时钟同步，记录音画同传及断线恢复；当前 800×480 H.264 已可直接解码，优先复用直通链路。
3. 对接现有 CarLife 媒体接口；华为手机作为 HiCar 信源，另用第二台 Android HU 或实车验证输出，电脑接收器先验证媒体格式和流量。
4. 固定 PIN 首配、可信重连、冷启动及异常恢复，加入 CarPlay/HiCar 的单会话互斥策略，再集成自启与固件。

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
