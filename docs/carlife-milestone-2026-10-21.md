# CarLife 首个开发阶段（2026-10-21）

先开发车机侧 CarLife 手机端，再整合无线 CarPlay 接收端。主程序用 Rust，与现有应用及 CatPlay 的语言一致；Python 用于独立车机测试工具。参考仓库固定版本见 [roadmap](roadmap.md)。

应用构建：[2177647 / Actions 39582555230](https://github.com/Aki894/CarLink/actions/runs/39582555230)。

| 检查 | 结果 |
| --- | --- |
| Rust 单元测试 | Actions 中 25 项通过，覆盖协议流、状态、AOA 描述符和请求、原有诊断 |
| ARMv7 交叉编译 | 成功，产物上传并校验 SHA256 |
| H3 运行 | 已安装到 `/usr/local/bin/carlink`，help 和 doctor 正常 |
| 独立 HU 脚本 → H3 原生应用 | SSH 转发回环 TCP，握手与视频协商、错误信道、截断 EOF、未支持认证分支四项通过 |
| USB gadget | FunctionFS 描述符和字符串加载，绑定 UDC；到期与手动停止均成功解绑并清理 |
| 电脑物理 USB | UDC 仍为 `not attached`，Windows 未发现新设备；等待用户回家后继续检查 |

USB 测试进程已停止，网线 SSH 保持 `192.168.77.2`。用户表示同一连接此前可识别 Android ADB/fastboot，后续先核实实时 USB 枚举状态。

下一步：完成电脑模拟车机的 AOA 切换与 bulk 握手，随后接入测试 H.264/PCM，再做 Lexus 实车连接。
