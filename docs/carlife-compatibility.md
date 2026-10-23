# CarLife 与 Lexus 实车兼容性

截至 2026-10-23，Lexus 首轮实车连接仍需复验：进度 80% 失败、重试 0%，偶发车机应用退出。后续持久日志修复与接受通知顺序修复已经完成，并通过原生 ARM / 华为台架；构建版本见[验证记录](validation.md)。

车型为 2020 Lexus ES200 卓越版，仅支持有线 CarLife。实车只有车机 USB 供电，中途手动断电一次，通电到失败约 30 秒。

以下区分当时取证与后续代码分析。

## 取证结果

- 已通过有线 SSH 读取日志并归档到本地忽略目录，归档包含 CarLink 全部现存 journal、启动索引、最近四次启动的系统日志、当前内核日志和服务状态。
- 该轮开发板应用为 `95d7147`，没有部署 30 帧实验版。
- 最近四次历史启动的持久日志末尾分别约在开机 18.68、18.97、19.03、19.51 秒；其中没有 CarLink 服务消息。该次启动中 CarLink 于约 23.69 秒启动。
- 查询 `systemd-journald` 记录及实际 Armbian ramlog 脚本，确认 ramlog 启动流程先执行 `journalctl --flush`，随后执行 `journalctl --relinquish-var`。后者把后续记录切回 `/run/log/journal`；断电会丢失这些内存日志。仅配置 `Storage=persistent` 和 `/var/log/journal` 链接没有覆盖这一后续操作。
- 这次实车失败时的完整 USB/AOA 与 CarLife 握手记录未保留下来。现有记录用于确认日志保存缺陷，连接失败根因仍待复现取证。
- 首轮取证时华为 USB 主机能够识别开发板初始设备 `18d1:4ee7`，尚未开始该次启动的 AOA 切换及 CarLife 握手。

## 已应用的日志修复

1. 先归档原始日志，再执行 `journalctl --flush`、`journalctl --sync`。确认活动 journal 已位于 SD 卡上的 `/var/log.hdd/journal`，内存活动 journal 已退出。
2. 在 `carlink.service` 增加启动前的 flush/sync 步骤，运行在本地启动依赖就绪后，使 ramlog 的 relinquish 不再影响之后的投影日志。
3. 固件启动配置中的日志同步间隔统一为 5 秒，与开发板现有配置一致。
4. 开发板只添加日志启动配置并同步到盘；投影二进制、配对和无线参数保持本轮取证时的版本。

本地启动配置测试 9 项通过。随后执行受控软件重启：`carlink.service` 自动启动，两个 `ExecStartPre` 均以状态 0 完成，`NRestarts=0`；直接从 `/var/log.hdd/journal` 成功读到本轮投影日志，未再使用内存活动 journal。后续实车复现继续验证突然断电时的记录保留。

## 用户补充及台架对照

回到台架后 Type-C 接华为 OTG。历史持久日志的 19 秒末尾源于日志切换。

华为解锁并打开车机端后，捕获到一次 `UsbManager.openDevice` 异常：AOA 切换后仍访问先前 USB 设备路径，报 `device ... does not exist or is restricted`，随后 CarLife SDK `openAccessory` 出现 `NullPointerException`。重启华为车机端后，协议版本、设备信息、认证、功能和视频协商完成，画面恢复。此为台架接收端异常；实车同一异常待日志核对。

保持华为车机端前台，再次受控重启开发板，使用本轮开机单调时钟记录：

| 事件 | 开机后时间 |
| --- | ---: |
| 初始 USB 身份绑定 | 24.347 秒 |
| iPhone 无线会话活跃 | 31.456 秒 |
| 收到 AOA 身份请求 | 35.638 秒 |
| accessory bulk session enabled | 36.406 秒 |
| 收到 CarLife 协议版本 | 42.052 秒 |
| 完成视频协商、等待视频源 | 42.074 秒 |

华为随后恢复画面；本轮协议协商阶段约 22 ms。日志及手机接收端输出已归档到本地忽略目录。下一步优先验证冷启动等待和 USB 重枚举时序是否超过真实车机的连接窗口，连接失败阶段需下一次实车完整日志确认。


## 实际检查的材料

- 当前 Linux `app/src/gadget.rs`、`app/src/carlife.rs`、`app/src/runtime.rs`。
- 本地参考 CarProjection 的 `MsgProcess.java`，特别是 `STATISTIC_INFO` 分支。
- 从华为测试机提取的 `com.baidu.carlifevehicle` 4.0.0.43 APK，5,446,708 字节，SHA256 `3cbdd967f4b679eee73db91d6374bf54a81e40cddd1a7d74317af879054b8976`。
- 使用 [JADX 1.5.6](https://github.com/skylot/jadx/releases/tag/v1.5.6) 反编译；下载包摘要核对 GitHub release 元数据。反编译有 6 处错误，不能照抄其中恢复失败的方法；本次关注明确可追踪的调用、常量和分支。
- [HeadSwap Handshake.kt](https://github.com/YegorkaPomidorka/headswap/blob/main/android/app/src/main/java/com/yegorkapomidorka/headswap/Handshake.kt) 的 `onStatisticInfo`。
- [公开保存的旧版车机源码](https://github.com/fgrcwp/apollo-DuerOS/tree/master/CarLife-Android-Vehicle)，以及同仓库 `CarLife-Android-Vehicle-V2.0` 源码。

APK、反编译结果和参考源码只保存在忽略的 `.build/apk-analysis/`，没有提交。尚未取得 Lexus 原厂 CarLife 可执行文件。

## 截图补充：原厂系统范围

用户截图显示 DCU 型号 `19LDL1CN-EBPC`、零件号 `86140-33L00`、音频软件版本 `1800`。其中 1800 是整机音频软件字段；CarLife 协议版本通过握手读取。

[Lexus 技术通告](https://static.nhtsa.gov/odi/tsbs/2020/MC-10179725-9999.pdf) 将相似 `17LDL2NA` DCU 标识列于 Panasonic 系统，[腾讯科恩实验室对这一代 Lexus AVN 的原始研究](https://keenlab.tencent.com/en/2020/03/30/Tencent-Keen-Security-Lab-Experimental-Security-Assessment-on-Lexus-Cars/) 记录了 DCU 内的 Linux、U-Boot 和 ext4 分区。依据命名家族、车辆年份和资料，当前倾向松下 MM19 代原生 Linux 方案；精确中国区子型号尚未找到对应公开包。后续原厂分析优先寻找匹配的 DCU 更新包、Linux 可执行程序和共享库，Android APK 用作协议参考。

## 1. 已修复：将两个方向的认证状态错误绑定

原 Linux 实现只有同时满足以下条件才发送 `MD_PEER_ACCEPT`（0x1004b）：已收到 HU_INFO、STATISTIC_INFO，且已经回答车机挑战（`auth_sent`）。

CarProjection 的统计信息分支和 HeadSwap 的 `onStatisticInfo` 都将 0x1004b 作为手机端接受车机的状态通知。车机向手机发出 0x18048 挑战，手机计算 0x10049 回应，再收到 0x1804a 验证结果，是另一个方向的处理。旧车机若等待 0x1004b 才继续，且没有发出挑战，原条件会一直不成立。

修复：收到 STATISTIC_INFO 后发送一次接受车机通知，不再等待 HU_INFO 或挑战认证。重复统计包不重复通知。挑战计算和车机返回失败时关闭会话的检查保持有效。回归覆盖无挑战的 1.0 流程、先统计后挑战、重复统计、明确认证拒绝以及播放中的统计重发。

这是已确认的实现兼容性缺口；是否触发在这台 Lexus 上，由实车首轮握手记录确认。

## 2. 旧版 SDK 中确实有明确的 80% 位置

旧源码 `connect/AOAHostSetup.java::initUsbDevice` 的流程如下：

| 进度 | 源码动作 |
| --- | --- |
| 10 | 扫描到设备、开始连接 |
| 20 | 请求 USB 权限 |
| 30 | 打开设备并 claim interface 0 |
| 40 | 尝试切换 accessory 模式 |
| 50 | 确认为 accessory 模式 |
| 60 | 获取 IN/OUT endpoint |
| 80 | 启动本地 socket 读取线程，随后等待 1 秒、连接各本地通道、启动 AOA USB 读取线程 |
| 100 | `ConnectManager.addConnectSocket` 收齐内部通道后由 `ConnectClient.setIsConnected(true)` 发出 |

这条旧实现中的 80% 属于 USB accessory 和内部通道建立阶段，不能直接标为“手机挑战认证失败”。其 `MainFragment` USB 超时为 20 秒，从 CONNECTING 开始计时；超时调用 `stopConnectThread()`。重新连接按钮同样先停止再启动连接线程，UI 会清零。

华为 4.0.0.43 的新版 `ConnectionEstablishHandler` 则是 0 → 30 → 70 → 100，没有 80；其挑战回应超时为 30 秒。两个实现说明，必须先匹配车机版本，再解释进度。

公开旧源码为匹配实现提供线索，当前尚未确定 Lexus 使用哪一分支。

## 3. 华为端旧 USB 对象异常已对应到代码

已有日志显示 `UsbManager.openDevice` 报设备路径不存在，随后 SDK `openAccessory` 捕获 NullPointerException。反编译的 `UsbAccessoryScanner` 权限广播直接使用 Intent 中的 `UsbDevice` 调用打开流程，打开返回值后紧接 Kotlin 非空检查，没有在这里重新查询当前 deviceList。

AOA 切换会产生 detach/attach 和新设备路径，因此旧权限回调可能撞到已消失的设备。当前这些异常被 SDK 捕获并记录；Lexus 的应用退出原因需要其对应日志或可执行文件进一步定位。

## 4. Linux 失败恢复的现状

- FunctionFS UNBIND/DISABLE 或 USB 读写失败会结束进程，由服务重新建立 gadget。
- 如果车机只关闭应用层连接，USB 仍 configured 且没有读写错误，当前没有独立的握手超时恢复；重复相同版本包的回复也受到去重约束。
- 先保留这一行为以采集下一次真实失败，避免未经判定的定时重枚举打断正常连接。下一轮若确认车机静默停留在 accessory 模式，增加按握手阶段判断的恢复，而不是周期性重启整个系统。

## 下一次实车记录与判断

1. 使用已经修复的持久日志，记录一次从点火到 80% 失败，再点一次重新连接的完整周期，保留板子通电。
2. 检查是否收到 AOA 51/52/53、是否绑定 18d1:2d00、是否出现 bulk enabled。
3. bulk enabled 后没有协议首包：优先检查 USB 通道开启、端点传输和车机内部桥接；上述接受通知修复尚未执行到。
4. 收到协议版本后卡住：检查具体版本、统计信息、0x1004b、挑战与回应、视频 INIT/START 的最后一条记录。
5. 首次失败后 0%：比较 USB 是否仍 configured、是否再次发送协议版本、是否有设备 detach/attach、应用退出时是否出现读写错误。
6. 当前没有取得原厂镜像或原厂 APK；版本信息拿到后再匹配对应软件，避免混用新版手机测试端的进度定义。
