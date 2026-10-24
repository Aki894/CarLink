# 后续路线

当前基线和构建见[验证记录](validation.md)，各主题见[文档目录](README.md)。继续 Linux 主线：Rust CarLife MD、独立 OpenXPlay 接收端、Python 媒体桥接；Android 4.4 暂不并行维护。

## 优先级与验收

2026-10-24 调整：当前开发优先推进 [Linux 无线 HiCar](hicar.md)，范围限定鸿蒙 4.2。[双协议连接与设备菜单设计](connection-management.md)已整理，采用统一选择、单活动手机和长驻 CarLife。现有 CarPlay/CarLife 台架基线保留；下列 Lexus 项目在具备实车条件时优先复验，它同时影响 CarPlay 和 HiCar 两条输入路线。

HiCar 原计划第 4 阶段已实测发现、首次认证、免 PIN STS 重连、热点/TCP、连续 H.264、基础 UIBC、AAC 接收与 H3 原生解码、周期时钟及双向保活。当前为独立手动实验接收器，下一步进入 CarLife 转发与产品集成：

1. 对接现有 CarLife 视频、音频和输入接口；优先尝试同台华为通过无线提供 HiCar、通过 OTG 运行 CarLife HU 的台架，检查前后台、音频焦点与队列。随后用独立 HU 验证。
2. 拆分 CarLife 与信源后端生命周期，引入统一连接管理器、目标设备记录、无线资源独占、会话代号和切换回退。
3. 实现车机设备菜单，复用 Remote Touch 焦点导航；实体按钮呼出，原车快捷键通过实车抓包后确定。继续隐藏 CarPlay 品牌图标。
4. 完成 HiCar 无人操作发现/自动重连、多手机优先级、冷启动和常电待机验证，再集成自启与固件。

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
