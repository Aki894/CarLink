# Linux 无线 HiCar 实施计划

## 目标与当前结果

当前状态（2026-10-30）：HiCar 已由 Rust manager 集成、自启并进入完整 refreshed 固件；独立 HU 双向音频及组合通话已验收。首次刷机身份初始化和其他差额见 [status-audit.md](status-audit.md)，最新结果见 [validation.md](validation.md)。以下为早期探索计划与按日期保留的历史记录，其中“尚未集成”等描述对应当时版本。

目标平台为悟空派 H3（ARMv7、512 MiB）、AX900/AIC8800、Linux/BlueZ，手机范围为鸿蒙 4.2。HiCar 接收后复用现有 CarLife 输出，保留有线 SSH。探索程序与现有 CarPlay 接收端隔离，暂不随生产固件安装或自启。

2026-10-23 已实测走通：FDEE 发现 → FE35 控制通道 → FE36 业务 19 → 身份 JSON → 固定六位 PIN → HiChain 认证成功。手机为 HiCar 14.2.0.400；H3 上两次独立进程测试均得到结果 `[2,3,0]`，即处理中、密钥协商完成、认证成功。两次使用同一持久化身份与 PIN。

认证后的 Nearby ResourceManager、5 GHz 热点加入、TCP 控制通道和业务 20 已实测通过。DMSDP 加密控制、HiSight H.264、输入和 STS 重连、AAC 收流与 H3 解码、周期时钟已通过。实验 CarLife 桥接已显示真实桌面并支持点击/滑动；下一阶段按 [Rust 迁移计划](hicar-rust.md)工程化，同时保留独立 HU 音频和 Lexus 实车验收。

## 阶段与验收

| 阶段 | 状态与实现内容 | 验收方式 |
| --- | --- | --- |
| 1. 发现与传输 | 已通过；FDEE、FE35/FE36、限定手机 SSP 代理、退出恢复设置 | 手机发现并建立双通道；SIGTERM 释放通道；原 CarLink 保持 active |
| 2. Nearby 封装 | 已通过所观察 BR 路径；变长整数/TLV/CRC、拆包粘包、有界捕获 | 真实首包及业务 19 请求与参考实现对应；异常长度/校验测试 |
| 3. 身份认证 | 固定 PIN、持久化身份/HUKS、公开 Linux HiChain 后端、真实手机免 PIN STS 重连已通过 | 正确/错误 PIN 本机对照；真实手机 STS 17/18 后恢复视频与输入 |
| 4. 网络与接收 | 热点/TCP、AES 控制、H.264、点按/方向键、AAC 收流与解码、周期时钟已实测 | 800×480 视频与真实音频同收；H3 输出 44.1 kHz PCM，真实手机免 PIN 重连和输入有效 |
| 5. CarLife 桥接 | 实验版已接入 H.264、AAC/PCM 和 UIBC；同手机桌面、点击、滑动已反馈 | 独立 HU 验证实际出声和音画同步，再测 Lexus 的声音、触控板、重连与延迟 |
| 6. 启动与交付 | 待桥接稳定；服务互斥、日志、恢复、固件集成 | 完整 firmware job、镜像校验和上传；冷启动、长时播放与断电恢复 |

## 已确定的协议与适配

- HCI 曾因 `connectable` 关闭以 `Unacceptable BD_ADDR` 拒绝手机；临时开放后，SSP 又因缺少代理被拒绝。探针现在只为指定手机、FE35/FE36 注册 NoInputNoOutput 代理，退出恢复原 connectable/bondable 设置。
- 手机请求 CCMP v5、加密偏好 2；参考实现兼容的 v2/明文控制封装已实测。CRC 用于帧完整性；身份认证由后续 HiChain 完成。
- FE35 默认 RFCOMM 23；FE36 默认 24。BR 流创建请求为 channel 2、业务 19、协议 0。
- 首包是 `oper_type=1`、`version=14.2.0.400`，`data` 为嵌套 JSON 字符串，允许省略 `discover_type`。身份回复声明 `has_pin=true`，无线业务流不直接传送 PIN。
- 认证请求为 `oper_type=3`，内层 `message=1/2/3`；手机声明版本范围 `1.0.0..2.0.31`。后端返回相应的 `32769/32770/32771`，完成 PAKE 和身份交换。
- 有未完成的网络连接时，手机可能直接重试 P2P，跳过新一轮认证。本轮结束旧 HiCar 连接进程后，恢复了 BR 身份/认证流程；没有清除手机应用数据。

## 认证后端与固定 PIN

本地参考 APK 的认证库依赖 Bionic、JNI 与 Android 日志。采用公开 OpenHarmony 源码编译 Linux C 后端，通过 Python ctypes 接入，避免在 glibc 上模拟整个 Android 运行库。公开源提交固定于 [sources.json](../experiments/hicar/native/sources.json)；DeviceAuth 基线为 [OpenHarmony-v1.1.4-LTS hichain.h](https://github.com/openharmony/security_deviceauth/blob/552edb521d0bd7c17f2dab2791e7cdc3de284405/interfaces/innerkits/deviceauth_lite/hichain.h)。探针根据实际手机 STS 请求使用 `com.huawei.hicar` / `CarDevice` 命名空间；通用测试仍保留旧 JNI 默认值。

- 每设备首次生成六位 ASCII PIN，保留前导零，后续启动保留；操作员可通过标准输入修改。认证失败不会轮换 PIN。
- 身份/PIN 为 `/var/lib/carlink/hicar/identity.json`，0600；HUKS 目录 `/var/lib/carlink/hicar/keys` 为 0700，三个存储文件均为 0600。原始认证包、密钥及厂商资料不提交。
- ctypes 启动时核对结构体大小；回调先排队，退出原生调用后再发送；认证失败关闭连接，防止错误包互相循环。
- OpenSSL 3 的公开 EVP 接口适配旧 HUKS 的 Ed25519 函数。RFC 8032 空消息测试向量和签名篡改拒绝在 Actions 主机及 H3 均通过。
- 真实双端 PAKE 对照在 Actions 主机和 H3 均通过：正确 PIN 六个消息完成认证、会话密钥一致；错误 PIN 在第三个消息失败。

运行命令、配置和测试入口见 [实验说明](../experiments/hicar/README.md)。目前 Python 共 48 项测试在 Windows 与 H3 通过，包括 NIST SP 800-38A 的 AES-128-CTR 标准向量、分片协商、加密保活和异常输入拒绝。

## 构建与证据

手动工作流 `.github/workflows/hicar-auth.yml` 先构建和测试主机库，再交叉编译 ARMv7，打包依赖、许可证、ELF 信息和 SHA-256 清单；随后调用完整生产镜像刷新任务。

[认证基线 Actions 39930292376](https://github.com/Aki894/CarLink/actions/runs/39930292376) 与 [网络/端口协商 Actions 39935892052](https://github.com/Aki894/CarLink/actions/runs/39935892052) 的 native 和完整 firmware job 均成功，镜像验证与产物上传完成。认证独立产物下载后已核对 artifact SHA-256、每个成员校验和，H3 依赖加载及原生测试通过。本轮生产镜像沿用已成功的应用/驱动基线；HiCar 实验通过 SSH 单独部署，无需刷卡。

私有证据保存在 `.build/hicar-research/`：主机/ARM 构建日志、原生库清单、手机日志与原始业务流。板上 `/var/tmp/hicar-agent-16.log`、`hicar-agent-17.log` 为两次真实认证事件记录，包含状态与长度，不输出 PIN 或密钥。已有 `smoke-result.json` 保留探针监听与清理证据。

## 无线网络与 DMSDP 实测（2026-10-23）

独立实验入口增加 `--network-interface`。程序先确认 192.168.77.1 仍走有线管理网口，保存当前无线连接和 CarLink 服务状态，再临时建立 5 GHz/channel 149 热点。退出、SIGTERM 或建网失败时删除本次临时连接、恢复原无线连接及 CarLink 服务，不修改开机配置。

已走通的链路：

1. HiChain 成功后才接受 kind 4 的资源协商。手机申请 channel 3；开发板返回已就绪的 5745 MHz 热点端能力，手机通过 TLV 7 确认自己作为客户端。
2. 开发板以 kind 33 / TLV 1 索取手机 Wi-Fi 地址，收到 TLV 2 后，通过 TLV 3 发送本次热点参数。参数采用参考实现的软件 AP 路径，包含 version 3、控制端口和 hicarApIp；临时密码不进入日志。
3. 手机自动加入热点，使用 192.168.49.2 连接开发板 192.168.49.1:4005，完成 TCP 上的 CCMP。
4. 手机请求 channel 3、业务 20、tag 为 DMSDP、协议 1。开发板只在 4006 已监听后回复实际端点；手机随后连接并发送 21 字节首包。
5. 首包为 DMSDP module 1、16 字节全零载荷，CRC16-ARC 正确。其外层为 `7f | module varint | size varint | payload | CRC`，没有 CCMP 的 flags TLV。
6. 对照原生 `VirtualDeviceSink::OnReceiveData` / `SendNegData`，接收端先创建新的控制监听，再回复 16 字节结果：状态 0、两字节大端端口、13 字节保留位。手机接受 4007 并建立连接。

第 19/20 轮完成热点及业务 20 测试；第 21 轮在独立 USB 供电、开发板重启后完成控制端口协商。私有事件日志为 `/var/tmp/hicar-agent-19.log` 至 `hicar-agent-21.log`；原始帧和关机前快照在本地 `.build/hicar-research/`。

AX900 驱动声明支持 P2P-client/GO/device。直接调用现有 supplicant 创建 GO 时命令返回 FAIL，因此当前采用已实测的软件 AP 兼容路径；不依赖创建真正的 Wi-Fi Direct 组。

## 下一阶段实现顺序

### 计划复盘与调整（2026-10-23）

原六阶段保持，第三阶段的固定 PIN 与真实手机免 PIN STS 重连已通过；第四阶段已实现音视频接收、输入和周期时钟。第五阶段复用 CarLife 输出接口，验证音画同步及队列调度；Lexus 80% / 0% 兼容性及 Remote Touch 随后实车复验。第六阶段的完整构建已具备，HiCar 尚未加入生产自启。

| 近期里程碑 | 实施范围 | 通过条件 |
| --- | --- | --- |
| A. 完整控制会话 | 本端 101 信息、必要时间同步、设备/服务状态、已有连接重试分支 | 连续至少 5 分钟控制会话；主动退出、失败重试后旧 socket/热点资源清理正确 |
| B. 首帧与连续画面 | 最小显示服务、媒体端点、编码参数和接收组帧 | 获取真实可解码首帧；连续切换页面，测量接收帧率、队列深度和恢复情况 |
| C. 音频与反向操作 | 音频格式/时钟、基本点击与按键；再映射 Remote Touch | 声画同时运行、基本操作闭环；输入角色与媒体退出恢复正确 |
| D. CarLife 桥接 | 复用现有 H.264/PCM 输出，按实际 HiCar 格式适配 | 第二台 Android HU 的画面、音乐和操作；具备条件后 Lexus 复验 |
| E. 无屏产品化 | 固定 PIN 首配、可信重连、CarPlay/HiCar 互斥、服务恢复和自启 | 三次冷启动、至少一小时使用、断线/断电恢复；随后完整固件交付 |

时间同步先满足建立媒体服务所需部分，真实时钟效果随音视频一起验证；按最小显示能力推进首帧，不一次声明所有语音/通话服务。H.264 能否直接转发以实际码流为准，优先避免 H3 软件转码。

软件 AP 作为当前鸿蒙 4.2 已验证的主路径；暂不投入真正 Wi-Fi Direct GO。Python 负责协议与状态迭代，原生 C/系统 OpenSSL 负责认证与密码运算，接入媒体后以测量结果决定是否迁移高频路径。

2026-10-24 更新：先尝试同台华为通过无线提供 HiCar、通过 OTG 运行 CarLife HU，验证完整转发回路；同时检查音频焦点及前后台共存。独立 HU 和实车随后验收，接线与步骤见[连接管理设计](connection-management.md)。

本轮代码提交 `b9fa5f9` 的 [Actions 39942615426](https://github.com/Aki894/CarLink/actions/runs/39942615426) 已完整成功：48 项 Python 测试、原生正/误 PIN PAKE、ARMv7 交叉编译、完整 firmware job、`mode=current` 镜像结构/校验及上传均通过。原生库 ZIP 的下载 SHA-256 与 10 个清单成员逐项核对通过。镜像产物为 `carlink-h3-refreshed-image`，实验仍由 SSH 单独部署。

1. 处理时间同步请求，解析模块 3 的设备/服务消息；目前手机等待后续业务约 30 秒后结束连接。
2. 完成设备/服务能力交换和实际媒体端口协商，解析 H.264、音频与输入事件，再复用 CarLife 输出；尽量直接转发，减少 H3 上的软件解码/重编码。
3. 将稳定协议迁入常驻接收端，完成真实 HU、可信重连及长时间运行测试后再加入生产固件。

### 2026-10-23 控制会话实测

- 核对手机实际 `/system/app/HwDMSDP/HwDMSDP.apk`（`com.huawei.dmsdp`，4.0.0.334）及 ARM64 库，确认三字节外层封装与参考接收端一致。
- `CryptoAbility=222` 是原生实现检查的协议值；发送 0 时手机不应答。接收端发出 OPTIONS 后，手机以 SET_PARAMETER 返回 `CryptoSwitch=222`，本端按手机 CSeq 应答。
- AES-128-CTR 使用本轮认证产生的会话密钥前 16 字节；每帧为 16 字节随机 IV 加密文，外层标记 `0xAB`。调用系统 OpenSSL EVP，密钥只留在内存，关闭会话时释放本端可变密钥缓冲区。
- `hicar-agent-31.log` 成功解密 GET_PARAMETER；`hicar-agent-32.log` 连续应答约 30 次加密保活，同时收到 TimeServerPort/TimeUsHigh/TimeUsLow，以及模块 3、类型 `0xffffffff`、394 字节业务消息。
- `hicar-agent-33.log` 的私有解密捕获确认：394 字节业务以本协议变长整数 `101` 开头，其后是设备信息 JSON，字段包括 deviceType、网络端点、deviceId、properties。对应参考 `DMSDPDeviceManager.onDataReceived` 的 101 分支及 `PackageTool.packageDeviceData`；下一轮发送本端设备信息，继续服务协商。
- DMSDP 的 CTR 记录没有独立认证标签；软件 AP 的 WPA2 和 HiChain 身份认证继续保留，解密内容受长度、格式和状态约束。
- 显式启用私有捕获时，解密控制记录写入同目录 `.plain` 文件，0600，沿用接收大小上限。密钥不写入捕获文件；参考 APK、手机程序及所有捕获均留在忽略目录。

单独调试无线 HiCar 时使用独立 USB 电源，手机可独立充电。CarLife 桥接阶段可按[单手机台架计划](connection-management.md#单台华为同时作为-hicar-信源和-carlife-测试端)接回手机 OTG；统一切换单路供电后再测试，AX900 和有线管理通道保持连接。

## 参考定位

| 本地参考源码 | 用途 |
| --- | --- |
| Nearby `PublishDeviceInfo`、BLE 广播实现 | FDEE service data：版本、型号/子型号、序号 |
| Nearby `h/e/e.java`、`h/e/h.java` | 自定义变长整数、TLV、CRC 与 CCMP |
| Nearby `transfertype/message/MessageManager.java` | 业务流创建与 FE36 |
| Nearby `transfertype/resource/ResourceManager.java`、`resource/a.java` | kind 4、数据通道能力和 P2P 协商 |
| Nearby `BleDevice`、`P2pChannelStateMachine` | `046C` 返回 P2P 地址或 busy、网络状态机 |
| AuthAgent `NearbyConnectControl`、`HiChainAuthControl` | 身份 JSON、oper_type 3、原生回调及 PIN |

参考资料位于 `hellocar/reference`，反编译和临时依赖位于本地 `.build/hicar-research/`。探针独立编写，公开仓库保留接口观察与源版本引用。

官方接口资料：[HiCar 平台](https://developer.huawei.com/consumer/cn/HiCar)、[连接方式](https://developer.huawei.com/consumer/cn/doc/HiCar-Guides/obtain-connection-mode-0000001052220416)；BlueZ [ProfileManager](https://github.com/bluez/bluez/blob/master/doc/org.bluez.ProfileManager.rst) 与 [LEAdvertisement](https://github.com/bluez/bluez/blob/master/doc/org.bluez.LEAdvertisement.rst)。


## 显示与 HiSight 接收实测（2026-10-23）

本轮推进 A/B/C：设备与显示服务交换、真实视频接收、基础输入发送。A 的控制会话和退出恢复已覆盖，免 PIN 的 STS 重连仍需兼容手机端签名；B 已收到可解码的真实画面，C 的音频尚未实现。

- 模块 3 的 101 设备交换、104 服务请求（831）、102 显示服务（display1/type 8）已实测通过。模块 5 的 101 消息给出 HiSight RTSP 端口。
- HiSight 使用四字节大端长度、独立 RTSP 协商和 AES-128-CTR。手机端存在省略或误计 Content-Length 的记录，解析以已验证的外层记录长度为界；DMSDP 保持原严格解析。
- SETUP 返回视频和反向输入端口。必须在 PLAY 前同时建立两条 TCP 连接；仅连接视频端口时实测等待约 60 秒后退出、没有视频数据。
- 第 58 轮已在 H3 上持续接收超过 326 秒、1980 帧。协议请求 800×480 / 30 fps / 2 Mbps（动态范围 1–3 Mbps）；实际发送帧率随画面活动变化。
- 已用 FFmpeg 解码真实视频，依次确认安全驾驶声明、首次配置和 HiCar 桌面。用户同意后通过 ADB 完成首次声明确认；原始媒体只保存在私有目录。
- hisight_media.py 实现 TCP/RTP/PES 拼接和 ES 解密，输出有界原始流及 H.264。hisight_input.py 实现加密 UIBC 点按、方向键、确认、返回和播放暂停；实验接口仅监听私有捕获目录下权限 0600 的 Unix socket。
- 新增独立双进程认证测试：双方分离 HUKS 存储，首次配对后结束进程，再用持久存储完成 STS 认证。该测试已在 H3 通过；实际鸿蒙手机 STS 会返回签名校验错误，需要进一步比对差异。

接下来的优先顺序：验证基础输入、解决真实手机 STS 重连、接入音频/周期时钟同步，再用第二台 CarLife HU 或实车验证桥接。实验接收器仍独立于正式 CarPlay/CarLife 服务。


## 本轮构建交付（2026-10-24）

代码 4eeb9ec 对应 [Actions 40056842897](https://github.com/Aki894/CarLink/actions/runs/40056842897)：native 与完整 firmware job 均成功，镜像结构、应用/运行库内容、内核配置、模块、固件和校验清单通过，产物上传完成。

- 原生主机完成 54 项协议测试、正确/错误 PIN 对照及分离存储、进程重启后的 STS 认证。ARMv7 库构建成功，artifact 12545408918；下载后 ZIP SHA-256 和包内 10 个文件清单核对通过。
- 完整镜像 artifact 12546265473，ZIP 420,401,070 字节；上传摘要 7886f14c55512228d7be4e33b869213014eea92ea80b0557a40659d15459a80d。镜像沿用正式 CarPlay/CarLife 应用与驱动，独立 HiCar 实验暂不安装为生产自启服务。
- 本地保留的视频快照完整解码出 1,856 帧、800×480 H.264，包含声明页面与真实 HiCar 桌面；稍后的板端日志已记录 1,980 帧、326 秒以上连续接收。
- 用户重启盒子后，电脑直连网口 Up/100 Mbps、192.168.77.1 地址有效，盒子 192.168.77.2 暂无 ARP/SSH 响应。手机无线 ADB 已重新连通。新增 UIBC 输入回传实机测试与真实手机可信重连继续排查，待有线管理恢复后进行。

## 基础输入回传实测（2026-10-24）

盒子再次通电后 SSH 已恢复，正式 carlink 服务为 active，手机无线 ADB 可用。第 66 轮以同一实验身份重新 PIN 配对并进入 HiSight，验证了加密 UIBC 回传：

1. 点按退出智慧聆听配置、关闭更新提示并打开应用列表。
2. 两次向右按键使焦点移动到设置，确认键打开设置页。
3. 返回键回到应用列表。每一步均从接收的 H.264 解码图像确认，未用 ADB 代发这些车机操作。

关键修正是对手机的 GET_PARAMETER 明确返回 his_uibc_capability，声明 GENERIC 类别和点按、按键的按下/抬起能力。此前只连反向控制 socket 能启动视频，但事件不生效。54 项协议测试已在本地和 H3 的独立源码快照上全部通过。

真实手机的免 PIN STS 仍返回签名校验失败；删除手机上这一条实验车辆记录后，固定 PIN 首配可恢复。设备/控制交换、连续视频和基础输入已通过，音频与周期时钟同步待接入。下一步优先修复可信重连，随后做音画同传，再接 CarLife 输出。

本轮结束统计：连续接收 540.858 秒、2,554 帧、8,529,854 字节媒体包。SIGTERM 后本地输入 socket 清理，carlink 恢复 active，CarPlay 热点和有线管理连接恢复。

## 可信重连身份修正（2026-10-24）

手机实际实现 AuthenUtils / NearbyConnectControl 使用大写 SHA-256(model ID + 去冒号的大写 BR MAC) 作为车端认证身份。此前探针发现摘要已采用该值，但业务身份和原生 HiChain 使用随机持久化 ID。PAKE 交换公钥时可接受声明的 ID，后续 STS 按发现身份查找公钥并拼接验签内容，导致重连失败。

已用同次请求、响应与手机验签日志确认：盒子签名对其自身输入验证成功，手机使用的摘要则精确匹配上述派生身份。现将发现摘要、业务身份、原生签名和 DMSDP 设备身份统一；持久化随机 ID 继续作为 HUKS 本地硬件种子，原文件、固定 PIN 和已有密钥存储保留。更正后需为该实验车辆重新首配一次。

新增固定协议向量、大小写规范化、输入检查、持久化文件不变及握手/后端同身份回归，Python 测试共 57 项，Windows、H3 和 Actions 均通过；原生隔离进程测试改用含大写字母的 64 字节身份。

第 74 轮完成更正后的 PIN 首配；第 76 轮在保留身份、PIN 和密钥存储并重启接收进程、手机测试应用后，真实手机走 STS 消息 17/18，结果 [2,0]、16 字节业务密钥就绪，无需输入 PIN。随后自动加入热点、建立 DMSDP/HiSight 并恢复可解码画面，反向按键回传也生效。原身份文件与 PIN 的前后字节比较完全一致。

后续复测仍遇到手机发现阶段有时不弹连接提示，未进入认证交换；将发现/断开恢复作为独立验收项，下一步结合正常 TEARDOWN 和服务状态收尾处理。音频接收与周期时钟同步继续按计划接入。

## 投屏退出与重复重连（2026-10-24）

提交 38eac3b 在退出时先保留认证、热点和底层 TCP，发送加密 HiSight TEARDOWN，并处理手机的结束请求，随后关闭媒体/输入、Nearby 和蓝牙连接。收尾读取有时间与字节上限，媒体 socket 关闭也有超时；对端主动结束时先应答，再释放资源。

- Windows 与 H3 的独立源码目录各通过 62 项测试，覆盖正常应答、手机反向结束、无响应超时以及控制层先于底层传输关闭。
- 同一持久化身份与密钥下，连续完成 80→81→82→83 三次重连，期间未重启手机应用、切换蓝牙或重新输入 PIN；每次 STS 17/18 成功后均恢复 HiSight 视频，82/83 的方向键输入返回 OK。
- 82/83 的退出日志分别记录发送 TEARDOWN 后约 15/30 ms 收到手机反向结束请求。手机采用自己的 TEARDOWN 结束会话，没有返回原请求的 200，应按实际双向结束流程处理。
- 83 结束后 carlink 恢复 active，私有输入 socket 已删除，有线 SSH 持续可用。原始手机日志和板端记录保留在忽略的 `.build/hicar-research/reconnect*`，不提交仓库。

当前优先级转向音频服务声明、接收格式及周期时钟同步，随后做音画同传与 CarLife 桥接。无线发现弹窗仍需用户点击确认；产品化的冷启动、意外断电与长时间恢复测试安排在常驻接收端阶段。

38eac3b 的 [完整构建 40079990109](https://github.com/Aki894/CarLink/actions/runs/40079990109) 已成功：62 项协议测试、原生认证回归、ARMv7 编译、完整 firmware job、镜像结构与内容校验、产物上传全部通过。认证 artifact 12554379334 的下载 ZIP 与包内 10 个文件校验通过；镜像 artifact 12554811970，ZIP 420,559,926 字节，上传 SHA-256 为 895a80da7a0b800f2ac836eef934d6afb00547366fe3f52708af50c701d1993b。板上通过 SSH 更新实验 Python 文件，完整镜像未重新刷写。

## AAC 音频、周期时钟与双向保活（2026-10-24）

实现提交 3ffd02c，仍为独立手动实验接收器，不随正式镜像自启。`--audio` 在已有认证、网络及 `--dmsdp-options` 上增加扬声器服务。手机仍是同一台鸿蒙 4.2 / HiCar 14.2.0.400。

- 服务层声明 `speaker1/serviceType=4`，实际传输控制使用 `ServiceType=3`。Simple 模式收到 OPTIONS 触发后直接发送含加密协商的 SETUP，省略额外 OPTIONS/PLAY。手机音频 RTP 的 RTCP 关闭，Transport 必须为 `client_port=实际端口--1`；两个正数端口会被原生发送器以 `-2099999998` 拒绝。
- 线上媒体是 AES-CTR 加密的 UDP 三字节外层、RTP payload type 96、完整 AAC/ADTS 帧。AudioParameter 的 48 kHz / 16 位 / 双声道描述解码 PCM，`Codec=0` 对应本次 AAC 传输。首批 206 帧经 FFmpeg 完整解码，包含非静音音频。
- 按 SSRC 独立提取媒体，最多 8 个流，序号支持回绕、重复过滤和有界重排。相同 RTP 时间戳可携带两个不同的 1024 样本 AAC 帧，全部保留，并按样本偏移标记时间。私有捕获有体积上限，所有媒体与时间戳记录均留在忽略目录。
- 时钟使用已认证密钥加密的 UDP 四时间戳交换；每轮 5 个样本，30 秒更新一次，选最小有效往返样本，60 秒后过期。映射处理 90 kHz RTP 的 32 位回绕；这一路映射供后续实际播放调度使用。
- 原生 `OnGetParameterRequest` 只标记对端存活。现改为双方各自每秒发送 GET_PARAMETER，去掉原先的 200 应答；手机日志确认 `alive=1、retryCnt=0`，不再持续记录未请求的保活响应。控制写入串行，退出先发送音频及 HiSight TEARDOWN，再释放底层连接。
- Windows、H3 独立目录各通过 73 项测试，包含真实 UDP 回环解密/提取、损坏帧、错误会话、RTCP 禁用、重排/回绕、重复时间戳、多流上限和时钟过期。第 98 轮约 303 秒会话共收到 11,126 个有效 AAC 包，丢包/重复/拒绝均为 0；视频和方向键同时有效。第 97 轮曾记录 7 个丢包及 3 个重复包，已由有界解析路径处理。
- H3 现有 `carlink-audio` 对约 170 秒的真实 AAC 解码并重采样到 44.1 kHz / 双声道 S16LE，输出 30,079,260 字节，完整解码无错误；独立重复解码耗时 3.278 秒、CPU 累计 7.074 秒、峰值 RSS 8,324 KiB。PC 对该 AAC 的完整解码检查也通过。第 99 轮完成播放暂停后重建音频会话与恢复收流。

第 99 轮最终接收 9,534 帧，丢包和拒绝为 0；覆盖暂停、源端关闭后新建会话及继续播放。9,534 帧均获得本地时钟映射，4,767 帧具有共享 RTP 时间戳下的样本偏移，共 6 个会话/SSRC 分流。退出时音频和 HiSight TEARDOWN 均发送，随后正式 carlink 为 active、私有输入 socket 删除，end0 的 SSH 地址保持有效。

[完整构建 40083411013](https://github.com/Aki894/CarLink/actions/runs/40083411013) 已成功：73 项测试、原生认证回归、ARMv7 编译、完整 firmware job、镜像结构/内容校验与上传全部完成。认证 artifact 12555898805 的下载 ZIP 与包内 10 个文件校验通过；镜像 artifact 12555779690，ZIP 420,403,510 字节，上传 SHA-256 为 c9694eb789b408930cc4fab309a3c0775dabce214e1870e2ea3dfb0703c4c3d3。板上通过 SSH 更新实验源码，完整镜像本轮未重刷。

下一步将 H.264、分流 AAC 解码后的 PCM 和反向输入接到 CarLife 实时接口，完成媒体/导航混音、节拍与有界队列调度，再验证 HU 的实际音画输出及 Lexus Remote Touch。麦克风、通话与无屏常驻产品化仍按后续阶段推进。

## HiCar → CarLife 实时桥接（2026-10-24）

实现版本 `9e2ea35`，通过 `probe.py --live-socket` 显式启用。热点实验期间暂时停止正式 CarPlay，启动独立 CarLife USB 输出，正常退出后恢复原服务。HiCar 尚未加入生产自启或镜像默认运行入口。

- H.264 直接接入既有有界 live 通道，缓存 SPS/PPS，等待真实 IDR 后恢复；AAC/ADTS 经现有原生解码器输出 44.1 kHz 双声道 PCM。
- CarLife 归一化触摸与焦点键转为加密 UIBC。复用已验证的发送边沿节奏，新增 `left_move` 能力及事件 2。用户确认点击、左右滑动；较长按压后的直接点击识别仍需改进，Lexus 优先焦点导航。
- 修复 DMSDP SSRC：原生 `RTPDataReceiver::OnRTPData` 将用途字段与流编号分开，流编号取低 12 位。此前高位变化被当成不同音频流，产生虚假丢包和频繁解码器重建。现保持同一协商/格式的解码器连续。
- 修复 CarLife 心跳调度：连续音频不得提前返回并挤掉视频心跳。Unix Rust 回归覆盖持续 PCM 时仍发送 `0x20002`。
- 输出子进程退出后进行有界恢复并保留 HiCar；界面主动退出、USB 断开和自动重连继续留作产品管理器的场景测试。

第 107 轮 600 秒限时探针最终记录：7,575 个视频访问单元、19,030 个 AAC 帧、246 次输入发送；音频丢包/拒绝均为 0。中间一次约 306 秒采样为 14,317 AAC 帧、15,267 块 PCM，其中 6,009 块非静音。完整探针包含界面切换与输出恢复，原始日志留在忽略目录 `.build/hicar-research/bridge107-board.log`。

单手机音频路由已核实：CarLife 进程存在活动的 44.1 kHz 双声道 AudioTrack，输出是 `AUDIO_DEVICE_OUT_PROXY`；实体扬声器没有对应活动轨。HiCar 手机同时充当 CarLife HU 时，回放也进入手机的投屏媒体路由。实际出声、同步和回声使用独立 HU 或实车验证。

构建与校验：

- Windows 和 H3 各通过 79 项 HiCar 测试。
- [应用构建 40103163348](https://github.com/Aki894/CarLink/actions/runs/40103163348)成功；临时板端应用 SHA256 为 `15b6b8ddaf1fea4212df807c348cda295b410caf9afcc0e4c9ae3d3a93e5cf3f`，与下载清单一致。
- [完整构建 40105347479](https://github.com/Aki894/CarLink/actions/runs/40105347479)的 native、firmware、镜像校验和上传均成功。公开源码 artifact `12563853293`，35 项清单哈希已校验，并与本地源码按统一换行比较一致。
- 镜像 artifact `12564358354`，ZIP 420,710,190 字节，SHA256 `a94760427e41c6a8a7edb426773fc54816b029933d86c347bc2d9dcf6d24b123`。已实际下载，压缩镜像清单校验通过：`4a11f8f04d3cf480906b52a52b22c898aea1d7bb6916d5e67db0c7c299e566ea`。
- 实验结束已恢复正式 `carlink` 为 active、NRestarts=0；Ethernet 管理路由保留。生产应用仍保留原版本，心跳修复版仅在临时实验路径运行。

下一步按 [Rust 迁移与集成](hicar-rust.md)分 R1 协议/FFI、R2 实际会话与媒体、R3 统一连接管理与菜单推进。

## 前期构建记录

提交 bb8b6b3 的 [完整构建 40063371200](https://github.com/Aki894/CarLink/actions/runs/40063371200) 已完成 native 测试、ARMv7 编译、完整 firmware job、镜像校验与上传。认证库 artifact 12548188315 的 ZIP 和包内 10 个成员校验通过；镜像 artifact 12548461553，ZIP 420,203,422 字节，上传摘要 1b4937638b5776b35240842a584a3d2bab6de3a38faad56bef048ba76acaa708。

输入修正提交 9fc8bb1，文档 59d356d。[完整构建 40061050563](https://github.com/Aki894/CarLink/actions/runs/40061050563) 的 native 与 firmware 均成功，镜像检查和上传完成；镜像 artifact 12547795788，ZIP 420,203,138 字节，上传摘要 e1db73cd64b4fa9b5d1d0f2aba6d705a700509c72d1623efcecf66d50544ac3a。认证库 artifact 12547218616 下载后，ZIP 及包内 10 个成员校验通过。本轮通过 SSH 更新独立实验代码，生产镜像未重刷。
