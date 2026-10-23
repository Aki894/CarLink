# Linux 无线 HiCar 实施计划

## 目标与当前结果

目标平台为悟空派 H3（ARMv7、512 MiB）、AX900/AIC8800、Linux/BlueZ，手机范围为鸿蒙 4.2。HiCar 接收后复用现有 CarLife 输出，保留有线 SSH。探索程序与现有 CarPlay 接收端隔离，暂不随生产固件安装或自启。

2026-10-23 已实测走通：FDEE 发现 → FE35 控制通道 → FE36 业务 19 → 身份 JSON → 固定六位 PIN → HiChain 认证成功。手机为 HiCar 14.2.0.400；H3 上两次独立进程测试均得到结果 `[2,3,0]`，即处理中、密钥协商完成、认证成功。两次使用同一持久化身份与 PIN。

下一处接入点是认证后的 Nearby ResourceManager / P2P 数据通道。手机发送 kind 4 / TLV 1，申请 channel 3 的 Wi-Fi/P2P 能力。网络、DMSDP 和 HiCar 媒体接收待实现；完整会话完成后的免 PIN 自动重连另作验收。

## 阶段与验收

| 阶段 | 状态与实现内容 | 验收方式 |
| --- | --- | --- |
| 1. 发现与传输 | 已通过；FDEE、FE35/FE36、限定手机 SSP 代理、退出恢复设置 | 手机发现并建立双通道；SIGTERM 释放通道；原 CarLink 保持 active |
| 2. Nearby 封装 | 已通过所观察 BR 路径；变长整数/TLV/CRC、拆包粘包、有界捕获 | 真实首包及业务 19 请求与参考实现对应；异常长度/校验测试 |
| 3. 身份认证 | 固定 PIN、持久化身份/HUKS、公开 Linux HiChain 后端已实测成功 | 正确/错误 PIN 本机对照；华为手机两次成功；可信免 PIN 重连待完整会话完成后验收 |
| 4. 网络与接收 | 下一阶段；按 ResourceManager 请求实现 P2P/AP 参数、端口和 DMSDP | 建立无线会话；连续收到可解码视频和音频；输入回传有效 |
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

运行命令、配置和测试入口见 [实验说明](../experiments/hicar/README.md)。目前 Python 共 28 项测试在 Windows 与 H3 通过。

## 构建与证据

手动工作流 `.github/workflows/hicar-auth.yml` 先构建和测试主机库，再交叉编译 ARMv7，打包依赖、许可证、ELF 信息和 SHA-256 清单；随后调用完整生产镜像刷新任务。

[Actions 39930292376](https://github.com/Aki894/CarLink/actions/runs/39930292376) 的 native 和 firmware job 全部成功，镜像验证与产物上传完成。认证独立产物下载后已核对 artifact SHA-256、每个成员校验和，H3 依赖加载及原生测试通过。本轮生产镜像沿用已成功的应用/驱动基线；HiCar 实验通过 SSH 单独部署，无需刷卡。

私有证据保存在 `.build/hicar-research/`：主机/ARM 构建日志、原生库清单、手机日志与原始业务流。板上 `/var/tmp/hicar-agent-16.log`、`hicar-agent-17.log` 为两次真实认证事件记录，包含状态与长度，不输出 PIN 或密钥。已有 `smoke-result.json` 保留探针监听与清理证据。

## 下一阶段实现顺序

1. 解析 kind 4 资源协商：请求 TLV 1 内首个变长整数是目标通道（本机为 3），随后为 channel 3 的详细能力 TLV；回复 TLV 2 和结果 TLV 7 的状态机按参考实现衔接。
2. AX900 驱动已声明 P2P-client、P2P-GO、P2P-device 及 AP 模式，组合上限包含一个 P2P client/GO。在有线 SSH 下停止 CarPlay 无线会话后，实测 GO 建立、手机加入和 DHCP，避免现有热点占用。
3. 实现参数与端点交换，先建立网络和控制通道，再处理 DMSDP 认证/会话。仅在网络就绪后声明相应能力。
4. 验证 HiCar 输出的编码、分辨率、音频和输入事件，再复用现有 CarLife 输出；尽量直转 H.264，减少 H3 上的软件解码/重编码。

当前同一台华为手机承担供电、CarLife HU 测试和 HiCar 源测试。媒体桥接阶段使用第二个 HU 测试端或实车；现阶段保持现有接线。

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
