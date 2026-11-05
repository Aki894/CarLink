# 安装与初始化

1. 核对[硬件要求](../firmware/HARDWARE.md)，确认镜像适合你的设备，并备份目标存储介质。
2. 从 [CarLink builds](https://github.com/Aki894/CarLink/actions/workflows/build.yml) 的成功运行下载 `carlink-h3-refreshed-image`。自行触发时选择 `firmware=true`、`kernel_baseline=false`。仅有应用包或 baseline 镜像时，尚未得到完整固件。
3. 解开 artifact 外层 ZIP，用 `sha256sum -c SHA256SUMS` 验证压缩镜像。使用支持 `.img.xz` 的写盘工具写入目标介质，或先解压为 `.img`。核对写入目标，写盘会覆盖该介质。
4. 启动盒子并保留有线管理连接。检查 `systemctl status carlink` 和 `carlink doctor --json`，再连接车机的 CarLife 数据口。

## 本地身份和配置

- CarPlay 将合法取得且兼容的 `identity.pk8` 和 `certificate.p7b` 放在 `/var/lib/carlink/carplay/mfi/`。目录权限为 0700，文件为 0600。参见[接收端说明](../receiver/README.md)。
- HiCar 首次使用会生成本地身份及固定 PIN，也可通过 `carlink hicar identity-init` 初始化。升级时保留身份及配对状态。
- 管理器配置位于 `/etc/carlink/manager.json`，参见[配置示例](examples/manager.json)。使用 `carlink wifi-interface` 检查自动选中的无线接口。
- 更新时使用同一次构建的管理器、协议子进程及配套库，保留旧版和本地状态以便回滚。身份、凭据和配对记录应留在设备本地。
