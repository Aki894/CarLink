# Linux 无线 HiCar 实验

目标：悟空派 H3 + AX900/AIC8800 接收鸿蒙 4.2 手机的无线 HiCar，后续复用现有 CarLife 输出。详细阶段和证据见 [实施计划](../../docs/hicar.md)。本目录是手动运行的诊断程序，不随固件安装，不设置开机启动。

`probe.py` 默认使用 BlueZ D-Bus 注册 FDEE 发现广播和 FE35 RFCOMM 服务，只观察收到的字节流。可选启用控制回复、FE36 业务通道和限定手机的蓝牙 SSP 代理，尚未实施 HiChain 连接码认证。运行时间默认 60 秒，上限 300 秒；退出时注销自己注册的服务。需要 Linux、BlueZ、Python 3 和 requirements.txt 中的依赖，以及系统 D-Bus/蓝牙管理通道的访问权限（开发板当前使用 root）。

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

`--reply-control` 会选择 CCMP v2、明文控制帧，并在 FE36 监听已注册后，响应已实测的 BR/channel 2、业务 19、协议 0 的连接请求。FE35 默认 RFCOMM 通道 23，FE36 使用相邻通道 24，均有 SDP 记录。此选项接受业务流，不回复 HiChain 认证结果。

默认 JSON 日志仅输出会话编号、接收长度和流摘要，不输出设备地址或原始内容。需要分析协议时，显式传入 `--capture-dir /var/tmp/hicar-private` 保存有大小限制的原始流；这些数据按私有调试资料管理，下载到本机 `.build/`，不提交。RFCOMM 的单次读取不是完整协议包，后续解析器必须处理拆包和粘包。

`nearby_wire.py` 已实现控制帧解包：`0x7f` 标记、最高位结束的 28 位变长整数、flags TLV、负载长度与小端 CRC16-ARC。支持跨读取分片和一次读取多个帧；单帧负载限制 64 KiB、选项限制 256 字节。加密负载保留原样；日志只报告类型、标志、长度，以及明文管理消息的 TLV 类型和长度。遇到未知格式或校验错误时停止解析，原始捕获可继续，便于检查手机实际使用的变体。

CRC 仅用于帧完整性检查。`control.py` 处理可选控制回复，HiChain 认证由后续独立实现。

手机可能显示六位码输入框。当前程序没有认证后端，此时退出输入框即可。FE35 使用参考实现的非配对 RFCOMM 传输，应用层身份认证由后续 HiChain 阶段完成。

本地验证（先将依赖加入 Python 路径）：

```sh
python -m unittest discover -s experiments/hicar -p 'test_*.py'
```
