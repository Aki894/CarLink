# Linux 无线 HiCar 实验

目标：悟空派 H3 + AX900/AIC8800 接收鸿蒙 4.2 手机的无线 HiCar，后续复用现有 CarLife 输出。详细阶段和证据见 [实施计划](../../docs/hicar.md)。本目录是手动运行的诊断程序，不随固件安装，不设置开机启动。

`probe.py` 默认使用 BlueZ D-Bus 注册 FDEE 发现广播和 FE35 RFCOMM 服务，只观察收到的字节流。可选启用控制回复、FE36 业务通道和限定手机的蓝牙 SSP 代理，可通过 `--reply-identity` 和 `--auth-library` 接入固定 PIN 身份握手及 HiChain 认证。运行时间默认 60 秒，上限 300 秒；退出时注销自己注册的服务。需要 Linux、BlueZ、Python 3 和 requirements.txt 中的依赖，以及系统 D-Bus/蓝牙管理通道的访问权限（开发板当前使用 root）。

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

每个进程只初始化一个 HUKS 存储。设备身份、HUKS 文件与会话密钥均留在私有目录/内存中。原生回调先入队，再由业务通道发送；认证失败即关闭连接，避免双方错误包互相循环。此实验尚未实现认证后的 Wi-Fi/DMSDP 与媒体通道，不随生产镜像安装或自启。
