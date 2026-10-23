# Linux 无线 HiCar 实施计划

## 目标与现状

目标平台为悟空派 H3（ARMv7、512 MiB）、AX900/AIC8800、Linux/BlueZ，手机范围为鸿蒙 4.2。HiCar 接收后复用现有 CarLife 输出，保留有线 SSH。开发与现有 CarPlay 接收端隔离，先用手动诊断程序验证协议，再接入常驻服务。

2026-10-23 已实测：FDEE BLE 广播使手机发现设备，点击后出现六位连接码输入框。参考 Android 程序的连接码由安全随机数生成，供 HiChain 原生认证使用。当前尚未实现连接码认证、Wi-Fi 建链或 HiCar 媒体接收。

## 实施顺序与阶段验收

| 阶段 | 实现内容 | 验收证据 |
| --- | --- | --- |
| 1. 发现与传输 | FDEE 广播；注册 FE35 RFCOMM 服务；限制运行时间、连接数和捕获大小 | 手机发现；BlueZ 注册成功；手机真实连接与首包；退出后注销，原 CarLink 服务保持正常 |
| 2. Nearby 封装 | 确认 BR 控制通道、业务 19、FE36 业务通道和实际帧边界；实现拆包、粘包、长度校验与状态机 | 手机抓包与参考源码相互对应；分片、合包、畸形长度和断开重连用例通过 |
| 3. 身份认证 | 实现握手 JSON、会话连接码、HiChain 数据交互与可信设备存储 | 新配对成功；错误码失败路径；断开后可信重连；密钥与配对资料只落在设备私有目录 |
| 4. 网络与接收 | 按手机实际请求确定 AP/P2P 模式、网络参数、端口与控制协议；协商视频、音频和输入能力 | 无线会话建立；收到可解码视频及连续音频；输入回传被手机接受 |
| 5. CarLife 桥接 | 接入现有本地媒体接口；H.264 格式与分辨率适配；音频格式、时钟和有界队列；触控/Remote Touch 回传 | 华为 HiCar 源与独立 CarLife HU 联调；随后 Lexus 实车连接、声音、控制、重连与延迟验证 |
| 6. 交付与启动 | 常驻服务、互斥选择 CarPlay/HiCar、日志与失败恢复、固件打包 | GitHub Actions 完整 firmware job、镜像校验和上传成功；冷启动、长时间播放及异常断电恢复实测 |

阶段 1 的发现已通过，RFCOMM 探针已开始实现。GATT `046A/046C` 与 BR 通道的关系需要通过手机实际访问确认；参考实现的 `046C` 读取路径会触发 P2P 状态机并返回 MAC 或 `busy`，不能简单当作连接码特征。先观察实际链路，再补必要服务。

首版探针验证记录（2026-10-23）：

- 本地 7 项测试通过，覆盖广播字段、无效型号、D-Bus 签名、SDP 通道、分片字节流、大小上限和取消清理。
- 开发板实测通过 BlueZ 广播/FE35 注册、控制器实际地址上的 RFCOMM 通道占用、SIGTERM 清理、退出后通道释放；最终运行 stderr 为空。
- 退出后 BlueZ `ActiveInstances=0`，原 `carlink` 服务仍为 `active`。探针没有加入开机启动，也没有修改固件或构建流程。
- 手机此前已完成发现并出现六位码输入框；新版 RFCOMM 的手机首包和身份认证待验证。

后续进展（同日）：新增独立实现的 Nearby 控制帧解析器，支持逆向确认的变长整数、TLV、CRC16-ARC、拆包/粘包和长度限制；加密负载只记录元信息。本地测试总计 14 项通过，其中 7 项帧解析测试在 H3 上也通过，探针监听与退出清理回归通过。首轮 180 秒捕获窗口没有收到 RFCOMM 首包，手机实际交互验证继续等待操作。

实机推进结果（同日）：

1. 手机能收到广播，但部分轮次进入 `FindFarawayState`；探针增加广播功率和序号参数后，重新观察到发现窗口。功率和序号同时变更，分别影响后续再对照。
2. 第一处接入阻碍为控制器 `connectable` 关闭，HCI 明确记录开发板以 `Unacceptable BD_ADDR` 拒绝华为手机的 ACL 请求。
3. 临时打开接入后，下一处阻碍为 SSP 确认被 BlueZ 拒绝，日志为 `device_confirm_passkey: Operation not permitted`。增加限定手机的 NoInputNoOutput 代理并临时开放 bondable 后，FE35 收到真实首包。
4. 手机请求 CCMP v5、传输加密偏好值 2；探针选择参考实现支持的 v2/明文控制帧后，手机继续发送 channel 2、业务 19、协议 0 的流创建请求。兼容协商已实测通过。
5. 注册 FE36/SDP 并回复真实可用的业务端点后，手机建立第二条 RFCOMM 连接，发送 343 字节 JSON：`oper_type=1`、`version=14.2.0.400`；嵌套 `data` 包含 `deviceId`、`devicename`、`has_pin`、`usb_connection` 等字段，`has_pin=true`。此包未含 `discover_type`，后续解析须兼容缺省字段。
6. 手动停止后，两条流正常关闭，临时 bondable/connectable 恢复关闭，原 `carlink` 服务仍 active。阶段 1 和阶段 2 的已观测路径通过，接下来实施双方身份握手及 HiChain。

当前本地和 H3 板上均为 20 项测试通过。原始业务数据和设备身份留在设备私有捕获目录与本机 `.build/hicar-research/`，不写入公开文档或源码。运行事件证据：设备 `/var/tmp/hicar-agent-11.log`，本机 `.build/hicar-research/session-11-events.jsonl`；原始业务流单独保存为私有资料。

本地运行证据为 `.build/hicar-research/smoke-result.json`，认证 ELF 清单为同目录 `native-inventory.json`。测试脚本、参考二进制和运行证据按本地研究资料管理。

## 认证技术路线与决策点

认证是优先调查的依赖，同时推进传输层取证，避免媒体接口写完后才发现认证不可用。参考 APK 通过 JNI 调用 `libConnectAuthNative`、`libHwDeviceAuthSDK`、`libHwKeystoreSDK` 等库；先核对 ELF 架构、动态依赖、导出接口和初始化要求。

1. 优先寻找参考资料中已有的 ARMv7 Linux 原生接收端/认证实现，确认其运行依赖与使用条件。
2. Android 原生库若依赖 Bionic、JNI 或 Android 服务，先记录具体依赖，再评估小范围适配成本。
3. 如需自行实现认证，比较公开 DeviceAuth/HiChain 实现与手机实际消息的版本、消息结构和密码算法。公开实现与当前手机的兼容性需实测确定。

首轮 ELF 检查已确认：参考包提供 ARMv7 库，`libHwDeviceAuthSDK` 导出 `start_pake`、`receive_data`、`authenticate_peer` 等接口；包装库导出 JNI 方法并依赖 `liblog.so`，底层认证/密钥库存在 Bionic 的 `__sF` 等导入。下一步优先检查底层 C 接口和回调结构，以及可对应的公开实现，评估替换包装层和运行库适配的工作量。

已锁定公开研究基线：OpenHarmony `security_deviceauth` 的 `OpenHarmony-v1.1.4-LTS`，提交 `552edb521d0bd7c17f2dab2791e7cdc3de284405`。其 [hichain.h](https://github.com/openharmony/security_deviceauth/blob/552edb521d0bd7c17f2dab2791e7cdc3de284405/interfaces/innerkits/deviceauth_lite/hichain.h) 中的 14 个公开 API 名称全部存在于参考库导出符号中。回调结构、编译宏导致的 ABI 差异、PAKE 版本和 HUKS 存储依赖仍需逐项核对；暂未构建或加载该认证库。比较清单保存在 `.build/hicar-research/auth-api-comparison.json`。

在认证通过前，不向手机发送虚假的成功结果，也不以任意六位码反复尝试。研究脚本不持久化可信身份；正式实现再设计设备私有存储和清除配对流程。

## 代码与运行方式

- 探索阶段使用 Python，便于对照抓包调整 D-Bus 和协议字段；代码位于 [experiments/hicar](../experiments/hicar/README.md)，不进入固件和开机服务。
- 协议稳定后，常驻接收端采用 Rust，与现有 CarLife 应用共享进程管理和有界媒体队列设计。是否复用进程按接口与故障隔离需求决定。
- 音视频尽量直接转发。先验证 HiCar 输出编码、分辨率、音频格式，再决定是否需要转换，避免 H3 上不必要的软件解码/重编码。
- AX900 的 AP/P2P 能力与并发模式在网络阶段专项验证。切换无线模式前安排受控测试窗口，避免与 CarPlay 同时抢占热点。
- 当前同一台华为手机承担供电、CarLife HU 测试和 HiCar 源测试。媒体桥接阶段需要第二个 HU 测试端或实车；现在保持现有接线，先做无线发现和协议验证。

## 逆向依据与本地证据管理

本地参考资料在 `hellocar/reference`，反编译、抓包和临时依赖在 CarLink 的 `.build/hicar-research/`。这些目录不进入公开仓库。以下仅记录接口观察和定位点，探针独立编写：

| 参考定位 | 观察 |
| --- | --- |
| Nearby `f/a.java`、`f/d/a.java`、`PublishDeviceInfo` | FDEE service data：版本、TLV 类型、型号/子型号、序号 |
| Nearby `g/a/a.java` | `NearbyServerSocket` 使用 FE35 非配对 RFCOMM 服务 |
| Nearby `transfertype/message/MessageManager.java` | 部分业务连接路径使用 FE36；业务协商类型 9/10，需继续确认帧封装 |
| Nearby `BleDevice`、`P2pChannelStateMachine.t2` | `046C` 读请求返回 P2P 地址或 `busy` 并触发状态转换 |
| AuthAgent `PinCodeProducer` | 六位数字由 `SecureRandom` 逐位生成，允许前导零 |
| AuthAgent `NearbyConnectControl` | 业务 19；握手 `oper_type=1`，认证转发 `oper_type=3`；PIN 交给原生 HiChain |

官方资料：[HiCar 平台](https://developer.huawei.com/consumer/cn/HiCar)、[连接方式接口](https://developer.huawei.com/consumer/cn/doc/HiCar-Guides/obtain-connection-mode-0000001052220416)。协议细节以本机参考程序与鸿蒙 4.2 手机实测为依据。

探针的 BlueZ 接口依据：[ProfileManager](https://github.com/bluez/bluez/blob/master/doc/org.bluez.ProfileManager.rst)、[LEAdvertisement](https://github.com/bluez/bluez/blob/master/doc/org.bluez.LEAdvertisement.rst)。当前开发板 BlueZ 还会通过 `Properties.Set` 回报实际 `TxPower`，探针已处理该行为。
