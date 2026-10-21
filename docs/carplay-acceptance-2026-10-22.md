# CarPlay 接收端验收记录（2026-10-22）

本轮使用悟空派 H3、AX900/AIC8800 USB 无线模块和 iPhone 16 Pro（iOS 27.2），通过有线 SSH 管理。车机暂不可用，当前验证范围为无线 CarPlay 接收端及反向控制。

## 实现和来源

- CarLink 主程序保持独立；OpenXPlay Relay 和 Stream 作为独立进程运行。
- Relay：`6a7cdab585113e29422f2306e90fdb5a789c318a`。
- Stream：`356a3f56c0e0b4e839b91387bf2b8cfa0b0a8537`。
- ARMv7 应用和接收端通过 GitHub Actions 编译；产物包含许可证、上游源码归档、补丁、来源记录和 SHA256SUMS。
- Powered by OpenXPlay. [上游项目](https://github.com/OpenXPlay/OpenXPlay-Relay)。Relay 使用 AGPL-3.0-only；协议库使用 GPL-3.0-only。
- MFi 身份只在本机提取并检查密钥匹配，部署在开发板的私有目录；配对数据和屏幕录制也只保留在本地。

## 已通过的硬件检查

| 检查 | 结果 |
| --- | --- |
| 蓝牙配对和 MFi 验证 | iPhone 实际连接成功；无线隧道再次认证成功 |
| 5 GHz 热点 | CN 地区、149 信道（5745 MHz），实际连接成功 |
| 画面接收 | 800×480 H.264 High Level 3.1，连续接收超过 13,000 帧 |
| 视频解码检查 | 接收数据转换为 Annex B 后，电脑上的 FFmpeg 成功解码 |
| 关键帧反向请求 | 发出请求后，接收端观察到新增关键帧 |
| 触控反向发送 | 点击播放控件，实际画面从暂停变为播放，再恢复 |
| 断开重连 | 重启接收进程后，iPhone 自动恢复连接 |
| 音频输出与媒体接收 | 用户确认 iPhone 输出选项出现并播放；板子收到 48 kHz 双声道 AAC 媒体包 |
| 软件重启恢复 | 蓝牙驱动修复后，开发板软件重启成功并恢复连接 |
| 资源采样 | 接收进程约 7.1 MiB RSS；一次连续运行的平均 CPU 约 6.6% |

探针排空视频和音频并记录计数，默认不录制。录制需显式启用，所有已有片段合计上限为 128 MiB。

## 蓝牙驱动修复

初次配对触发 `aic_zlp_quirk` 在 ARM 的 `alloc_bulk_urb` 返回探针内故障，阻塞 HCI 发送和软件重启。补丁在 ARM 使用驱动已有的 `usb_submit_urb` 路径，并维持设备、接口和 ACL 端点筛选。

[驱动构建 39670639717](https://github.com/Aki894/CarLink/actions/runs/39670639717) 成功。板上确认 `hook=usb_submit_urb`，实际蓝牙配对、无线视频接收和软件重启均已通过。

## 音频定位

iPhone 起初仅发送 130（iAP2）和 110（视频）流，没有建立音频流，音频输出列表也没有接收端。

通过 USB 信任连接读取 iPhone 日志后，定位到：

```text
Error in creating formats for type 100 / telephony
APCarPlayAudioFormatsCreate: ... kParamErr
```

收窄通话 PCM 采样率后，iPhone 仍返回相同错误。随后开启 Opus 语音声明，音频格式初始化成功，iPhone 创建主音频、缓冲音频和提示音通道。触控启动播放后，板子收到 type 102 媒体流；初次采样为 307 个音频包、179,765 字节，格式为 AAC-LC、48 kHz、双声道。用户确认音频输出列表出现接收端并播放。

探针模式默认开启 `CARLINK_CARPLAY_VOICE_OPUS=1` 和 `CARLINK_CARPLAY_BUFFERED_AUDIO=1`，可分别设为 `0` 进行对照。当前探针排空压缩音频；音频解码、外放和 CarLife 转发列入后续桥接工作。

## 完整镜像检查

[完整增量镜像构建 39674743077](https://github.com/Aki894/CarLink/actions/runs/39674743077) 的 firmware job 成功，完成镜像内容校验、XZ 检查、校验文件生成和产物上传。

该镜像包含 `b3e1c18cfcbcc4f5fa8925f7417022cf67c0b73b` 的应用和接收端，以及上述蓝牙驱动补丁。原始启动程序和内核采用已在板上运行的基线镜像，内核版本为 `6.18.57-current-sunxi`。

压缩镜像 SHA256：

```text
2b0948582b43afad49cac6feb2705ace23c3d0d0f39d6763786ddfe7dfce296d
```

此镜像已下载并核对 SHA256。开发板采用单独部署的应用和驱动进行本轮测试；完整镜像尚未重新烧录。

## 后续工作

音视频接收已通过，接下来连接接收端与 CarLife 发送端：视频实时队列、音频解码及采样率转换、触控映射，再处理麦克风回传和电话场景。Lexus 原车验收待实车条件恢复。
