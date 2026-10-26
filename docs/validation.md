# 验证与部署记录

## 2026-10-26：Rust HiCar 完整后端与固定 PIN 台架交付

交付提交 `a47a9f0cdf307ff1f17518e8c90049821fee528d`，部署目录 `/opt/carlink/releases/r2-a47a9f0`；通过 SSH 更新运行程序并保留回滚目录，MFi/HiCar 身份文件校验保持一致。生产 HiCar 使用 `carlink hicar run`，由 Rust 管理无线、认证、DMSDP/RTSP、媒体和输入，Python 保留作参考与测试。

- [应用构建 40281149701](https://github.com/Aki894/CarLink/actions/runs/40281149701) 成功；[完整构建 40281831497](https://github.com/Aki894/CarLink/actions/runs/40281831497) 的 native、rust-interop、firmware 全部成功，镜像结构/依赖/校验和检查及上传完成。
- 镜像 `CarLink-refresh-39459953269-a47a9f0c.img.xz` SHA256 `8a7f3149996e263a87779b5f820992b7ed5acee73066a9d70102d60f022e4a74`；下载的固件 ZIP 为 436,560,040 字节，SHA256 `28ccd87aa9c9afa802a88279782984c7dc2094bbf3f30d7744d0906317c9dcb0`，GitHub artifact 摘要及内部镜像摘要均匹配。
- H3 已通过媒体、无线、原生私有 IPC、后端四组回归；真实 HiCar → H3 → 华为 CarLife 运行 361.0 秒，采样失败为 0，HiCar/CarLife 进程保持不变，应用网格点击有反馈。
- 强制停止真实 HiCar 源后，实际画面返回 CarLink 菜单，CarLife PID `29631` 和 USB 连接保持不变。切到 CarPlay 后 HiCar worker 为 0，用户确认画面、音乐和操作均正常。
- 最终版本已验证断电后开机自启及 CarPlay 自动重连；固定 PIN 配对页经真实菜单点击显示，取消后按原策略恢复 CarPlay，CarLife PID `968` 与 USB 连接保持不变。配对不生成或轮换原有 PIN。
- 补齐热点 IPv4 就绪等待、启动阶段取消清理、会话代际过滤、静态关键帧恢复，以及 AOA bulk 启用后 30 秒未完成初次 `VIDEO_START` 的 USB 握手恢复。成功连接后的静态/暂停会话不重启。

本轮按用户要求交付台架版本。Lexus 先前的 80%/0% 连接问题、实体触控板映射，以及独立 HU 的 HiCar 音频/麦克风/通话，留待实车验收。公开文档不保存 PIN、身份或配对数据；本地回执在私有 `.build/hicar-research/rust-pin-final/`、`rust-pin-firmware-final/` 和 `rust-owner-release/`。详细架构与阶段记录见 [HiCar Rust 集成](hicar-rust.md)。


## 2026-10-25：R2 第二段，Rust HiSight 控制与退出修复

当前部署源码 `0d29152d8319e66a1d46140a0876d33449fde7c1`，板上目录 `/opt/carlink/releases/r2-0d29152`；应用 SHA256 `de7f962fc87be7de15114a658189914b935e407e33475cb6cf1e9350c7fdd857`。通过 SSH 热部署，MFi 和 HiCar 身份文件校验保持一致，保留对应回滚目录。

- [应用与接收端 40201394421](https://github.com/Aki894/CarLink/actions/runs/40201394421)成功。Rust 103 项测试、145 组 HiSight 对照和 10 组异常输入通过。
- [完整构建 40201974697](https://github.com/Aki894/CarLink/actions/runs/40201974697)的 native、rust-interop、firmware 全部成功，镜像结构检查及上传完成。下载复验 ZIP SHA256 `ac183e06e437eb19f4b706380324a13e7b250ab652559010a41b272f1bf61d51`，内含 `CarLink-refresh-39459953269-0d29152d.img.xz` SHA256 `65578076652701788e22ce6a461a1af37087ea1b8061d159a2a67257ad4993cc`。
- H3 四组测试全部通过：HiSight 对照、原生认证及私有 IPC、连接管理、原生菜单与复用器。包含认证通道正常结束后的加密保活、新认证代号撤销旧媒体、SIGTERM 后继续服务退出 IPC、最终 EOF 清理。菜单 PCM 合成测试最大间隔 25.7 ms。
- 真实华为 HiCar → H3 → 华为 CarLife 持续观察 361.9 秒，2 秒采样，失败记录为 0；HiCar PID `26578`、CarLife PID `27027` 及代号均保持，USB 一直连接。ADB 点击应用列表后画面正常更新。随后切到 CarPlay，旧 HiCar 私有 worker 数量为 0，CarLife PID 仍为 `27027`。
- 已切回 HiCar，恢复自动连接、30 秒生产连接窗口及手机原来的 10 分钟亮屏超时；临时诊断 wrapper 已移除。切回后保持同一 CarLife PID，新的 HiCar 后端和单个私有 worker 工作正常。
- 初始候选 `2727997` 的真实回归发现约 60 秒后断流，日志和新增 H3 用例均复现 `stale HiSight session`。修复了认证通道结束时误销毁媒体状态，以及进程组停止时 worker 先退出、父进程无法完成 RTSP TEARDOWN 的顺序问题。
- CarLife 连续短时退出采用 1、2、4、8 秒退避，并独立于无线重连计时。华为测试 App 在热部署或切换后台后仍可能留下旧 USB 会话，本轮通过重新打开 App 恢复；测试固定前台。手机锁屏会阻止发现确认，已使用用户授权解锁后继续测试。

本轮完成 HiSight 控制迁移；R2 的 DMSDP、媒体/音频/输入实时 IO 和 BlueZ 适配仍按[迁移计划](hicar-rust.md)推进。独立 HU 音频及 Lexus USB/实体按键继续留在硬件验收项。产物、校验、回归及失败复现保存在私有 `.build/hicar-research/r2-hisight-exit-final/`、`r2-hisight-exit-firmware-final/` 和早期候选目录。

## 2026-10-25：R3 断线恢复、开机重连与三连返回键

已部署源码 `4d2892037a4b4319784773a487ab5b16bdd71101`，应用 SHA256 为 `5becf8f8a23f4aa93c85d7eddd6d9ed169783ce8743c2e108920f7d210cf7f6f`。板上版本目录 `/opt/carlink/releases/r3-4d28920`，对应回滚目录 `/var/lib/carlink/manager/rollback-r3-4d28920`；MFi、HiCar 身份文件校验保持一致。

- [应用与接收端构建 40192610690](https://github.com/Aki894/CarLink/actions/runs/40192610690)成功，两个产物的 GitHub ZIP 摘要和文件清单均校验通过。
- [完整固件构建 40193164958](https://github.com/Aki894/CarLink/actions/runs/40193164958)的 native、rust-interop、firmware 全部成功，镜像结构检查与上传完成。下载后再次校验 ZIP 和镜像清单：`CarLink-refresh-39459953269-4d289203.img.xz` SHA256 `531ede4205c340002fadc9f93245ca25e58fe7143c24a5d7384e31011a81858f`。本轮通过 SSH 热部署应用，保留原有系统。
- H3 运行当前 `test-manager.py` 和 `test-manager-menu-control.py` 均通过：开机偏好与暂停清除、九次协议切换、三连返回、断线回菜单、自动尝试另一协议、CarLife PID 保持及无线后端无重叠。菜单 PCM 合成测试最大间隔 29.8 ms。
- 实际华为 CarLife 端已显示新菜单和 HiCar 桌面。向 HiCar 后端注入 SIGTERM 后，菜单恢复，随后自动接上现场 iPhone；CarLife PID 保持 `3714`。
- 整机重启前明确设为暂停，重启后 `paused=false`，先尝试已保存的 CarPlay，再轮询 HiCar。系统启动耗时 22.816 秒。华为本轮出现连接确认，经确认后恢复 HiCar 画面及 USB 输出；手机无确认的冷启动重连还需继续观察。
- 同一华为同时承担 HiCar 手机端和 CarLife 车机端时，本轮切到系统 HiCar 设置再返回后，出现 `USB bulk write: Cannot send after transport endpoint shutdown (os error 108)`，管理器持续重建 CarLife 输出。完全停止并重新打开华为 CarLife App 后，循环停止；HiCar 后端 PID `5500` 保持，CarLife PID `6762` 持续超过两分钟且 `usb_connected=true`。保留此复现条件继续观察测试 App 的旧 USB 会话恢复。

投屏期间连续按三次返回键打开菜单，相邻按键间隔不超过 350 ms；普通返回因此最多延后 350 ms。Lexus 实体触控板的原始返回键消息待实车验证。H3 测试、下载回执、镜像校验记录保存在私有 `.build/hicar-research/r3-recovery-final/` 和 `r3-recovery-firmware-final/`。

状态截至 2026-10-25。这里汇总已完成的检查，日常运行见[运行说明](live-forwarding.md)。

下表保留正式部署的 CarPlay/CarLife 基线。最新 HiCar 实验已接入 CarLife 视频、AAC/PCM 和 UIBC，79 项测试通过；[完整构建 40105347479](https://github.com/Aki894/CarLink/actions/runs/40105347479)成功，镜像已下载并校验 ZIP 与压缩镜像清单。实测和产物摘要见 [HiCar 构建与实测](hicar.md)。下一阶段按 [Rust 迁移计划](hicar-rust.md)推进；HiCar 当前仍手动实验运行。

## 2026-10-25：Rust R2 连接状态接入实机

[应用构建 40127257815](https://github.com/Aki894/CarLink/actions/runs/40127257815)与[完整构建 40127675093](https://github.com/Aki894/CarLink/actions/runs/40127675093)成功。76 项 Rust 单测、1,700 项基础对照、184 组连接协商、3 组损坏 IPC 及跨实现认证通过；镜像已下载校验。Rust 真实既有配对认证、HiCar 桌面与输入已通过台架，连续 AAC/PCM 已建立，用户确认固定 CarLife 前台时一分钟稳定。详细计数、输出进程重拉记录和后续稳定性回归见 [R2 记录](hicar-rust.md)。正式开机服务仍保持原部署。


## 2026-10-25：Rust R1 与连接名称更新

R1 的 73 项 Rust 单测、1,700 项差分、混合认证和已有身份只读验证见 [Rust 迁移记录](hicar-rust.md)。[改名版应用构建 40124305351](https://github.com/Aki894/CarLink/actions/runs/40124305351)成功，接收端 artifact `12572404057` 的 63 个成员、应用 artifact `12572199134` 的 68 个成员均已下载并校验。

[完整构建 40124788949](https://github.com/Aki894/CarLink/actions/runs/40124788949)的 native、rust-interop、firmware 全部成功，镜像结构、应用/接收端清单、依赖源码和许可证检查通过，产物上传完成；镜像 artifact 为 `12572024913`，已下载复核 ZIP SHA256 `e24388c9a28a22e201ebe1468a07c0e81b2eb8ced5a0936048b1632653d40970` 和包内镜像 SHA256 `365d0de00bfe7ee6a43b5e9478514e4273dda27a1e719041df2ba67565278c24`。

板端接收端已热更新为 `9b14895`：蓝牙 Alias、5 GHz 热点 SSID、CarPlay 的 iAP2/AirPlay/mDNS 名称统一为 **CarLink**；HiCar 默认身份名、BLE LocalName、热点 SSID 与设备信息名也统一为 **CarLink**。热点位于 CN channel 149 / 5745 MHz，服务 active、NRestarts=0，有线 `end0` 管理保持可用。HiCar 新广播在 H3 上成功注册并正常退出，三份已有身份改名后 PIN 和硬件种子一致。

用户随后在 iPhone 上连接改名后的 CarLink，确认 CarPlay 画面和声音正常。

生产 CarLife 二进制继续使用下表的已验证版本；R1 新二进制在独立临时路径通过板端测试。接收端回滚目录为 `/var/backups/carlink/receiver-20261024T192132Z/runtime`，部署收据为 `/etc/carlink-receiver-deploy.json`。静态主机名 `carlink-h3` 保持作为内部管理标识，用户可见的 pretty hostname 为 CarLink；协议认证用的 MAC、model ID、USB/AOA 标识保持原值。


## 当前基线

| 项目 | 版本 / 结果 |
| --- | --- |
| CarPlay 接收端源码 | `422ab12600da3ff1815d4f4d7479a3ec7f6412eb` |
| CarLife 握手实现 | `2785fb3`；板上二进制 SHA256 `efdceea8f6f18ed2571b05e1527dda59a1524526be71cdc0b9cb8c27140a9289` |
| 应用与接收端 CI | [39818918089](https://github.com/Aki894/CarLink/actions/runs/39818918089)，测试、ARMv7 编译与上传成功 |
| 完整固件 CI | [39885833239](https://github.com/Aki894/CarLink/actions/runs/39885833239)，镜像内容检查与上传成功 |
| 镜像 artifact | `carlink-h3-refreshed-image`，ID `12472691421`，ZIP 420,740,370 字节 |
| 镜像 ZIP SHA256 | `f0389354934650bd0c82d563c5328293db0c1c037a3d29338c7d2bade513ccc5`；烧录前另校验包内镜像清单 |
| 接收端 artifact | `12443079783`，本地/板端 62 个文件校验通过，已热部署 |
| 板端收据 | `/etc/carlink-receiver-deploy.json` |
| 板端接收端回退 | `/var/backups/carlink/receiver-20261022T171141Z`；目录名时间取自当时板端时钟 |

镜像以 [39459953269](https://github.com/Aki894/CarLink/actions/runs/39459953269) 为基线，保留启动程序和 `6.18.57-current-sunxi`，集成 [39670639717](https://github.com/Aki894/CarLink/actions/runs/39670639717) 的匹配 AIC 驱动。当前开发板通过热部署更新接收端，最新完整镜像尚未重新烧录。

## 已完成的检查

| 层次 | 结果 |
| --- | --- |
| 源码与合成测试 | Rust 协议/状态/输入测试、Python 帧协议、ARM 音频重采样及真实转发进程的合成闭环通过 |
| 构建与镜像 | ARMv7 ELF/ABI、启动配置、镜像结构、应用/内核/模块/驱动、源码/许可和校验清单通过 |
| 无线硬件 | AIC 5 GHz 客户端、HTTPS、蓝牙扫描、iPhone 热点关联和重启恢复通过；XR819 已禁用 |
| USB | AOA 51/52/53、`18d1:4ee7 → 18d1:2d00`、bulk 拆分传输和连续三轮枚举通过 |
| 独立 CarLife 媒体 | 华为 OTG 测试端显示彩条、播放间歇提示音；1280×720@30 合成视频与 PCM 通过 |
| 独立 CarPlay 接收 | iPhone 配对、MFi、800×480 H.264、48 kHz 双声道 AAC、反向触控及重连通过 |
| 实时桥接 | iPhone → H3 → 华为 CarLife 的画面、连续音乐、触屏返回通过 |
| 30 fps 对照 | 用户确认播放/暂停音乐时滑动和点击都流畅，声音连续 |
| 过渡动画 | 录制源视频中复现约三秒停顿；完整重启 iPhone 后恢复，减弱动态效果可关闭 |
| OEM 入口移除 | 省略字段曾留下空图标；参考 DiPlay 显式 `oemIconVisible=false` 后，用户确认图标和占位均消失 |
| 启动 | 一次软件重启由 100.878 s 降至 22.872 s，CarPlay ACTIVE 28.861 s；详细计时见启动文档 |
| Lexus 实车 | 首轮 80% / 重试 0% 失败；握手修复后的实车复验尚未完成 |

## 关键历史里程碑

| 日期 | 内容 | 来源 |
| --- | --- | --- |
| 10-21 | 内存 MemTotal 501244 KiB；有线接口修正为 end0，直连 SSH 可用 | 基线 [39459953269](https://github.com/Aki894/CarLink/actions/runs/39459953269)、独立校验 [39546347234](https://github.com/Aki894/CarLink/actions/runs/39546347234) |
| 10-21 | AIC `main` profile Wi-Fi/蓝牙/AP 通过，板载 XR819 禁用 | [39549290693](https://github.com/Aki894/CarLink/actions/runs/39549290693)；芯片日志 `chip_id=7, chip_mcu_id=1` |
| 10-21 | 原生 CarLife USB 与 WinUSB 台架通过；MUSB RX 改为 32 个 512 字节 AIO 预投 | [39610764455](https://github.com/Aki894/CarLink/actions/runs/39610764455) |
| 10-21 | 固定 H.264 / PCM 在华为车机应用实际播放 | [39628785953](https://github.com/Aki894/CarLink/actions/runs/39628785953) |
| 10-22 | 修复 ARM 蓝牙 quirk 返回探针，改用已有 usb_submit_urb 路径；配对及软件重启通过 | [39670639717](https://github.com/Aki894/CarLink/actions/runs/39670639717) |
| 10-22 | 开启 Opus 语音能力后 iPhone 音频格式初始化恢复；接收 AAC 媒体流 | [39678214827](https://github.com/Aki894/CarLink/actions/runs/39678214827)、[完整固件 39678818571](https://github.com/Aki894/CarLink/actions/runs/39678818571) |
| 10-22 | 实时桥接、启动精简；修复触控扩展字段、批量 USB 外层帧、AudioTrack 反复重建 | [39728191942](https://github.com/Aki894/CarLink/actions/runs/39728191942)、[完整固件 39728628096](https://github.com/Aki894/CarLink/actions/runs/39728628096)；用户确认可点击且正常出声 |
| 10-22 | Lexus 触控板输入及完整键脉冲合成测试 | [39729533171](https://github.com/Aki894/CarLink/actions/runs/39729533171)、[完整固件 39730049076](https://github.com/Aki894/CarLink/actions/runs/39730049076) |
| 10-22 | 持久日志修复与受控重启取证 | [完整固件 39801437061](https://github.com/Aki894/CarLink/actions/runs/39801437061) |
| 10-22 | `MD_PEER_ACCEPT` 与挑战认证解耦，原生 ARM 和华为 4.1 握手通过 | [39805684763](https://github.com/Aki894/CarLink/actions/runs/39805684763)、[完整固件 39807108167](https://github.com/Aki894/CarLink/actions/runs/39807108167) |
| 10-23 | 部署初始 30 fps 声明及实际 FPS/Mbps 统计，用户操作与声音对照通过 | 接收端 artifact `12436739878`，来源 `2785fb3` |

## 后续验证

1. Lexus 点火到首次连接、失败后重试、完整持久日志和原始触控板输入。
2. 至少三次冷启动、长时间音乐/导航、退出重连和热点/蓝牙共存。
3. 电话、Siri、麦克风回传及提示音/音乐混音。
4. 内存压力、SD 稳定性和独立供电条件。

本地证据集中在 `.build/evidence/`、`.build/car-investigation/`、`.build/apk-analysis/`、`.build/transition/`；私有身份和配对留在 `.build/private/`，不进入公开仓库。
