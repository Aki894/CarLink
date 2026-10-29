# 验证与部署记录

## 2026-10-30：原生菜单入口、软件音量与 CarPlay 音频突发修复

运行源码 `1aa023d3927e28b802bc9c166eaed2bc7474baca` 已部署至 `/opt/carlink/releases/calls-1aa023d3927e`，应用 SHA256 为 `de3361729e155f050da778d714f92129cf0060c5cb2099de38bfd95534065ce9`。部署后核对 11 个运行文件、菜单动态库映射和子进程路径；上一运行版本保留于 `/var/backups/carlink/calls-20261028T183249Z`，目录名使用板端时钟。

- CarPlay 使用上游车形图标，名称为“CarLink菜单”；HiCar 保留系统返回车机图标和名称，接入相同本地菜单。HiCar 手机实际发送 DMSDP module 3 / command 103 的显示服务 action 202；兼容旧 command 107，并去重和回复其序号。返回投屏发送显示服务 action 203，普通 HiSight PAUSE 不触发菜单。
- 投屏期间打开菜单保持无线会话与音乐；“返回投屏”位于第一行，返回键也可恢复画面。声音页提供 5% 步进的软件音量，设置持久化，PCM 使用约 20 ms 增益渐变；没有独立静音功能，麦克风上行保持原幅度。
- 用户台架确认：CarPlay 图标、调音量、返回画面正常；HiCar 图标可进入和返回菜单，音乐不中断。音频修复版再次试听，用户确认没有卡顿。
- CarPlay 卡顿诊断显示合法 AAC 批量到包触发 `input_queue_full`：队列 16 包、队首等待仅约 0.1–0.9 ms，解码器被反复重建。现将上游预送上限从 1000 ms 降至 200 ms，保留 400 ms 协议播放锚点和 500 ms 解码积压上限；Rust 输出在处理一批 OXP 音频包时同步推进非阻塞解码管道。
- 新版两轮各 45 秒状态采样覆盖音乐播放、菜单调音量和返回投屏：播放开始后解码器启动次数保持 1，故障次数为 0，CarLife PID `12924`、CarPlay 后端 PID `11621` 均保持不变，USB 持续连接。调音量经过 55% / 40% 后返回 50%，音乐与投屏继续。
- [应用构建 40909769468](https://github.com/Aki894/CarLink/actions/runs/40909769468) 成功：203 项主程序测试、加密 DMSDP 菜单闭环、真实输出进程菜单事件、原生解码器 18 包突发、持续 AAC 输出间隔，以及上游 48 包预送与暂停/刷新/恢复回归通过。
- [完整固件 40911102836](https://github.com/Aki894/CarLink/actions/runs/40911102836) 的 native、rust-interop、firmware、镜像结构校验及产物上传全部成功。

本地完整下载后，GitHub artifact digest、包内镜像 SHA256SUMS 和检查报告均核对通过。镜像 `CarLink-refresh-39459953269-1aa023d3.img.xz` SHA256 为 `3c030b068943494ee6f87bd6569503356bc23a33665203eeeeab43a60c551c59`；ZIP SHA256 为 `fa3ed41a2005c02cb1b40289df8439942989d784eed3f0e8ee8a55784b93f266`。

本轮为台架验证；Lexus 实车声道与实体触控板探针另行开发。

## 2026-10-29：Rust CarPlay 媒体桥接与资源优化

运行源码 `90ee05a450b6ef80cb3f309bcb8ba7c41a5cc49c` 已部署至 `/opt/carlink/releases/calls-90ee05a450b6`。主程序 SHA256 为 `b7184d5676d951813bf3f1bbb761cef92ffa1e3d5280d74f753359e9dcd514b0`；部署后核对 11 个运行文件、菜单动态库映射及各子进程路径。旧运行版本保留于 `/var/backups/carlink/calls-20261028T175858Z`（目录名使用板上时钟）。

- **运行主链路**：CarPlay 输出进程已从 Python 切换为 `carlink carplay-output`；实机 CarPlay 与 HiCar 两种模式下，服务内没有 Python 进程。原生音频工具、libopus 与菜单动态库继续使用；Python 参考、诊断、测试和首次配置入口保留。
- **用户台架验收**：用户确认新版 CarPlay 的画面、音乐、操作、双向通话及挂断恢复音乐均正常。日志记录麦克风累计 377 包、386,048 B，通话结束后停止采集。
- **连接回归**：CarPlay → HiCar → CarPlay → HiCar 三次切换自动连通，分别约 8.88 / 13.64 / 8.97 s；始终保留 CarLife PID `9460`，USB 均连接。重复选择同一协议的五次请求中位数 31.02 ms，接收端 PID 与代次保持。测试结束切回 CarPlay，连接正常。
- **资源测量**：静态画面、无解码子进程，三次五秒采样的服务总 PSS 中位数由 21,887 KiB 降至 14,474 KiB；CPU 为 19.93% / 21.10%（单核口径），未显示改善。ARMv7 主程序增加 120 KiB。方法和前两阶段优化见[性能记录](performance.md)。
- **源码与应用构建**：[40819266235](https://github.com/Aki894/CarLink/actions/runs/40819266235) 成功，193 项主程序单测、OXP 字节级对照、真实 Rust 输出进程与合成 CarLife 闭环、麦克风代次/错误隔离、原生 PCM 持续播放和 Opus 往返均通过；旧 Python 路径仍作为对照运行。
- **完整固件**：[40820009281](https://github.com/Aki894/CarLink/actions/runs/40820009281) 的 native、rust-interop、firmware、镜像结构检查及上传全部成功。本地下载后核对 artifact digest、压缩镜像 SHA256SUMS 和检查报告；镜像内新 `carplay-output.sh`、运行脚本、应用和依赖校验通过。

镜像 `CarLink-refresh-39459953269-90ee05a4.img.xz` SHA256：`185caea67dfd8d1ff99802c95b6df17d0669824dcf853b3c548f20500acd31fb`。本轮验证范围为台架；Lexus 实车连接与实体触控板延续原有独立验收项。

## 2026-10-29：导航与音乐并行时的 HiCar VoIP 通话

修复源码 `68dfc2ea9d1b26b5a60c4f8291c0a33ffe6e3288` 已部署至 `/opt/carlink/releases/calls-68dfc2ea9d1b`，应用 SHA256 `5333f159badaeca8b484feb312160f9e6fbad6e2aa3cacf62c6cc22a77457a03`，七个运行文件及子进程路径核对通过。

根因是微信通话复用 `speaker1/mic1`，此前只根据 modem 服务识别通话，漏掉 DMSDP module 4 的 VoIP 焦点请求。车机麦克风独占焦点后暂停 MEDIA 播放器，而下行通话仍被当作普通音乐发送，形成积压。现按明确的焦点 usage 进入/退出通话角色，并结合 RTP stream/usage 区分同端口通话、音乐及导航；角色切换清理旧解码与待发 PCM，保留播放器 STOP，挂断后的迟到通话包被丢弃。另保留 CarLife 错误原因与 PID，便于复查 USB 失败。

- 用户在导航、音乐同时运行时复测通话，确认对方声音实时播放、挂断后音乐自动恢复、CarLife 未断开。
- 本轮日志记录 HiCar 和 CarLife 两侧 `call_active=true`；CarLife PID `2412` 在通话前后保持不变，USB 连接正常；新版 manager 启动后未出现 CarLife 重启。
- [应用构建 40722403781](https://github.com/Aki894/CarLink/actions/runs/40722403781) 成功，168 项 Rust 单测及真实 DMSDP 焦点入口、backend 到媒体控制、同端口混流和挂断恢复回归通过。
- [完整固件构建 40723628181](https://github.com/Aki894/CarLink/actions/runs/40723628181) 的 native、rust-interop、firmware、镜像结构校验及产物上传均成功。

已下载固件并核对 GitHub artifact digest、SHA256SUMS 与镜像检查报告；`CarLink-refresh-39459953269-68dfc2ea.img.xz` SHA256 为 `f4ecbe4b01b7fcd98594063e50762611e8eee120d4a324ed3c017719a1924da8`。

此前独立通话场景的结果保留在下节；本节补充的是导航与音乐并行的组合场景。对应 HiCar 代码已同步至本地 HelloCar，尚未推送此次修复。

## 2026-10-28：HiCar 双向音频、无感连接与协议切换

当前部署源码 `d4205f2ef4744acccd67261c9fb1b2566d6399e2`，运行目录 `/opt/carlink/releases/calls-d4205f2ef474`；应用 SHA256 `efb1cf20065de90e676c291f2ec5c81b9ead41909a7ff596763ff83ea8031d31`。部署后核对实际运行路径和七个关键文件哈希，保留回滚目录 `/var/backups/carlink/calls-20261028T100739Z`。

- **HiCar 自动连接**：主动查询已配对手机的 SDP，选择 FE35 的 RFCOMM 记录，排除同 UUID 的 ATT 记录；用户确认已能无感连接。
- **协议与菜单**：实机 HiCar → CarPlay → HiCar 自动连接分别约 8.5 / 9.7 / 8.5 秒，期间 CarLife PID `11095` 和 USB 会话保持不变。手动断开暂停自动连接，再点击连接可恢复；重复点击连接不重建接收端。该组切换运行于 `6b65118`，后续 `d4205f2` 仅修改麦克风上行。
- **麦克风根因与修复**：DMSDP `DataSessionID` 是控制会话编号，不能作为虚拟麦克风的 RTP SSRC。上行改为 stream `1100` 后，手机执行 AAC/ADTS 解码、PCM 入队和读取，原先的 `no packet` 与录音反复重启消失。语音探针采集 283 包 / 289792 字节，发送 418 个 RTP 包。
- **实际通话**：独立 Android CarLife 测试端与华为 HiCar 源机分离；用户确认双向通话、挂断后音乐恢复和画面连接均正常。本轮通话后 CarLife PID `16281` 保持不变。
- **异常退出恢复**：对 HiCar 接收进程注入 SIGTERM 后，约 3.29 秒返回菜单、19.69 秒自动恢复 HiCar；CarLife 进程与 USB 会话保持不变。
- **USB 与控制链路**：视频心跳等待 VIDEO_START 且传输就绪后发送，PAUSE 保留已完成握手，SUSPEND 停发、RESUME 恢复。CarPlay 控制 TCP 增加六秒连接失效检测，媒体/事件 socket 保留独立参数。CI 覆盖握手门槛、暂停恢复、健康静默连接、媒体背压、音频角色/服务代次隔离及三次返回菜单。

[应用构建 40667649054](https://github.com/Aki894/CarLink/actions/runs/40667649054) 和 [完整固件构建 40669393281](https://github.com/Aki894/CarLink/actions/runs/40669393281) 成功，native、rust-interop、firmware、镜像结构检查和产物上传完成。已下载并复核 GitHub artifact digest 与镜像 SHA256SUMS：`CarLink-refresh-39459953269-d4205f2e.img.xz` SHA256 `051c814c43853d24f56ed797ee58cd04ecfc2725a8c5a2e967a05c937079fd48`。

HelloCar 对应模块、测试及接口文档已本地同步，未提交或上传。Lexus 实车握手、实体触控板及长时间导航/通话路测仍安排在后续实车验收。

## 2026-10-27：双向通话与 USB 恢复

运行提交 `cac132a0644ecd017adc9f090015d7d6a930b082`，台架目录 `/opt/carlink/releases/calls-cac132a0644e`。应用由 [Actions 40379459149](https://github.com/Aki894/CarLink/actions/runs/40379459149) 编译，GitHub artifact digest、内部 SHA256SUMS、ARM ABI 与部署文件哈希均已核对；管理器、CarLife、CarPlay 和 HiCar 音频工具路径使用同一发布目录。

- 自动回归：132 个主程序单测、14 个镜像安装测试通过；包含真实 UnixStream 的双向音频、USB 控制命令分类、视频 RESET/PAUSE、连接恢复和旧会话隔离。
- ARM 编解码：固定 Opus 1.5.2 库在 QEMU 下通过 48 kHz、20 ms 单双声道编解码；AAC 麦克风编码、加密 HiCar RTP 与独立 SSRC 混音均有回归。
- 实机通话：用户确认 iPhone 经 CarLink 和华为 CarLife 测试端的双向声音正常，通话期间画面未断开。CarLife 进程与 USB 会话保持不变，麦克风持续采集约 32 kB/s。
- HiCar 已加入 mic1 / modemMic / modemSpeaker、服务拥有权及停止恢复；同一台华为兼任 HiCar 手机和测试 HU，独立 HU 通话与实车声学效果继续作为外部验收项目。

用户进一步确认“音乐 → 通话 → 挂断 → 音乐”正常且稳定。

完整固件 [Actions 40380718657](https://github.com/Aki894/CarLink/actions/runs/40380718657) 的原生认证、ARM 互通及 firmware job 全部成功，镜像结构、运行文件与源码校验通过，产物上传完成。下载后再次核对 GitHub artifact digest 和镜像 SHA256SUMS：镜像 `CarLink-refresh-39459953269-cac132a0.img.xz` 的 SHA256 为 `0bff0aa962dd7d7cfbd5f4debf9318182272ff7aafb0799351381517ac01a933`。

实现细节见[双向通话与麦克风](calls.md)；HelloCar 对应模块同步保留在本地，未提交和上传。

## 2026-10-26：Rust HiCar 完整后端与固定 PIN 台架交付

交付提交 `a47a9f0cdf307ff1f17518e8c90049821fee528d`，部署目录 `/opt/carlink/releases/r2-a47a9f0`；通过 SSH 更新运行程序并保留回滚目录，MFi/HiCar 身份文件校验保持一致。生产 HiCar 使用 `carlink hicar run`，由 Rust 管理无线、认证、DMSDP/RTSP、媒体和输入，Python 保留作参考与测试。

- [应用构建 40281149701](https://github.com/Aki894/CarLink/actions/runs/40281149701) 成功；[完整构建 40281831497](https://github.com/Aki894/CarLink/actions/runs/40281831497) 的 native、rust-interop、firmware 全部成功，镜像结构/依赖/校验和检查及上传完成。
- 镜像 `CarLink-refresh-39459953269-a47a9f0c.img.xz` SHA256 `8a7f3149996e263a87779b5f820992b7ed5acee73066a9d70102d60f022e4a74`；下载的固件 ZIP 为 436,560,040 字节，SHA256 `28ccd87aa9c9afa802a88279782984c7dc2094bbf3f30d7744d0906317c9dcb0`，GitHub artifact 摘要及内部镜像摘要均匹配。
- H3 已通过媒体、无线、原生私有 IPC、后端四组回归；真实 HiCar → H3 → 华为 CarLife 运行 361.0 秒，采样失败为 0，HiCar/CarLife 进程保持不变，应用网格点击有反馈。
- 强制停止真实 HiCar 源后，实际画面返回 CarLink 菜单，CarLife PID `29631` 和 USB 连接保持不变。切到 CarPlay 后 HiCar worker 为 0，用户确认画面、音乐和操作均正常。
- 最终版本已验证断电后开机自启及 CarPlay 自动重连；固定 PIN 配对页经真实菜单点击显示，取消后按原策略恢复 CarPlay，CarLife PID `968` 与 USB 连接保持不变。配对不生成或轮换原有 PIN。
- 补齐热点 IPv4 就绪等待、启动阶段取消清理、会话代际过滤、静态关键帧恢复，以及 AOA bulk 启用后 30 秒未完成初次 `VIDEO_START` 的 USB 握手恢复。成功连接后的静态/暂停会话不重启。

本轮按用户要求交付台架版本。Lexus 先前的 80%/0% 连接问题、实体触控板映射，以及独立 HU 的 HiCar 可听音频，留待实车验收。麦克风/通话尚未实现，HiCar 多音源分流与混音仍需补齐，详见[完成情况核对](status-audit.md)。公开文档不保存 PIN、身份或配对数据；本地回执在私有 `.build/hicar-research/rust-pin-final/`、`rust-pin-firmware-final/` 和 `rust-owner-release/`。详细架构与阶段记录见 [HiCar Rust 集成](hicar-rust.md)。


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


## 2026-10-27 连接与混合音频稳定性回归

用户确认独立车机端的 HiCar 音乐卡顿已经消失。继续处理的实测问题包括通话麦克风无声、混合导航/音乐/通话后的积压与静音、HiCar 闪回菜单及协议抢连。

代码修复覆盖本地 IPC 压力隔离、完整帧边界、角色/服务/源代次、已配对 HiCar 主动 Nearby 回连、三行菜单、默认协议与协议粘性。新增单测包含退役源迟到事件、视频暂停与真正会话退出的区别，以及相同 SSRC 的新服务序号归零。首轮 Actions Rust 检查 148 项通过；协议参考事件字段的严格差分已同步，后续完整流水线与台架结果继续记录于本节。
