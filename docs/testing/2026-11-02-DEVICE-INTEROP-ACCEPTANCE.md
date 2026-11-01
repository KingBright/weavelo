# 六台设备互通实测记录

2026-11-02。总体验收尚未完成；以下区分已安装版本、已验证行为与待解决问题。

## 当前安装

四台电脑均为原 GPUI 产品入口、build `20261102.71`；两部手机均为 Android `versionCode=60`。本轮产品源码为 `03f0f20d589920da6ae917f315bf4f15d6342343`。保留现有设备身份、分组、签名及系统授权；本记录的后续文档提交不表示客户端发生了升级。

| 设备 | 产品设备名 | 实际运行验据 |
| --- | --- | --- |
| Mac Studio | MacStudio.local | 已安装并启动；GUI generation `87497-1793569679085639000` |
| MacBook | MBPM2MAX.local | 已安装并启动；GUI generation `93991-1793570205372851000` |
| Windows cube | CUBEX | 已安装并启动；GUI generation `3984-1793569929958223300` |
| Linux HO5 | bazzite | 已安装并启动；GUI generation `1094925-1793569660304749671` |
| Book USB 手机 | nubia NX741J | ADB 安装成功后独立读取 `versionCode=60` |
| Studio USB 手机 | Redmi 2409BRN2CC | ADB 安装成功后独立读取 `versionCode=60` |

两部手机同时出现在设备目录，名称使用实际型号。通用旧名称的迁移不覆盖用户自定义名称。Android 60 的正式默认 APK 为 18,800,939 字节，SHA-256 `940a9cce80f8c9749b74db5277273ba1686b7bfe69448a6084ed49b154575d0a`；现有签名指纹为 `215aedabf17f40a454ad381d9027772eef9446346cae3db031cac2a5ae51c3a5`。

## 文件

- 多轮真实发送累计完成六台设备全部 **30 个有向方向**，每个方向使用 262,161 字节文件，并完成接收后独立 SHA 校验。该累计覆盖桌面 67–71、Android 57–60 的多轮安装过程，不声称全部方向均在最后一次升级后重跑。
- 手机私有接收目录不能由 shell 直接读取。其字节验证通过产品内“已接收文件 → 发送”将实际收到的文件转发至电脑后独立计算 SHA；接收状态或发送方 `remote_content_verified=false` 不充当接收端证明。
- 桌面 **71 当前安装版本**之间的“发布单一文件 → 读取共享列表 → 使用该列表身份拉取 → 独立接收 SHA”全部 **12 个有向方向通过**，包含 Book → Studio 反向 Fetch。
- 发布仅增加已核对的测试文件，未清空其他共享内容。四台电脑拉取样本均为 `weavelo-phone57-redmi.bin`，SHA-256 `8a3ff3df9098b6888f5f79d5a73d526cdf49eddf61c5e8a8f1cdc4137da98f72`。

各设备原始证据保存在各自主 checkout 的 `.rust-storage/validation-logs/weavelo-interop-20261029/`。最终手机四项字节证明为 Book 的 `phone59-nubia-four-receiver-sha-verified.json`；桌面拉取证明为各机 `fetch71-{host}-receiver-verified.json`。不向 Git 提交构建缓存或整份运行日志。

## 画面和输入

- Redmi 58 在真实已连接工作区中按系统返回键，已验证回到产品设备页；源码 `d7700c7` 统一了系统返回与断开按钮的关闭逻辑。
- 手机实际显示过 Linux 桌面，以及 Windows 两个独立窗口、应用图标。Android 60 双窗口实机截图为 Studio 接收文件 `phone60-two-windows-actual.png`，SHA-256 `f4db141bf97a63ec33ba21cd0046d0654a31b7b05a9f5659e5a578822831b564`。这证明可见画面，不代替持续稳定性、音频或输入验收。
- **Nubia 59 → HO5 71 → Studio 71** 两层真实链路中，使用手机触摸拖动 Studio 整屏中的原生测试窗口标题栏，测试窗口记录了位置从 `[480,270,720,432]` 到 `[877,140,720,432]` 的变化。HO5 两分钟观察含 57 次有效会话采样，控制均未锁定、连接均活跃、解码错误为 0；测试按其正常 120 秒期限结束。
- 上述单次拖动与两分钟状态观察不等于长时间连续拖动、中文输入或全部平台输入通过。Windows 单窗口测试仍收到 `INPUT_UNAVAILABLE: The selected window is covered at this pointer position`，目标测试窗口未收到点击。实际两个进程均非 topmost、同属 session 1、medium integrity；尚不能将其归因于 DPI 或权限级别差异。

## 本轮修复与持续测试

`03f0f20` 修复连接活跃判断只依赖时钟回复的问题：已接受的当前对端 Session、HostTelemetry、PipelineTelemetry 现在共用活跃时间更新；认证拒绝路径和原有超时时长不变。真实 UDP 回归在修复前 6.57 秒失败，修复后两项通过，非法明文不能污染认证会话的既有测试仍通过。macOS 生产配置检查及现有发布回归通过（1,039 项、19 项跳过）。

此前 Nubia 主动摄像头投送出现过 8 秒确认超时。Android 59 起保留结构化确认失败信息，`CameraService.dump` 可通过普通 ADB 读取，不修改系统日志策略。Android 60 → Studio 71 的持续测试及 Windows 并发来源测试仍在执行；`phone60-root71-watch.jsonl` 与 Book 的 `phone60-source-service-diagnostics.jsonl` 为本次证据。过程中人为停止、最小化或更换 Windows 测试来源需单独记录，不能算作持续最大负载通过，也不能混同摄像头自主失败。

## 尚未完成

持续投送确认超时的最终定位、完整连续输入与中文实测、定向投送中的本地／远端前后摄替换、桌面同源采集编码共享、屏幕音频实测、图片剪贴板与通知往返、完整窗口／全屏／输入法／旋转／DPI 验收，仍须继续。

Redmi 无障碍授权暂时无法由机主操作，未代为启用；普通手机屏幕投屏的新会话仍遵守系统确认。摄像头、文件与手机作为控制端的验收继续独立进行。USB 托管采集实现未获采用。

后续方案保持在 [摄像头共享](../plans/CAMERA-SHARING.md)、[统一共享与权限](../plans/UNIFIED-SHARING-PERMISSIONS.md)、[主动投送／画板／教学](../plans/ACTIVE-PUSH-WHITEBOARD-TEACHING.md) 与 [原生交互](../plans/NATIVE-REMOTE-INTERACTION.md)。本记录不将这些规划项目标为已完成。
