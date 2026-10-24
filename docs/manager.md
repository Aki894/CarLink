# R3 连接管理器

第一段实现常驻 CarLife USB、单无线后端、代次隔离、持久化设备选择和定向配对恢复。车机菜单将使用同一控制接口；当前通过私有 Unix socket 操作。HiCar 沿用已验证的 Python 媒体适配器和 Rust 认证后端。

`/etc/carlink/manager.json` 存在时，`carlink-run` 启动 Rust 管理器；未配置时保留原 CarPlay 启动方式。配置与设备记录应为 0600、所在目录为 0700。[示例配置](examples/manager.json)不含手机地址、认证身份或密钥。

HiCar Python 源码及固定版本 dbus-next 随接收端 artifact 打包；HiChain 原生库使用认证 workflow 的独立、带校验清单的 ARMv7 artifact。开发板部署还需配置既有的 HiCar 私有身份/HUKS 目录及 CarPlay MFi 身份。迁移私有数据时保留原随机种子、固定 PIN 和 model/MAC 派生方式。

## 控制接口

```sh
carlink manager ctl /run/carlink-manager/control.sock '{"op":"status"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"devices"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"remember","device":{"protocol":"hicar","address":"02:00:00:00:00:02","name":"我的华为","auto":true}}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"connect","protocol":"hicar","address":"02:00:00:00:00:02"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"pair","protocol":"carplay"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"repair"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"pause"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"disconnect"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"resume"}'
```

示例地址是占位值。`connect` 明确切换到已登记手机；`pause` 仅暂停自动选择，`disconnect` 同时停止无线后端；`resume` 恢复上次稳定手机优先的自动连接。新 CarPlay 使用限时配对窗口；本段新增 HiCar 手机仍需先登记其蓝牙地址。菜单、未知 HiCar 发现入口和按键交互继续在后续 R3 实现。

## 恢复行为

- 仅一个后端拥有蓝牙与 AX900。停止旧进程组并完成退出后启动新后端，CarLife PID 保持不变。
- 无线后端崩溃与 USB 输出崩溃分别恢复；USB 进程退出时，清理其已结束的本地 socket 后单独重启输出。
- 暂时离线采用有界尝试和候选轮换，活动投屏不会被其他手机自动抢占。
- 对目标 iPhone 的明确认证失败，或连续三次主动拒绝连接，停止旧回连、仅删除该目标 BlueZ 配对，然后开放限时重新配对。盒子已经删除该目标时直接开放窗口。短暂不可达保留配对数据。
- 配对恢复有时间窗和冷却，MFi、HiCar PIN/HUKS 和其他手机记录保留。稳定媒体会话后更新最近使用记录并关闭可发现、可配对状态。

`scripts/test-manager.py` 使用合成进程组和模拟蓝牙工具覆盖互斥切换、USB PID 保持、独立 USB 恢复、离线保护、认证失败/连续拒绝恢复、目标隔离和配对窗口关闭。
