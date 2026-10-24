# Linux 无线 HiCar 实验

目标：悟空派 H3 + AX900/AIC8800 接收鸿蒙 4.2 手机的无线 HiCar，后续复用现有 CarLife 输出。详细阶段和证据见 [实施计划](../../docs/hicar.md)。本目录是手动运行的诊断程序，不随固件安装，不设置开机启动。

`probe.py` 默认使用 BlueZ D-Bus 注册 FDEE 发现广播和 FE35 RFCOMM 服务，只观察收到的字节流。可选启用控制回复、FE36 业务通道和限定手机的蓝牙 SSP 代理，可通过 `--reply-identity` 和 `--auth-library` 接入固定 PIN 身份握手及 HiChain 认证。运行时间默认 60 秒，上限 900 秒；退出时注销自己注册的服务。需要 Linux、BlueZ、Python 3 和 requirements.txt 中的依赖，以及系统 D-Bus/蓝牙管理通道的访问权限（开发板当前使用 root）。

在独立 Python 依赖目录中准备 dbus-next 后运行：

```sh
PYTHONPATH=/var/tmp/hicar-python python3 probe.py --model "$HICAR_MODEL" --duration 60
```

`HICAR_MODEL` 为从本地参考资料选取的八位十六进制型号值，通过参数提供。探针默认请求本广播使用 0 dBm 发射功率，可通过 `--tx-power 11` 使用 AX900 已确认支持的 11 dBm；用 `--sequence` 设置发现序号。不修改适配器名称、供电状态、Wi-Fi 或现有 CarLink 服务。同一时间只允许一份本探针。运行前应结束旧版发现探针，避免重复广播。

需要手机主动连接时，使用本地手机地址参数：

```sh
PYTHONPATH=/var/tmp/hicar-python python3 probe.py --model "$HICAR_MODEL" \
  --peer "$PHONE_BT_ADDRESS" --allow-incoming --reply-control \
  --tx-power 11 --sequence 2 --duration 120 --capture-dir /var/tmp/hicar-private
```

`--allow-incoming` 会在需要时临时打开控制器 connectable/bondable，并注册仅接受 `--peer` 所选手机、仅允许 FE35/FE36 服务的 NoInputNoOutput 代理。正常退出/SIGTERM 后恢复原接入设置并注销代理；不打印 SSP 确认码。板上 Python 3.13 没有 HCI control channel 的完整 bind 元组支持，探针通过 libc `bind` 访问 Linux 管理通道，读取设置后再作受控变更。

`--reply-control` 会选择 CCMP v2、明文控制帧，并在 FE36 监听已注册后，响应已实测的 BR/channel 2、业务 19、协议 0 的连接请求。FE35 默认 RFCOMM 通道 23，FE36 使用相邻通道 24，均有 SDP 记录。身份握手和认证需要另行显式启用。

默认 JSON 日志仅输出会话编号、接收长度和流摘要，不输出设备地址或原始内容。需要分析协议时，显式传入 `--capture-dir /var/tmp/hicar-private` 保存有大小限制的原始流；这些数据按私有调试资料管理，下载到本机 `.build/`，不提交。RFCOMM 的单次读取不是完整协议包；控制帧和 JSON 流解析器均处理拆包与粘包。

`nearby_wire.py` 已实现控制帧解包：`0x7f` 标记、最高位结束的 28 位变长整数、flags TLV、负载长度与小端 CRC16-ARC。支持跨读取分片和一次读取多个帧；单帧负载限制 64 KiB、选项限制 256 字节。加密负载保留原样；日志只报告类型、标志、长度，以及明文管理消息的 TLV 类型和长度。遇到未知格式或校验错误时停止解析，原始捕获可继续，便于检查手机实际使用的变体。

CRC 仅用于帧完整性检查。`control.py` 处理可选控制回复，`auth_wire.py` 处理身份/认证 JSON，`hichain.py` 调用独立构建的原生认证库。

手机显示六位连接码输入框。连接码由本机私有配置提供，应用层身份认证由 HiChain 完成。

本地验证（先将依赖加入 Python 路径）：

```sh
python -m unittest discover -s experiments/hicar -p 'test_*.py'
```

## 固定 PIN 与原生认证

```sh
python3 identity.py init
# 仅设备操作员在私有终端读取；不要把输出保存到公开日志。
python3 identity.py show-pin
# 自定义 PIN 从标准输入读取，保留前导零；不要把 PIN 放进 shell 命令历史。
python3 identity.py set-pin
```

`set-pin` 更新后应结束旧会话再重新运行探针。损坏的身份文件会报错，程序不会静默覆盖已有配对身份。无线身份回复仅声明 `has_pin=true`，PIN 不直接发送到无线业务流。

原生库由手动工作流 `hicar-auth.yml` 构建：固定公开源提交、Debian Bookworm 主机测试、ARMv7 交叉编译、ELF 与 SHA-256 清单。主机测试包括 RFC 8032 Ed25519 向量、篡改签名拒绝，以及真实 HiChain 正确/错误 PIN 双端交换。依赖库与许可证随独立实验产物分发。

```sh
PYTHONPATH=/var/tmp/hicar-python python3 probe.py --model "$HICAR_MODEL" \
  --peer "$PHONE_BT_ADDRESS" --allow-incoming --reply-control --reply-identity \
  --auth-library /var/tmp/hicar-auth/libcarlink_hichain.so \
  --auth-store /var/lib/carlink/hicar/keys \
  --tx-power 11 --sequence 3 --duration 300
```

每个进程只初始化一个 HUKS 存储。设备身份、HUKS 文件与会话密钥均留在私有目录/内存中。原生回调先入队，再由业务通道发送；认证失败即关闭连接，避免双方错误包互相循环。实验已实现认证后的热点/TCP、DMSDP 控制端口协商、AES 控制与保活；时间同步、服务与媒体协商继续开发。实验不随生产镜像安装或自启。

## 无线建链测试窗口

在上面的认证命令后增加：

```sh
--network-interface "$AX900_INTERFACE"
```

此选项要求真实认证后端。程序在有线管理通道存在时临时停用 CarLink、保存原无线连接，建立中国大陆区域的 channel 149 / 5745 MHz 热点，并监听 192.168.49.1 的 4005/4006/4007 端口。临时 WPA2 密码随机生成，不打印。手机通过认证和资源协商后自动加入热点，再建立 DMSDP 通道。退出时自动恢复原网络和服务；默认 60 秒，最长 900 秒。

`network_wire.py` 实现 kind 4 资源协商、kind 33 地址/参数交换和业务 20 的真实 TCP 端点。`dmsdp_wire.py` 分别实现无 flags TLV 的模块帧和 16 字节控制端口协商，包含长度、CRC、拆包及粘包检查。

增加 `--dmsdp-options` 后，`control_wire.py` 主动发出 OPTIONS、核对 SET_PARAMETER 并按 CSeq 应答，随后处理加密控制帧与 GET_PARAMETER 保活。`cipher.py` 通过系统 OpenSSL EVP 实现逐帧 AES-128-CTR，使用认证会话密钥前 16 字节及每帧独立 IV。没有认证密钥时不提供 AES。普通服务参数和二进制业务目前只观察，时间同步、服务能力与媒体接收继续实现。

显式指定 `--capture-dir` 时，解密控制记录另存为原捕获路径加 `.plain`，每条为两字节大端长度与明文，权限 0600，受相同接收上限约束。该文件可能包含设备身份和应用信息，与原始捕获一样留在私有目录，不提交。正常日志仅输出方法、字段名称、公共数字能力和消息长度。

手机和开发板各自供电即可，保留开发板网线用于 SSH。测试原始流只保存到指定私有捕获目录，代码和单元测试使用自建示例数据。


## HiSight 接收与本地输入实验

开启真实认证、网络和 dmsdp-options 后，探针自动完成设备/显示服务交换与 HiSight RTSP 协商。视频和反向控制 TCP 都连接后再发 PLAY。hisight_media.py 接收有界 TCP/RTP/PES 并解密 H.264；显式捕获时每路原始媒体保存上限 64 MiB，控制记录总量受探针接收上限约束，文件权限 0600。原始媒体保存在忽略目录。

实验输入接口仅在指定私有捕获目录（0700）中创建 hisight-input.sock（0600），退出时移除。客户端通过本机 Unix socket 发送一行 JSON：key 字段允许 up/down/left/right/enter/back/home/play_pause，或整数 x、y 字段指定 800×480 范围内的单次点按。单条命令上限 256 字节；只接受固定字段，返回 OK 或 ERROR。它用于接收端联调，后续由 CarLife/Remote Touch 事件接口替代。

首次使用的 HiCar 声明需要用户确认。视频解码、原始帧检查和输入验证分别记录。音频通道和周期时钟同步尚待实现；当前媒体解包测试覆盖视频，音频包不会作为有效音频输出交付。
