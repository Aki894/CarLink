# CarLink

悟空派 H3 上的无屏无线 CarPlay → 有线 CarLife 桥接。采用 Rust CarLife 手机端、OpenXPlay 接收端和 Python 媒体桥接；视频 H.264 透传，音频解码为车机所需 PCM。

## 当前状态

截至 2026-10-23，iPhone → 5 GHz Wi-Fi → H3 → USB → 华为 CarLife 测试端的画面、连续音乐、触屏返回和自动重连已通过桌面验证。源头限制为 800×480、最高 30 fps；OpenXPlay 无效图标及空白占位已移除，用户已确认。

Lexus ES200 2020 卓越版的首次实车测试停在 80% / 重试 0%。后续已经修复接受车机通知的顺序并通过台架回归，下一项是实车复验及 Remote Touch 触控板验证。电话、Siri 麦克风回传和长时间稳定性单独安排。

## 系统与接线

- 悟空派 H3 / Orange Pi Zero H3、512 MiB；Armbian Debian 13 Trixie Minimal、ARMv7 hard-float，实测内核 `6.18.57-current-sunxi`。
- USB-A 接 AX900/AIC8800D80 无线模块；中国大陆、5 GHz 149 信道。Wi-Fi、蓝牙、热点实测通过后已禁用板载 XR819。
- Type-C 为 USB Device，连接车机或测试手机 OTG；CH340 的 5V 已断开，串口可保留 GND/TX/RX。
- 电脑直连以太网为 `192.168.77.1/24`，板端 `end0=192.168.77.2/24`，不设置默认网关。始终保留有线 SSH 管理。

## 构建与运行

本地修改源码；Linux、驱动和 ARMv7 应用通过 GitHub Actions 编译。

| 工作流 | 用途 |
| --- | --- |
| `CarLink builds` | 推送源码后测试并编译应用/接收端；手动勾选 firmware 可从源码构建整机镜像 |
| `AIC USB driver for installed kernel` | 根据匹配内核 headers 构建无线驱动 |
| `Refresh existing firmware` | 在已验证的基线镜像上集成成功的应用及驱动产物 |

当前完整固件、摘要和部署记录见[验证记录](docs/validation.md)。应用可单独更新，日常迭代无需重新烧录 SD 卡。涉及固件或构建流程的改动，交付前完成完整 firmware job、镜像校验及产物上传。

```sh
ssh root@192.168.77.2
systemctl status carlink
journalctl -u carlink -b --no-pager
carlink doctor --json
```

公开镜像不含 MFi 身份与配对数据；恢复自己的本地身份文件后，`carlink.service` 才满足自动启动条件。认证、厂商资料、参考仓库和原始媒体始终留在本地。

完整入口：[文档目录](docs/README.md) · [运行与协议](docs/live-forwarding.md) · [硬件](firmware/HARDWARE.md) · [下一步](docs/roadmap.md)。
