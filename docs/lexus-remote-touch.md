# 2020 Lexus ES Remote Touch 输入

2020 款 ES200 的触控板是主要操作入口。手机触屏只用于开发测试。本实现参考本机 CarProjection 的 `CarPlayPadInput`、`TouchPadTapTracker`、CarLife 触控板 protobuf 和 Linux 输入回归用例，接入 CarPlay 原生焦点导航，不依赖 Android 鼠标或屏幕光标。

## 消息与映射

| CarLife 输入 | 处理 | CarPlay 输出 |
| --- | --- | --- |
| `0x1005A` 按下 | 开始一段手势，使用本机单调时间计时 | 等待移动或抬起 |
| `0x1005B` 移动 | 字段 2/3 按有符号 int32 解释 deltaX/deltaY；6 单位移动阈值；48 单位一步；一次最多四步 | 主轴左/右/上/下焦点键 |
| `0x1005C` 抬起 | 500 ms 内且未移动为轻触；450 ms 内第二次轻触确认为双击 | 双击发送确认 |
| `0x1005E` 焦点变化 | 清除触控板手势状态 | 后续手势重新开始 |
| `0x68008`，键码 `0x14` | 机械按压确认；双击后的 150 ms 内抑制重复确认 | Select |
| 键码 `0x01 / 0x0E` | 主页 / 返回 | Home / Back |
| 键码 `0x06 / 0x07` | 后一个 / 前一个选择项 | KnobRight / KnobLeft |
| 键码 `0x10 / 0x0F` | 下一首 / 上一首 | Next / Previous |

协议兼容点：`0x1005A–0x1005E` 虽属于 CMD 编号范围，Lexus 参考实现将它们放入 Touch 通道。解码器仅对这五个明确编号允许该组合，其余编号继续执行原有通道检查；也处理将它们放入 CMD 通道的对端。

每个焦点键通过本地媒体反馈协议发送一个 `key-press` 事件，Python 转为 OXP Kind 17 的按下/抬起完整脉冲。接收端声明支持 0–9 号键，现有 OpenXPlay 转为 CarPlay HID。暂停/非活动投屏阶段清除手势状态并忽略输入；断开后不保留待释放的按键。

## 当前边界与验证

这一阶段提供焦点导航和确认/返回/主页/选曲。单次轻触不自动确认，避免抬指时误触；滑动仍按方向切换焦点。捏合与双击拖动的连续滚轮尚未映射，当前 OXP 接口仅暴露方向焦点键。

Rust 检查覆盖：负数位移、无按下时移动、步数上限、异常位移、双击和机械确认去重、特殊通道消息往返与相邻编号拒绝。Python 检查完整键脉冲和无效键值。合成端到端用例发送 Lexus 按下和负位移消息，验证其经过实际 Rust 会话及 Python 转发后成为 OXP 左键的按下/抬起。

真实 iPhone 与安卓车机端的画面、声音、触屏已经通过。触控板消息先用合成对端验证；Lexus 实车后需要确认原始消息、方向、步进阈值和按压手感。

2026-10-22 构建和部署记录：

- 应用来源 `945ba844b16b7da57fb1e9079eaaf69b9ada68d7`；[应用 job 39729533171](https://github.com/Aki894/CarLink/actions/runs/39729533171) 成功，包括新增触控板合成闭环。
- [完整 firmware job 39730049076](https://github.com/Aki894/CarLink/actions/runs/39730049076) 成功。日志确认 13 项启动配置核验、`mode=current` 全镜像内容与源码/校验和核验、压缩校验以及镜像上传全部完成。
- 镜像 artifact ID `12404372896`，ZIP SHA256 `57da2977ced6b47398464dd1b0a5241c146f699beef8acae5b5f4acefe1165ac`。
- 已远程安装到板子，原版本备份 `/var/backups/carlink/touch-20261022T054846Z`。启动后 iPhone 与安卓车机端恢复连接；音频回归成功创建并启动 AudioTrack，数分钟内服务 `NRestarts=0`。
