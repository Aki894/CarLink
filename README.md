# CarLink

悟空派 Hi-Zero / H3 上的无屏 CarPlay → 有线 CarLife 桥接项目。

当前阶段：Linux 硬件启动与诊断。**尚未实现完整 Linux CarPlay/CarLife 桥接，镜像构建成功也不代表实车连接成功。**

## 系统基线

- Armbian framework 固定于 `sources.lock.json`，`BOARD=orangepizero`。
- Debian 13 Trixie Minimal，ARMv7 hard-float，current 6.18 内核系列。
- Orange Pi Zero 在 Armbian 中属于社区支持，当前没有指定维护者，并非标准支持等级。
- 实际 USB 网卡为 AX900/AIC8800D80 系列，`a69c:5721` 存储模式切换后为 `a69c:8d80`；驱动与固件按芯片 revision 匹配，不使用 RTL8821CU 驱动代替。
- 独立 USB-A 接 AX900；Type-C 保持 USB Device，供后续 AOA 使用；默认不加载 `g_serial` 占用 UDC。
- AIC 的 Wi-Fi、蓝牙和热点实测通过后再禁用板载 XR819；开发期间保留板载无线及网线管理入口。
- 板卡启动差异及厂商 DRAM 参数见 [硬件说明](firmware/HARDWARE.md)。

## 构建

GitHub Actions 的 `CarLink builds`：普通代码推送运行测试并交叉编译原生 `carlink`；手动 Run workflow 并勾选 `firmware`，再构建内核、原生驱动和完整 SD 卡镜像。应用单独更新，无需每次重编系统。

产物：`carlink-armv7`、`carlink-h3-image`、构建日志与内核 deb 包。每个镜像带 SHA256 和项目/Armbian提交记录。框架固定不等于 apt 软件包及内核源码逐字节可复现；具体版本以当次构建证据为准。

## 电脑网线直连

镜像预设板端 `end0 = 192.168.77.2/24`，没有默认网关。电脑对应以太网口设为 `192.168.77.1/24`（仅在该网段未被其他网络占用时），保留电脑原有 Wi-Fi 上网。首版镜像误绑定 `eth0`，可在现有板上通过 `nmcli connection modify carlink-direct connection.interface-name end0` 后激活此连接修正，无需重新烧录。无需占用 AX900 建管理网络。

首次刷卡后等待 Armbian 初始化，通过 `ssh root@192.168.77.2` 登录。标准 Armbian 首次密码 `1234`，按首次登录流程改密码、建立普通用户；不跳过密码更改，也不内置个人SSH密钥。上板时实际确认登录行为与镜像版本。

```sh
carlink doctor --json
uname -a
lsusb -t
iw list
bluetoothctl list
sudo journalctl -b -k
```

`carlink doctor` 只采集硬件信息，AOA结果始终为 `not_tested`。UDC存在不能证明AOA/CarLife可用。日志保存上限16MiB，周期刷盘仍可能丢失掉电前尾部。

刷卡前先核镜像校验和、目标SD卡及旧卡备份。当前不写SPI Flash。首轮验证SSH、DRAM稳定性、AX900 Wi-Fi/蓝牙、UDC；然后用固定视频独立验证Lexus CarLife，再接入CarPlay。

详细路线：[阶段计划](docs/roadmap.md)。厂商资料、参考仓库、镜像和认证身份不提交到本仓库。
