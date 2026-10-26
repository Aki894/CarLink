# 文档目录

文档按主题维护当前说明，关键历史结果集中到验证记录。旧的逐轮过程文档已合并，完整原文可从 Git 历史查看。

| 文档 | 内容 |
| --- | --- |
| [验证与版本](validation.md) | 最新构建、部署、桌面实测、历史里程碑及未完成项目 |
| [完成情况核对](status-audit.md) | R2/R3 当前范围、尚需实现的功能与待验收项目 |
| [双向通话与麦克风](calls.md) | 音频角色、USB 采集、CarPlay/HiCar 上行和验证入口 |
| [运行与媒体协议](live-forwarding.md) | 启停、配置、本地接口、音视频和生命周期 |
| [CarLife 与实车兼容性](carlife-compatibility.md) | 80% / 0% 失败、持久日志、握手修复、下一轮取证 |
| [性能与源码参考](performance.md) | 30 fps、排队和音频修复、同类源码、动画故障对照 |
| [启动优化](boot-optimization.md) | 已实施配置、22.872 秒软件重启实测、回滚方法和依据 |
| [Lexus Remote Touch](lexus-remote-touch.md) | 触控板、按键映射与实车待验项 |
| [后续路线](roadmap.md) | 当前优先级、验收方式和参考版本 |
| [板卡与接线](../firmware/HARDWARE.md) | 厂商资料、DRAM、USB 角色和供电 |
| [CarLife 应用](../app/README.md) | CLI、合成媒体、独立 USB/TCP 测试和诊断 JSON |
| [无线 HiCar 计划](hicar.md) | 阶段验收、认证技术路线与隔离探针 |
| [双协议连接与设备菜单](connection-management.md) | 多手机优先级、配对与切换、Remote Touch 菜单、单手机台架计划 |
| [HiCar Rust 迁移](hicar-rust.md) | 协议对照、HiChain FFI、原生媒体、统一程序与分批验收 |
| [CarPlay 接收端](../receiver/README.md) | OpenXPlay、探针、录制和许可证 |
| [AIC 驱动](../drivers/aic8800d80/README.md) | 精确内核构建、驱动 profile 与安装 |

本地 `.build/README.local.md` 说明调试工具、私有资料、证据和保留的当前产物。`research/repos/` 是本地参考源码，不随 CarLink 发布。
