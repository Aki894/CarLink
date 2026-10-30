# 后续路线

更新：2026-10-30。运行源码基线 `3c1337f`，构建、镜像校验和台架记录见 [validation.md](validation.md)。完整差额清单与源码依据见 [status-audit.md](status-audit.md)。

CarLife、HiCar、CarPlay 媒体桥接和统一菜单的生产主链路均已迁入 Rust，编解码/绘制复用原生库。鸿蒙范围保持 4.2。Android 4.4 不并行维护。

## 阶段完成情况

- **R1 已完成**：纯协议、HiChain FFI、差分与原生认证互通。
- **R2 已完成**：Rust 媒体及双向音频已台架验收；编码器故障后的有界恢复已通过 host/ARM 故障回归。
- **R3 已完成**：协议独占、默认协议、菜单与音量；首选精确地址、跨协议策略协调及同协议离线轮换已通过进程测试。
- **正式发布已统一**：production 镜像完整安装 manager、HiCar native 和匹配 AIC 驱动，首次身份原子初始化，AIC 网卡自动识别；baseline 独立标识。

## 按顺序推进

1. **发布一致性（已完成）**：完整构建及下载后镜像校验通过，首次配置不依赖历史身份目录。
2. **策略与恢复（已完成）**：双目标、默认/首选边界和编码器故障已纳入 host/ARM 回归；真实多手机场景列入系统验收。
3. **Lexus 专用探针**：以 [实车诊断草案](vehicle-diagnostics-plan.md) 为基础实现 CarLife-only 模式，离线标注声道与按键结果，Mac 可用以太网扩展坞维护。原生菜单和软件音量已完成，探针尚未开发。
4. **实车校准**：80%/0% 连接失败复测、MEDIA/TTS/VR 与音量/焦点、Remote Touch/方向盘、三连返回、语音识别与回声。
5. **系统验收**：三次冷启动、一小时导航/音乐、反复通话、双手机、USB/Wi-Fi/蓝牙故障与断电恢复，记录完整上电到首帧/首音时间。
6. **性能继续优化**：HiCar 时间线/抖动恢复、声道分流、按实车结果校准混音/ducking，分离研究依赖以精简体积。同场景多次采样后评估收益。

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
