# 移除无效 OpenXPlay 图标

CarPlay 主界面原有 OpenXPlay 图标由接收端 OEM 返回入口声明生成。CarLink 没有对应原车界面，点击无响应。补丁 `receiver/patches/0007-hide-inert-oem-tile.patch` 清空该构建的 OEM 图标列表；后续补丁 `0008-explicitly-hide-cached-oem-tile.patch` 显式发送 `oemIconVisible=false`，并省略 `oemIconLabel` 和 `oemIcons`。原有许可证、来源及署名资料保留。

第一版省略可见性字段后，用户实测仍有空白图标占位。对照本地 DiPlay 的 `shared/src/main/java/com/shilapi/xcertplay/airplay/AirPlayInfoPlist.kt`（第 58–59 行）：该版本明确发送 false，注释指出用于隐藏之前连接留下的 OEM 入口。因此第二版补齐关闭声明，并同步调整协议响应及 Linux 接收端的回归检查。以下首轮构建/部署记录保留供追踪；最终版本见末尾。

- 源码提交：`8638c47c4f085e2fba9d6f6b6c9406aa5ca030a2`。
- [应用构建 39817034249](https://github.com/Aki894/CarLink/actions/runs/39817034249) 成功。回归检查 `carlink_air_play_config_omits_inert_oem_tile` 通过；ARMv7 编译及音视频/反向触控桥接测试通过。
- [完整固件构建 39817788488](https://github.com/Aki894/CarLink/actions/runs/39817788488) 成功。baseline/current 镜像结构、应用、内核、模块、驱动及接收端源码/校验和检查通过，镜像已上传。
- 镜像 artifact `12443197129`，zip SHA-256：`2ed0c39e6693d46af31851edba81063d6fba9c5094752850c1c63b1374b04327`。
- 接收端 artifact `12442198380` 已于 2026-10-23 00:58（UTC+8）热部署；本地与板端均验证 61 个文件。无需重新烧录 SD 卡。
- 板端部署收据 `/etc/carlink-receiver-deploy.json`，回退目录 `/var/backups/carlink/receiver-20261022T165817Z`。
- 继续使用原有 CarLife 握手修复、30 fps、无线及音频配置。桌面图标与操作由用户在重连后确认；实车测试另行安排。

## 显式关闭入口的修正版

- 源码提交 `422ab12600da3ff1815d4f4d7479a3ec7f6412eb`，增加补丁 `0008`；与 DiPlay 相同，无图标时发送布尔值 `oemIconVisible=false`。
- [应用构建 39818918089](https://github.com/Aki894/CarLink/actions/runs/39818918089) 成功，协议层无图标用例及 Linux 配置用例均通过，接收端 artifact `12443079783`。
- [完整固件构建 39885833239](https://github.com/Aki894/CarLink/actions/runs/39885833239) 成功，完成镜像校验和上传。镜像 artifact `12472691421`，zip SHA-256：`f0389354934650bd0c82d563c5328293db0c1c037a3d29338c7d2bade513ccc5`。
- 修正版接收端已热部署到开发板，62 个清单文件校验通过，服务 active；保留原 CarLife 二进制。板端回退目录 `/var/backups/carlink/receiver-20261022T171141Z`。目录时间来自板端时钟。恢复连接期间一次 USB accessory disable 后服务自动重启恢复。
- 用户重连后确认：图标和空白占位均已消失。
