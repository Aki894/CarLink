# CarLife 首个开发阶段（2026-10-21）

先开发车机侧 CarLife 手机端，再整合无线 CarPlay 接收端。主程序用 Rust，与现有应用及 CatPlay 的语言一致；Python 用于独立车机测试工具。参考仓库固定版本见 [roadmap](roadmap.md)。

应用构建：[9a7aff4 / Actions 39610764455](https://github.com/Aki894/CarLink/actions/runs/39610764455)，ARMv7 应用产物上传成功。

| 检查 | 结果 |
| --- | --- |
| Rust 单元测试 | Actions 通过，覆盖协议流、状态、AOA / WinUSB 描述符、异步接收 ABI / FIFO / 队列边界、原有诊断 |
| ARMv7 交叉编译 | 成功，产物上传并校验 SHA256 |
| H3 运行 | 已安装到 `/usr/local/bin/carlink`，help 和 doctor 正常 |
| 独立 HU 脚本 → H3 原生应用 | SSH 转发回环 TCP，握手与视频协商、错误信道、截断 EOF、未支持认证分支四项通过 |
| USB gadget | FunctionFS 描述符和字符串加载，绑定 UDC；到期与手动停止均成功解绑并清理 |
| 电脑物理 USB | 后置 USB-A → 板子 Type-C，AOA 1 从 `18d1:4ee7` 切换至 `18d1:2d00`，Windows 自动使用 WinUSB |
| 分段 bulk 收发 | 修复 MUSB 同步读重投间隙丢包，预投 32 个 512 字节异步读；原有 1/4/6/17 字节分段测试通过 |
| 连续重新连接 | 更新主机枚举等待后，连续三轮重新启动 gadget，完整 USB 测试全部通过；每轮停止后 `aio-nr=0`，自建 gadget 清理成功 |
| 单路供电 | 用户断开 CH340 的 5V，保留 GND/TX/RX，使用 Type-C 供电及数据；全过程启动 ID 未变，有线及 AX900 网络正常，无失败服务 |

USB 测试进程已停止，网线 SSH 保持 `192.168.77.2`。新应用 SHA256：`bca227a9a1788562722472b0fe11550db9ec37550105318accbf848f0714de3d`，下载产物与板上安装文件一致。该轮仅修改应用和测试脚本，系统镜像未改动。

当前已验证电脑模拟车机与实板 USB 的协议交互。下一步接入测试 H.264/PCM，再进行 Lexus 实车连接、画面、声音及控制验证。
