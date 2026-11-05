# 开发与验证

Rust 管理器维持 CarLife USB 会话，并按选择启动 CarPlay 或 HiCar 接收端。手机媒体经统一转发层进入 CarLife，车机事件由管理器分发给当前协议。本地菜单可以接管画面而保留媒体会话。

| 目录 | 职责 |
| --- | --- |
| `app/` | CarLife、HiCar、Rust CarPlay 桥接、设备管理和输入 |
| `receiver/` | CarPlay 接收端集成、原生补丁、菜单及调试工具 |
| `drivers/`、`firmware/` | 无线驱动和系统镜像 |
| `scripts/` | 构建、镜像校验和集成回归 |
| `experiments/` | 独立诊断工具 |

使用 GitHub Actions 的 **CarLink builds** 构建 ARMv7 应用和完整固件。完整固件产物为 `carlink-h3-refreshed-image`。更新基线内核时先生成 baseline，再完成配套驱动和完整镜像。**Verify built image** 用于单独校验产物。应用接口见 [app](../app/README.md)。

验证分为源码与 CI、固件结构与校验和、台架端到端、实际车机四层。实车检查覆盖冷启动、自动重连、协议切换、画面、触屏或实体按键、媒体、导航、双向通话及挂断恢复。

## Compatibility boundaries

- The CarLife side uses USB accessory negotiation. Media and input are only available after the head unit accepts the session. CarLife authentication and transport variants are described in the [application guide](../app/README.md).
- The runtime keeps projection video compressed and forwards PCM audio. A local H.264 menu replaces the display source while visible. Returning to projection requests a fresh keyframe.
- Verify media, navigation and call routes separately. During a call, check the microphone in the opposite direction and confirm that music resumes after hang-up.
- Test each available control independently. Touch, relative focus movement, confirmation and back can arrive through different CarLife events.
- Useful integration checks include `scripts/test-live-bridge.py`, `scripts/test-manager-menu-control.py` and the HiCar framing and authentication harnesses listed in the application guide.
