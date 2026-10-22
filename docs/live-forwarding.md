# 实时转发：实现范围与运行方式

## 数据路径

`carlink-run` 并行启动 CarLife USB 发送端和 OpenXPlay 无线 CarPlay 接收端。接收端通过 OXP Unix socket 将媒体交给 `receiver/forward.py`，再通过独立的本地媒体协议送给 Rust CarLife 会话。服务由 systemd 负责自动重启。

- 视频：固定 800×480、30 fps 的接收能力；H.264 AVC 转为 Annex B，关键帧补齐 SPS/PPS，High Profile 数据透传，不执行软件视频重编码。
- 媒体和提示音：AAC-LC、PCM S16LE、Opus 分别封装为所需输入格式，由精简 FFmpeg 解码并转换为 44.1 kHz、双声道 PCM16LE。MEDIA 使用 CarLife Media 通道，ALERT 使用 TTS 通道，二者维持独立初始化状态。
- 输入：CarLife 单点触控坐标按协商画面归一化，再通过 OXP 反向发送；暂停、重置和过载会请求新关键帧。
- 生命周期：断线重连重新声明源配置，丢弃旧媒体代际；等待关键帧期间不发送依赖旧帧的 P 帧。显式音频结束消息处理暂停播放而视频保持连接的情况。

当前电话、Siri 语音和麦克风回传还需要专门的 CarLife 协商；本次实现覆盖画面、媒体、提示音和单点触控。

## 边界和依赖

Rust 监听 `/run/carlink/live.sock`，权限 0600，父目录由启动服务以 0700 创建。帧包含 4 字节小端长度和 12 字节 `<BBHQ` 头：kind、flags、reserved=0、pts_us。视频单帧最大 512 KiB，PCM 最大 64 KiB；两端媒体队列有 1 MiB 上限。Python 音频输入另有 16 包上限，PCM 待发送队列最多 10 包，过期媒体不无限追赶。

| kind | 方向 | 内容 |
| --- | --- | --- |
| 1 | 接收端→CarLife | 3 个 LE u32：width、height、fps |
| 2 | 接收端→CarLife | Annex B access unit；flags bit 0 表示关键帧 |
| 3 | 接收端→CarLife | 44100 Hz 双声道 PCM16LE；flags 0=MEDIA、1=ALERT |
| 4 | 接收端→CarLife | 无 payload；flags bit 0 表示会话 ACTIVE |
| 5 | 接收端→CarLife | payload 单字节 0/1 表示音频停/启，flags 为角色 |
| 129 | CarLife→接收端 | UTF-8 JSON：keyframe 或 normalized touch |

音频工具固定 [FFmpeg 8.1.3](https://www.ffmpeg.org/download.html)，源码提交 `1041abdc962f4cc4f394aa8de9dc5236c0c3b9e7`。仅构建 AAC/PCM/Opus 音频解码、PCM 编码、pipe、AAC/Ogg/PCM 格式和重采样，不安装桌面、视频解码、网络输入和外部编解码依赖。产物包含 LGPL 许可、精确源码归档和构建配置。[FFmpeg 重采样文档](https://ffmpeg.org/ffmpeg-resampler.html)、[Ogg Opus RFC 7845](https://www.rfc-editor.org/rfc/rfc7845.html) 和 [Opus RFC 6716](https://www.rfc-editor.org/rfc/rfc6716.html) 是实现依据；视频协议继续使用已固定的 OpenXPlay。

## 启动和恢复

镜像中的 `carlink.service` 使用 `carlink-run`，不等待外网。它在本地 MFi 两个文件齐备时自动启动；公开镜像不含认证或配对数据。新刷镜像后需恢复自己的本地 MFi 文件。开发板已有的身份和配对数据在本轮代码工作中未更改。

```sh
# 正常使用：同时启动两个端点
systemctl start carlink.service

# 独立诊断接收端，运行前停止综合服务，避免争用 USB/蓝牙/热点
systemctl stop carlink.service
carlink-carplay probe
```

`/etc/default/carlink` 可设置已有的 `CARLINK_CARPLAY_WIFI`、国家/信道及安装路径变量。默认 CN、149 信道，保留有线管理。

## 验证安排

云端执行 Rust 单测、Python 帧协议测试、ARM 交叉编译、QEMU 音频重采样，以及合成 OXP→实际 Python 转发进程→CarLife TCP 的闭环检查，并验证反向触控。完整镜像校验还检查音频二进制、源码、启动配置和校验和。

按用户要求，本轮不部署新转发版本、不重启板子、不进行手机/车机联调。下一次上板时依次验证真实 iPhone→安卓车机测试端的画面/音频/触控，实际冷启动计时，最后再做 Lexus 原车测试。

首次云端验证：[39722488522](https://github.com/Aki894/CarLink/actions/runs/39722488522) 已通过合成闭环、ARM 编译和 QEMU 音频重采样。音频工具 ARM ELF 体积 1,046,184 字节（约 1 MiB）。完整源码归档作为分发合规资料随产物保留。后续修正由最终构建记录覆盖。

最终构建、完整镜像及校验结果见 [2026-10-22 验收记录](live-build-validation-2026-10-22.md)。
