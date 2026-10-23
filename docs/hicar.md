# Linux 无线 HiCar 实施计划

## 目标与当前结果

目标平台为悟空派 H3（ARMv7、512 MiB）、AX900/AIC8800、Linux/BlueZ，手机范围为鸿蒙 4.2。HiCar 接收后复用现有 CarLife 输出，保留有线 SSH。探索程序与现有 CarPlay 接收端隔离，暂不随生产固件安装或自启。

2026-10-23 已实测走通：FDEE 发现 → FE35 控制通道 → FE36 业务 19 → 身份 JSON → 固定六位 PIN → HiChain 认证成功。手机为 HiCar 14.2.0.400；H3 上两次独立进程测试均得到结果 `[2,3,0]`，即处理中、密钥协商完成、认证成功。两次使用同一持久化身份与 PIN。

认证后的 Nearby ResourceManager、5 GHz 热点加入、TCP 控制通道和业务 20 数据通道已实测通过。DMSDP 控制端口、OPTIONS/SET_PARAMETER 协商、AES-128-CTR 解密和加密保活应答也已在 H3 与鸿蒙 4.2 手机上通过。下一阶段处理时间同步、设备/服务能力及媒体协商；完整会话完成后的免 PIN 自动重连另作验收。

## 阶段与验收

| 阶段 | 状态与实现内容 | 验收方式 |
| --- | --- | --- |
| 1. 发现与传输 | 已通过；FDEE、FE35/FE36、限定手机 SSP 代理、退出恢复设置 | 手机发现并建立双通道；SIGTERM 释放通道；原 CarLink 保持 active |
| 2. Nearby 封装 | 已通过所观察 BR 路径；变长整数/TLV/CRC、拆包粘包、有界捕获 | 真实首包及业务 19 请求与参考实现对应；异常长度/校验测试 |
| 3. 身份认证 | 固定 PIN、持久化身份/HUKS、公开 Linux HiChain 后端已实测成功 | 正确/错误 PIN 本机对照；华为手机两次成功；可信免 PIN 重连待完整会话完成后验收 |
| 4. 网络与接收 | 热点/TCP/业务 20/控制端口、AES 控制及保活通过；服务与媒体待实现 | 已收到时间同步请求和模块 3 二进制业务；后续验收服务协商、连续视频、音频与输入回传 |
| 5. CarLife 桥接 | 待网络阶段；接入本地媒体接口、有界队列、Remote Touch | 独立 CarLife HU 联调，再测 Lexus 的声音、控制、重连与延迟 |
| 6. 启动与交付 | 待桥接稳定；服务互斥、日志、恢复、固件集成 | 完整 firmware job、镜像校验和上传；冷启动、长时播放与断电恢复 |

## 已确定的协议与适配

- HCI 曾因 `connectable` 关闭以 `Unacceptable BD_ADDR` 拒绝手机；临时开放后，SSP 又因缺少代理被拒绝。探针现在只为指定手机、FE35/FE36 注册 NoInputNoOutput 代理，退出恢复原 connectable/bondable 设置。
- 手机请求 CCMP v5、加密偏好 2；参考实现兼容的 v2/明文控制封装已实测。CRC 用于帧完整性；身份认证由后续 HiChain 完成。
- FE35 默认 RFCOMM 23；FE36 默认 24。BR 流创建请求为 channel 2、业务 19、协议 0。
- 首包是 `oper_type=1`、`version=14.2.0.400`，`data` 为嵌套 JSON 字符串，允许省略 `discover_type`。身份回复声明 `has_pin=true`，无线业务流不直接传送 PIN。
- 认证请求为 `oper_type=3`，内层 `message=1/2/3`；手机声明版本范围 `1.0.0..2.0.31`。后端返回相应的 `32769/32770/32771`，完成 PAKE 和身份交换。
- 有未完成的网络连接时，手机可能直接重试 P2P，跳过新一轮认证。本轮结束旧 HiCar 连接进程后，恢复了 BR 身份/认证流程；没有清除手机应用数据。

## 认证后端与固定 PIN

本地参考 APK 的认证库依赖 Bionic、JNI 与 Android 日志。采用公开 OpenHarmony 源码编译 Linux C 后端，通过 Python ctypes 接入，避免在 glibc 上模拟整个 Android 运行库。公开源提交固定于 [sources.json](../experiments/hicar/native/sources.json)；DeviceAuth 基线为 [OpenHarmony-v1.1.4-LTS hichain.h](https://github.com/openharmony/security_deviceauth/blob/552edb521d0bd7c17f2dab2791e7cdc3de284405/interfaces/innerkits/deviceauth_lite/hichain.h)。包名/服务名按本地 JNI 常量使用 `com.huawei.authagent` / `SFXTEST`。

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

无线 HiCar 阶段不需要手机连接开发板 OTG；开发板现已由独立 USB 电源供电，手机可独立充电。CarLife 桥接阶段再接第二个 HU 测试端或实车。

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
