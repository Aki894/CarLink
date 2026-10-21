# 悟空派 H3 无线硬件验证（2026-10-21）

环境：Debian 13 / Armbian，ARMv7，`6.18.57-current-sunxi`。

AX900 实际为 AIC8800D80：存储模式 `a69c:5721`，加载阶段 `a69c:8d80`，运行阶段 `368b:8d81`。加载日志为 `chip_id=7, chip_mcu_id=1`；本板实测使用固定提交的 `main` 驱动和固件，构建记录见 [Actions](https://github.com/Aki894/CarLink/actions/runs/39549290693)。

| 项目 | 实测结果 |
| --- | --- |
| 5 GHz 客户端 | 连接路由器，指定 AIC 接口 ping 网关无丢包，HTTPS 请求返回 200 |
| 蓝牙 | 标准 `btusb` 配合 `aic_zlp_quirk`，`hci0` 启用，LE 扫描收到周围设备 |
| 5 GHz 热点 | CN 区域、149 信道、5745 MHz；用户 iPhone 成功关联、取得 DHCP 地址并收发数据 |
| 板载 XR819 | 关闭原连接自动连接，卸载 `xradio_wlan`，安装持久 blacklist；重启后未加载 |
| 重启恢复 | 网线静态地址、AIC 自动联网、CN 参数、蓝牙控制器恢复，无失败 systemd 单元 |

现场测试热点已关闭，开发板通过 AIC 连接现有路由器。电脑直连入口保持 `192.168.77.2`。

当前配置文件：

- `/etc/modprobe.d/carlink-aic.conf`：`options aic8800_fdrv country_code=CN custregd=0`。
- `/etc/modprobe.d/carlink-onboard-wifi.conf`：`blacklist xradio_wlan`。

需要恢复板载 Wi-Fi 时，通过网线 SSH 删除第二个文件，再执行 `modprobe xradio_wlan`，重新配置其网络连接。

完整镜像使用 `Refresh existing firmware`，提供已成功的基线构建和驱动构建编号，并选择 `disable_onboard_wifi=true`。不含驱动的基线镜像默认保留 XR819。

下一阶段是 USB AOA / Lexus CarLife 连接与协议验证。
