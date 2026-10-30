# 正式固件与首次初始化

## 构建入口

日常使用 GitHub Actions 的 **CarLink builds**，选择 `firmware=true`、`kernel_baseline=false`。该入口在应用测试、交叉编译和上传完成后，依次构建原生 HiCar 认证库、执行 ARM 互通测试，并组装完整双协议镜像。

正式产物为 `carlink-h3-refreshed-image`，必须同时具备 Rust manager 及示例配置、HiCar 原生认证依赖、匹配内核的 AIC 模块/固件，以及菜单与媒体运行包。镜像内记录 AIC 来源和文件摘要；检查缺项、摘要、ABI、vermagic 或来源不一致即失败。

同一次 workflow 的固件任务可以读取已成功结束的 `app` job 产物，但须匹配本次 run、提交、仓库及两个已上传 artifact 的 digest。引用其他 run 时，仍要求该 run 整体成功。

需要重编内核时才选择 `firmware=true`、`kernel_baseline=true`，输出明确标记的 `carlink-h3-baseline-image`。更换内核后先构建匹配的 AIC 驱动，再通过 **CarLink complete firmware** 指定对应 `base_run_id`、`base_artifact_name`、`driver_run_id` 与成功应用 run。baseline 不是完整双协议发布镜像。

独立 **Verify built image** 默认复验正式 refreshed 产物；验证 baseline 须显式选择其 artifact。

## HiCar 本机身份

首次切换到 HiCar 时，manager 在显示配对码、交接旧会话之前初始化缺失身份；直接运行 HiCar 后端也执行相同检查。身份在本机生成随机设备种子和六位固定 PIN，文件以 0600 原子发布，新建私有目录为 0700。

也可在设备上显式初始化：

```sh
carlink hicar identity-init
# 自定义存储位置；名称仅在新建时使用
carlink hicar identity-init --identity /var/lib/carlink/hicar/production/identity.json --name CarLink
```

命令只报告是否创建成功，不输出 PIN、种子或密钥。并发调用只会有一个创建者。已有身份、固定 PIN、名称及 HUKS 数据原样保留；损坏、权限过宽或符号链接身份文件会报错，等待维护修复，不自动轮换身份。

CarPlay MFi 身份继续按本机配置提供，不进入仓库、构建产物或固件；它的检查限于 CarPlay 后端，菜单与 HiCar 可独立启动。

## 外置无线网卡

新示例的 HiCar `--network-interface` 与 CarPlay 默认网卡均使用 `auto`，通过同一 Rust 识别器选择已验证的 AIC 外置无线设备，兼容 USB 父设备和接口驱动两种 sysfs 布局。不会选择有线管理接口或板载 XR819。

```sh
carlink wifi-interface
```

网卡尚未就绪时报告错误，由服务重试；有两块候选 AIC 无线网卡时要求显式选择。已有自定义 HiCar `--network-interface` 和 CarPlay `CARLINK_CARPLAY_WIFI` 设置继续生效。

代码、纯安装测试、ARM 测试和镜像校验结果见 [验证记录](validation.md)，剩余实车及长稳项目见 [完成情况核对](status-audit.md)。
