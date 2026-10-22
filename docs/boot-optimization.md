# Linux 启动精简方案

适用：悟空派 H3 / Orange Pi Zero、Debian 13 Armbian minimal、512 MiB RAM、AX900/AIC8800，内核 `6.18.57-current-sunxi`。配置直接写入待构建镜像；本轮不修改运行中的开发板、不重启、不做手机联调。

## 已定位的启动等待

2026-10-22 的只读诊断：内核 7.244 秒，用户空间 92.839 秒，总计 100.084 秒。NetworkManager 自身耗时 3.544 秒。

现有 `getty.target` 包含 `serial-getty@ttyGS0.service`。其模板通过 `BindsTo=dev-%i.device`、`After=dev-%i.device`、`Before=getty.target` 依赖 USB 串口设备。系统已经为 CarLife AOA FunctionFS 禁用旧 `g_serial`，不会创建 `ttyGS0`；实际 `dev-ttyGS0.device` 为 inactive，设备任务超时为 1 分 30 秒。这是本轮优先处理的无效启动依赖。移除残留 wants 链接并 mask 对应实例，保留真实调试串口 `ttyS0`。

上述数据是修改前基线。新镜像的实际开机秒数、CarLink 就绪时间，安排下次通电验证。

## 实施内容

| 项目 | 配置与理由 |
| --- | --- |
| 残留 USB 串口 | mask `serial-getty@ttyGS0.service`，删除其唯一已确认的 wants 链接；消除不存在设备的启动等待 |
| 本地网络 | mask `NetworkManager-wait-online.service`；CarLink 需要本地热点与蓝牙，启动不依赖 Internet 或远程挂载 |
| CarLink 自启动 | `carlink.service` 在 NetworkManager 和 BlueZ 进程启动后运行 `/usr/local/bin/carlink-run`，不引用 `network-online.target`；动态硬件就绪由启动器处理 |
| 认证与恢复 | 缺任一 MFi 文件时跳过启动；失败后每 5 秒重试；运行目录权限 0700，子进程统一停止 |
| 运行目标 | `default.target` 指向 `multi-user.target`，使用现有无桌面基础系统 |
| 非必需服务 | mask ModemManager、motd-news service/timer、man-db timer；设备不使用蜂窝调制解调器，登录新闻和手册定时索引无需常驻触发 |
| 安全更新 | 保留 APT、签名验证、更新服务与 timer 启用状态；下载检查改为开机 10 分钟后，升级检查 15 分钟后，各增加最多 5 分钟随机延迟，之后每日重复 |
| 日志 | journald 保留持久日志，磁盘上限 16 MiB、运行时 8 MiB、单文件 4/2 MiB、保留 7 天；30 秒同步减少 SD 卡写入竞争 |

APT timer 由本轮配置改为单调时钟调度，避免离线车辆的旧日历任务在开机瞬间补跑。连续运行短于 10 分钟时不会启动自动下载；维护时仍可通过 `apt update` / `apt upgrade` 主动更新。原来未启用的 timer 保持未启用。

保留 NetworkManager、BlueZ、D-Bus、udev、有线 SSH、时间同步、文件系统检查、温控、zram swap、现有 ramlog、串口救援及其原始配置。未删除系统软件包，未更改 CPU 频率、USB autosuspend、内核模块、bootloader 或 `boot.scr`。这让首轮优化集中在已确认等待和启动任务竞争，后续再按计时结果缩减。

## 离线安装、验证与恢复

配置源在 `firmware/boot-profile/`，操作工具只读取/写入挂载的镜像文件，不执行镜像内程序。

```sh
python3 scripts/install-boot-profile.py --root /mnt/carlink-image
python3 scripts/install-boot-profile.py --root /mnt/carlink-image --verify
python3 scripts/install-boot-profile.py --root /mnt/carlink-image --revert
python3 scripts/test-boot-profile.py
```

原文件内容、权限、符号链接目标及安装内容记录于镜像的 `/etc/carlink/boot-profile-backup.json`（0600）。重复安装相同配置幂等；恢复仅处理显式允许的配置路径，恢复前检查这些路径仍与安装状态一致，遇到管理员后续改动即停止。拒绝宿主机根目录、符号链接父目录、越界路径和被替换成链接的备份清单。清理由逐文件恢复完成，不递归删除未知目录。

CI 应在最终镜像校验阶段调用 `--verify`，并将配置源与安装器一起纳入构建来源。硬件验证留待用户休息后：冷启动计时、CarLink 就绪时间、SSH 连通、热点/蓝牙重连、CarLife USB 枚举。

## 官方依据

- [systemd：网络同步目标](https://systemd.io/NETWORK_ONLINE/)：面向接收连接的服务通常无需阻塞至网络上线；应用应处理动态连接变化。本项目创建热点并接受手机连接，按该模型启动。
- [NetworkManager wait-online 文档](https://networkmanager.pages.freedesktop.org/NetworkManager/NetworkManager/NetworkManager-wait-online.service.html)：该服务等待启动阶段的网络配置完成，Wi-Fi 扫描和以太网 carrier 等待都可能增加延迟；本轮保留 NetworkManager，只移除全局上线屏障。
- [systemd timer 文档](https://www.freedesktop.org/software/systemd/man/latest/systemd.timer.html)：`OnBootSec` 与 `OnUnitActiveSec` 支持开机延后和重复维护；清空 `OnCalendar` 后使用单调时钟计划。
- [systemd journald 配置](https://www.freedesktop.org/software/systemd/man/latest/journald.conf.html)：用 drop-in 约束持久/运行时容量、保留时间及同步周期，保留排错能力。
- [systemd unit 文档](https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html)：mask 阻止启动、ConditionPathExists 按文件存在条件启动；用于废弃串口实例与本地 MFi 文件检查。
- [Armbian minimal/desktop 构建开关](https://docs.armbian.com/build-framework/switches/image-type/)：继续使用 minimal 无桌面基础镜像，不额外引入桌面服务。
