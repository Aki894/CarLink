# CarLink

[English](README.md)

**为支持有线百度 CarLife 的车机接入无线 CarPlay 和 HiCar。**

CarLink 运行在外接 Linux 盒子上，适合支持有线 CarLife、无法安装 DiPlay 等接收应用的非安卓原厂或后装车机。车机继续使用自带的 CarLife 界面，手机通过蓝牙和 Wi-Fi 连接盒子，盒子通过 USB 接入车机。

- 支持无线 CarPlay 和 HiCar，同一时间使用一个协议。
- 转发画面、音乐、导航播报、通话声音与麦克风。
- 支持触屏及兼容的实体控制，提供设备切换和音量菜单。
- 支持已登记设备自动重连，也可手动切换协议或设备。

## 使用条件

- 车机必须支持**有线百度 CarLife**。可用按键和音频通道取决于车机实现。
- Linux 盒子需要支持 USB 设备模式、Wi-Fi 和蓝牙。当前固件构建目标为 Allwinner H3 平台，刷写前请核对[硬件要求](firmware/HARDWARE.md)。
- 手机需支持无线 CarPlay，或运行鸿蒙 4.2 并支持 HiCar。当前 HiCar 适配目标为鸿蒙 4.2。
- CarPlay 需自行提供兼容的 MFi 身份文件。HiCar 身份由盒子本地生成。

## 开始使用

1. 按[安装指南](docs/provisioning.md)获取并校验完整固件，然后刷写盒子。
2. 将盒子接入车机的 CarLife USB 口，打开车机的 CarLife。
3. 在 CarLink 菜单选择协议和「连接设备」，按手机提示完成配对。添加 HiCar 设备时，输入菜单显示的 PIN。

[文档目录](docs/README.md)包含日常操作、排障和开发说明。CarLink 是独立项目，许可和上游组件见 [LICENSE](LICENSE) 与[接收端集成说明](receiver/README.md)。
