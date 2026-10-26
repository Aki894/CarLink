# 双向通话与麦克风

本轮增加 USB 麦克风采集、CarPlay 上行、HiCar 麦克风/通话音频服务，以及通话音频角色和切换保护。自动测试记录与实机结果分别记入 [validation.md](validation.md)。

## 通路与角色

```text
车机麦克风 → CarLife VR 0x58001 → Rust LiveSource → 二进制本地接口
                                                  ├─ CarPlay：PCM 重采样 → OXP microphone → iPhone
                                                  └─ HiCar：AAC 编码 → 加密 RTP → 手机

手机通话/语音音频 → 接收端 → 44.1 kHz 双声道 PCM → CarLife MEDIA / VR → 车机扬声器
```

CarPlay 保留无线会话的 Opus 输入和输出声明。桥接器将车机 16 kHz 单声道 PCM 转成 48 kHz、20 ms 数据块，由固定版本 libopus 编码为 OXP 麦克风包；PCM 协商仍有独立回退路径。编码复杂度为 3，单声道码率 32 kbps。麦克风数据不进入 JSON，也不写文件。

| 本地音频角色 | CarLife 输出 | 行为 |
| --- | --- | --- |
| 0 MEDIA | MEDIA，INIT `0x30001` / DATA `0x30006` / STOP `0x30002` | 普通音乐 |
| 1 ALERT | TTS，INIT `0x40001` / DATA `0x40003` / STOP `0x40002` | 提示音 |
| 2 VOICE | VR，INIT `0x50002` / DATA `0x50003` / STOP `0x50004` | 语音助手回应 |
| 3 TELEPHONY | VR，兼容模式可选 MEDIA | 通话下行；开始时清理其他音频队列，结束后普通媒体由后续数据重新初始化 |

车机语音模块状态使用模块 4。首次采集请求发送 `MIC_RECORD_RECOG_START (0x10024)`，停止发送 `MIC_RECORD_END (0x10023)`。麦克风、语音输出和通话各自持有语音模块状态，单个消费者结束不会提前关闭另一消费者仍使用的模块。

CarLife 音乐模块的暂停不关闭通话音频。通话角色活动时，管理器阻止另选手机或开启新协议配对；源断开后释放保护。接听、挂断等界面操作仍通过投屏输入回传给手机。

默认通话走 VR 通道，与模块 4 的语音焦点一致。Huawei 测试端会暂停被 VR 抢占焦点的 MEDIA 播放器。对需要 MEDIA 通话路径的车机，可给 CarLife 进程设置 `CARLINK_CALL_AUDIO_LANE=media`；不同物理通道的播放时钟分别保持单调。

## HiCar 音频服务

在已认证的 DMSDP 控制会话中提供：

| 服务 | 发现类型 | 音频传输类型 |
| --- | --- | --- |
| `speaker1` | 4 | 3 |
| `mic1` | 2 | 2 |
| `modemMic` | 256 | 9 |
| `modemSpeaker` | 512 | 10 |

麦克风声明 16 kHz、单声道能力。只有完成该服务的 SETUP 响应并获得有效端口后才请求车机采集。上行使用原生 FFmpeg AAC 编码、RTP 分包和会话密钥加密。停止请求按服务名隔离；旧普通麦克风的关闭事件不撤销当前通话麦克风。

下行按 SSRC 分别维护解码器，最多四个音源，并按 20 ms 节拍输出饱和相加后的 PCM。队列、解码器数量和缓冲均有上限。当前提供基础混音；导航提示音优先级及音量压低策略留待独立车机实测后校准。

## 本地媒体接口扩展

原有长度前缀和 `<BBHQ` 头不变：

- kind 5 的 flags 扩展为角色 0–3，payload 仍为 0/1。
- kind 7：JSON 采集请求，字段为 `generation`、`active`、`sample_rate=16000`、`channels=1`。
- kind 130：反向麦克风数据，flags/reserved 为 0，payload 是 LE u64 generation，随后为 PCM16。每条 PCM 为偶数字节且不超过 16384 字节。

采集代次在进程内递增并使用单调时钟种子，跨进程和协议切换时不复用从 1 起的旧代次。停止、EOF、源切换、编码错误或过期队列均停止采集并销毁编码器，旧代次 PCM 会丢弃。菜单复用器透传麦克风数据，菜单显示期间保留当前电话音频。

## 验证入口

- `receiver/test_voice.py`：PCM 分块、采样率转换、停止及代次隔离。
- `app/src/voice.rs` 与 `app/src/live.rs`：采集顺序、二进制门控、通话输出和音乐暂停。
- `scripts/test-live-bridge.py`：真实桥接进程、ARM 音频工具、CarLife TCP HU、OXP 麦克风及通话下行。
- `scripts/test-hicar-rust-live.py`：独立音源混音、加密麦克风 RTP、停止后拒绝旧数据。
- `scripts/build-audio.sh`：真实 ARMv7 编码器将麦克风 PCM 编码为 AAC，再由主机解码检查。

协议常量参考本地固定版本的 [HeadSwap](https://github.com/YegorkaPomidorka/headswap)、[Carlife_on_box](https://github.com/SadykovIV/Carlife_on_box) 和 OpenXPlay 的 OXP 接口；HiCar 服务定义与协商根据本地协议分析实现。参考程序及原始采集均保留在忽略目录。

接收端将音频格式和采集启停事件写到 tmpfs `/run/carlink-carplay/audio-diagnostics.jsonl`，限制约 64 KiB；字段仅包含角色、编码、采样率、声道和流编号。部署后需核对管理配置中的 CarPlay 接收端及 HiCar 音频工具路径，避免主程序已更新而子进程仍启动旧版本。
