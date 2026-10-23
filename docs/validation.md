# 验证与部署记录

状态截至 2026-10-23。这里汇总已完成的检查，日常运行见[运行说明](live-forwarding.md)。

## 当前基线

| 项目 | 版本 / 结果 |
| --- | --- |
| CarPlay 接收端源码 | `422ab12600da3ff1815d4f4d7479a3ec7f6412eb` |
| CarLife 握手实现 | `2785fb3`；板上二进制 SHA256 `efdceea8f6f18ed2571b05e1527dda59a1524526be71cdc0b9cb8c27140a9289` |
| 应用与接收端 CI | [39818918089](https://github.com/Aki894/CarLink/actions/runs/39818918089)，测试、ARMv7 编译与上传成功 |
| 完整固件 CI | [39885833239](https://github.com/Aki894/CarLink/actions/runs/39885833239)，镜像内容检查与上传成功 |
| 镜像 artifact | `carlink-h3-refreshed-image`，ID `12472691421`，ZIP 420,740,370 字节 |
| 镜像 ZIP SHA256 | `f0389354934650bd0c82d563c5328293db0c1c037a3d29338c7d2bade513ccc5`；烧录前另校验包内镜像清单 |
| 接收端 artifact | `12443079783`，本地/板端 62 个文件校验通过，已热部署 |
| 板端收据 | `/etc/carlink-receiver-deploy.json` |
| 板端接收端回退 | `/var/backups/carlink/receiver-20261022T171141Z`；目录名时间取自当时板端时钟 |

镜像以 [39459953269](https://github.com/Aki894/CarLink/actions/runs/39459953269) 为基线，保留启动程序和 `6.18.57-current-sunxi`，集成 [39670639717](https://github.com/Aki894/CarLink/actions/runs/39670639717) 的匹配 AIC 驱动。当前开发板通过热部署更新接收端，最新完整镜像尚未重新烧录。

## 已完成的检查

| 层次 | 结果 |
| --- | --- |
| 源码与合成测试 | Rust 协议/状态/输入测试、Python 帧协议、ARM 音频重采样及真实转发进程的合成闭环通过 |
| 构建与镜像 | ARMv7 ELF/ABI、启动配置、镜像结构、应用/内核/模块/驱动、源码/许可和校验清单通过 |
| 无线硬件 | AIC 5 GHz 客户端、HTTPS、蓝牙扫描、iPhone 热点关联和重启恢复通过；XR819 已禁用 |
| USB | AOA 51/52/53、`18d1:4ee7 → 18d1:2d00`、bulk 拆分传输和连续三轮枚举通过 |
| 独立 CarLife 媒体 | 华为 OTG 测试端显示彩条、播放间歇提示音；1280×720@30 合成视频与 PCM 通过 |
| 独立 CarPlay 接收 | iPhone 配对、MFi、800×480 H.264、48 kHz 双声道 AAC、反向触控及重连通过 |
| 实时桥接 | iPhone → H3 → 华为 CarLife 的画面、连续音乐、触屏返回通过 |
| 30 fps 对照 | 用户确认播放/暂停音乐时滑动和点击都流畅，声音连续 |
| 过渡动画 | 录制源视频中复现约三秒停顿；完整重启 iPhone 后恢复，减弱动态效果可关闭 |
| OEM 入口移除 | 省略字段曾留下空图标；参考 DiPlay 显式 `oemIconVisible=false` 后，用户确认图标和占位均消失 |
| 启动 | 一次软件重启由 100.878 s 降至 22.872 s，CarPlay ACTIVE 28.861 s；详细计时见启动文档 |
| Lexus 实车 | 首轮 80% / 重试 0% 失败；握手修复后的实车复验尚未完成 |

## 关键历史里程碑

| 日期 | 内容 | 来源 |
| --- | --- | --- |
| 10-21 | 内存 MemTotal 501244 KiB；有线接口修正为 end0，直连 SSH 可用 | 基线 [39459953269](https://github.com/Aki894/CarLink/actions/runs/39459953269)、独立校验 [39546347234](https://github.com/Aki894/CarLink/actions/runs/39546347234) |
| 10-21 | AIC `main` profile Wi-Fi/蓝牙/AP 通过，板载 XR819 禁用 | [39549290693](https://github.com/Aki894/CarLink/actions/runs/39549290693)；芯片日志 `chip_id=7, chip_mcu_id=1` |
| 10-21 | 原生 CarLife USB 与 WinUSB 台架通过；MUSB RX 改为 32 个 512 字节 AIO 预投 | [39610764455](https://github.com/Aki894/CarLink/actions/runs/39610764455) |
| 10-21 | 固定 H.264 / PCM 在华为车机应用实际播放 | [39628785953](https://github.com/Aki894/CarLink/actions/runs/39628785953) |
| 10-22 | 修复 ARM 蓝牙 quirk 返回探针，改用已有 usb_submit_urb 路径；配对及软件重启通过 | [39670639717](https://github.com/Aki894/CarLink/actions/runs/39670639717) |
| 10-22 | 开启 Opus 语音能力后 iPhone 音频格式初始化恢复；接收 AAC 媒体流 | [39678214827](https://github.com/Aki894/CarLink/actions/runs/39678214827)、[完整固件 39678818571](https://github.com/Aki894/CarLink/actions/runs/39678818571) |
| 10-22 | 实时桥接、启动精简；修复触控扩展字段、批量 USB 外层帧、AudioTrack 反复重建 | [39728191942](https://github.com/Aki894/CarLink/actions/runs/39728191942)、[完整固件 39728628096](https://github.com/Aki894/CarLink/actions/runs/39728628096)；用户确认可点击且正常出声 |
| 10-22 | Lexus 触控板输入及完整键脉冲合成测试 | [39729533171](https://github.com/Aki894/CarLink/actions/runs/39729533171)、[完整固件 39730049076](https://github.com/Aki894/CarLink/actions/runs/39730049076) |
| 10-22 | 持久日志修复与受控重启取证 | [完整固件 39801437061](https://github.com/Aki894/CarLink/actions/runs/39801437061) |
| 10-22 | `MD_PEER_ACCEPT` 与挑战认证解耦，原生 ARM 和华为 4.1 握手通过 | [39805684763](https://github.com/Aki894/CarLink/actions/runs/39805684763)、[完整固件 39807108167](https://github.com/Aki894/CarLink/actions/runs/39807108167) |
| 10-23 | 部署初始 30 fps 声明及实际 FPS/Mbps 统计，用户操作与声音对照通过 | 接收端 artifact `12436739878`，来源 `2785fb3` |

## 后续验证

1. Lexus 点火到首次连接、失败后重试、完整持久日志和原始触控板输入。
2. 至少三次冷启动、长时间音乐/导航、退出重连和热点/蓝牙共存。
3. 电话、Siri、麦克风回传及提示音/音乐混音。
4. 内存压力、SD 稳定性和独立供电条件。

本地证据集中在 `.build/evidence/`、`.build/car-investigation/`、`.build/apk-analysis/`、`.build/transition/`；私有身份和配对留在 `.build/private/`，不进入公开仓库。
