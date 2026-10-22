# 移除无效 OpenXPlay 图标

CarPlay 主界面原有 OpenXPlay 图标由接收端 OEM 返回入口声明生成。CarLink 没有对应原车界面，点击无响应。补丁 `receiver/patches/0007-hide-inert-oem-tile.patch` 清空该构建的 OEM 图标列表，使信息响应省略 `oemIconVisible`、`oemIconLabel` 和 `oemIcons`。原有许可证、来源及署名资料保留。

- 源码提交：`8638c47c4f085e2fba9d6f6b6c9406aa5ca030a2`。
- [应用构建 39817034249](https://github.com/Aki894/CarLink/actions/runs/39817034249) 成功。回归检查 `carlink_air_play_config_omits_inert_oem_tile` 通过；ARMv7 编译及音视频/反向触控桥接测试通过。
- [完整固件构建 39817788488](https://github.com/Aki894/CarLink/actions/runs/39817788488) 成功。baseline/current 镜像结构、应用、内核、模块、驱动及接收端源码/校验和检查通过，镜像已上传。
- 镜像 artifact `12443197129`，zip SHA-256：`2ed0c39e6693d46af31851edba81063d6fba9c5094752850c1c63b1374b04327`。
- 接收端 artifact `12442198380` 已于 2026-10-23 00:58（UTC+8）热部署；本地与板端均验证 61 个文件。无需重新烧录 SD 卡。
- 板端部署收据 `/etc/carlink-receiver-deploy.json`，回退目录 `/var/backups/carlink/receiver-20261022T165817Z`。
- 继续使用原有 CarLife 握手修复、30 fps、无线及音频配置。桌面图标与操作由用户在重连后确认；实车测试另行安排。
