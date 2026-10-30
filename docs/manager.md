# R3 连接管理与设备菜单

CarLink 使用一个 Rust 管理器保持 CarLife USB 常驻，并独占调度一个 CarPlay 或 HiCar 无线后端。切换手机时保留 USB 会话；已连接手机不会被后来上车的另一部手机抢占。R3 的菜单、网页、未知 HiCar 添加与策略代码已实现，完整构建及既有台架回归已完成；真实触控板和多手机场景继续验收。

## 菜单与连接生命周期

已连接时，屏幕菜单依次显示“返回投屏”、当前协议（点击切换另一协议）、连接/断开设备、声音调节、默认协议；打开菜单后焦点默认位于第一行。未连接时隐藏“返回投屏”，保持当前协议等待；手动断开后暂停回连，点击连接设备重新启用并开放配对。HiCar 配对行显示本机固定 PIN。三连返回键打开菜单，普通返回送给手机；菜单及声音页内按返回恢复投屏。

声音页仅提供音量加减和返回菜单，没有独立静音按钮。范围 0–100%，每次调整 5%，设置保存到本机，重启和协议切换后继续使用。软件音量作用于音乐、导航/语音及通话的下行 PCM，采用约 20 ms 渐变，不改变麦克风上行；100% 为原始幅度，不额外放大。网页同步提供音量滑块。

HiCar 已配对手机的主动回连使用 FE35 Nearby 控制、CCMP business 19 协商及动态 RFCOMM 认证通道。CarPlay 与 HiCar 接收端独占无线资源；切换先终止旧源、退休本地连接并隔离旧事件，再启动新源。CarLife USB 常驻。

管理接口新增：`switch_protocol`（可附 `protocol`）、`connect_device`、`default_protocol`（`mode=last/carplay/hicar`）。详细状态与设计见 [connection-management.md](connection-management.md)。

## 使用入口

- 开机等待连接时显示 CarLink 设备菜单；通过触摸或 Lexus 焦点方向、确认、返回操作。真实 Remote Touch 的原车快捷键仍待实车确定。
- 投屏期间，在当前热点内打开 `http://10.77.0.1:8088`（CarPlay）或 `http://192.168.49.1:8088`（HiCar），选择手机、添加设备或打开车机菜单。有线维护地址是 `http://192.168.77.2:8088`。
- CarPlay 桌面使用上游已有车形图标，名称为“CarLink菜单”，点击打开本地菜单。HiCar 默认返回车机图标也接入同一菜单；当前保留系统图标与名称，不修改蓝牙/热点名称。菜单显示期间保持无线连接和音频；返回投屏时请求源关键帧，HiCar 同时发送 DMSDP 显示服务恢复指令（203）。
- 菜单添加设备会在当前已有投屏时要求确认。网页添加与忘记操作也有明确确认。

菜单只编码本地静态 UI；源手机的压缩 H.264 与 PCM 直接转发。单个 live mux 持有 CarLife 输出连接，菜单和无线后端不会竞争同一个 Unix socket。队列按帧数、字节和等待时间限界，切换后等待新关键帧，清理旧源的未发送媒体与触点。

## 连接策略

`last` 优先上次稳定使用的手机，再尝试同协议其他允许自动连接的设备；`favorite` 只尝试指定地址；`manual` 每次等待选择。HiCar 初次连接超时会轮换同协议目标；已连接、通话和配对窗口不轮换。首选只影响后续选择，活动会话继续显示其真实协议；显式跨协议选择优先于旧首选，遗忘首选后恢复 last。修复与回归见[完成情况核对](status-audit.md)。改变策略保持当前会话，`disconnect` 会断开并暂停自动连接，`resume` 恢复当前策略。设备上限为八台。

添加 iPhone 或华为会启动对应协议的 120 秒配对窗口，另一协议同时停止。HiCar 不要求预先知道手机蓝牙地址；首个实际建立通道的手机成为本轮唯一目标，仍需通过 HiChain 认证。源媒体稳定且已交付 CarLife 后才登记成功使用。配置文件中的 devices 仅作为首次初始化数据，忘记的设备不会因重启复活。

忘记操作删除目标的登记和 BlueZ 配对，暂停自动连接；共享 MFi、HiCar 设备身份、固定 PIN 与 HUKS 存储保持稳定。

## 安装与控制

[示例配置](examples/manager.json) 不含手机地址或认证材料。镜像随附 `/usr/share/carlink/manager.example.json`；配置为 `/etc/carlink/manager.json` 后，`carlink-run` 启动 Rust 管理器。配置、设备状态使用 0600，私有目录使用 0700。HiCar 认证原生包、Python 过渡适配器、菜单库及字体分别经过清单和 ARM ABI 校验。部署时保留已有身份材料，并留存上一运行版本用于回滚。

```sh
carlink manager ctl /run/carlink-manager/control.sock '{"op":"status"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"devices"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"pair","protocol":"hicar"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"pair","protocol":"carplay"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"menu","visible":true}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"menu","visible":false}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"volume","percent":70}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"policy","mode":"manual"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"disconnect"}'
carlink manager ctl /run/carlink-manager/control.sock '{"op":"resume"}'
```

网页仅接受热点、有线管理和回环网段；修改操作要求同源 JSON POST，限制请求长度并拒绝重复长度、分块请求和未列入白名单的操作。页面不提供密钥或配对 PIN。

## 恢复与验证

明确认证失配或连续三次连接拒绝时，只清理目标 iPhone 的旧 BlueZ 配对并开放有界重配窗口；普通离线保留记录。CarLife 故障单独恢复，已建立的无线源保留。每次切换增加代际，忽略旧后端回调。

验证入口：`scripts/test-manager.py`（生命周期、策略、配对和遗忘）、`scripts/test-manager-web.py`（真实 HTTP 边界）、`scripts/test-manager-menu.py` 与 Rust `manager_menu::`（H.264 实际解码、菜单焦点、PCM 透传、关键帧恢复、USB 重连）。台架、完整固件与 Lexus 实车结果记入 [validation.md](validation.md)。HiCar 的 R2 运行链路与 CarPlay 媒体桥接均已迁入 Rust，Python 实验代码仅保留为参考与测试。当前缺口见[完成情况核对](status-audit.md)。

## 断连、启动与返回键入口（2026-10-25）

无线源结束、媒体连接关闭或后端退出时，立即恢复 CarLink 菜单。默认 last 策略先给原手机短暂恢复机会，再尝试其他允许自动连接的设备；等待时菜单可继续选择。USB 输出独立恢复。

已登记手机的无线连接建立后独立保存 preferred，开机优先该手机及协议；last_used 继续要求车机 USB 和稳定媒体均就绪。暂停自动连接只对本次开机有效，手动选择策略继续持久保存。

投屏期间连续按三次返回键打开菜单，相邻按键间隔不超过 350 ms。三次按键不转发至手机，当前无线会话与音乐保持。普通单次或两次返回最多等待 350 ms 判定后按原顺序送出；视频关键帧请求不打断判定。菜单内单次返回可回到已连接投屏。

快捷键已通过抽象 key-press 与模拟车机回归；Lexus 实车的返回键原始消息仍需实车回归。
