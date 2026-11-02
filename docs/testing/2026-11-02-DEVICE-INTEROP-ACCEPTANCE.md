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
- Redmi 60 从 Studio 71 读取共享列表并拉取成功；将新收到的私有文件从产品界面转发后，电脑独立核对 262,161 字节与 SHA 一致。证据为 `fetch71-redmi60-studio71-private-bytes-verified.json`。
- 发布仅增加已核对的测试文件，未清空其他共享内容。四台电脑拉取样本均为 `weavelo-phone57-redmi.bin`，SHA-256 `8a3ff3df9098b6888f5f79d5a73d526cdf49eddf61c5e8a8f1cdc4137da98f72`。

各设备原始证据保存在各自主 checkout 的 `.rust-storage/validation-logs/weavelo-interop-20261029/`。最终手机四项字节证明为 Book 的 `phone59-nubia-four-receiver-sha-verified.json`；桌面拉取证明为各机 `fetch71-{host}-receiver-verified.json`。不向 Git 提交构建缓存或整份运行日志。

## 画面和输入

- Redmi 58 在真实已连接工作区中按系统返回键，已验证回到产品设备页；源码 `d7700c7` 统一了系统返回与断开按钮的关闭逻辑。
- 手机实际显示过 Linux 桌面，以及 Windows 两个独立窗口、应用图标。Android 60 双窗口实机截图为 Studio 接收文件 `phone60-two-windows-actual.png`，SHA-256 `f4db141bf97a63ec33ba21cd0046d0654a31b7b05a9f5659e5a578822831b564`。这证明可见画面，不代替持续稳定性、音频或输入验收。
- **Nubia 59 → HO5 71 → Studio 71** 两层真实链路中，使用手机触摸拖动 Studio 整屏中的原生测试窗口标题栏，测试窗口记录了位置从 `[480,270,720,432]` 到 `[877,140,720,432]` 的变化。HO5 两分钟观察含 57 次有效会话采样，控制均未锁定、连接均活跃、解码错误为 0；测试按其正常 120 秒期限结束。
- 上述单次拖动与两分钟状态观察不等于长时间连续拖动、中文输入或全部平台输入通过。Windows 单窗口测试仍收到 `INPUT_UNAVAILABLE: The selected window is covered at this pointer position`，目标测试窗口未收到点击。实际两个进程均非 topmost、同属 session 1、medium integrity；尚不能将其归因于 DPI 或权限级别差异。
- 自动点击脚本早期按压时间可能超过 380 毫秒长按阈值，不据此判定产品短按失败。修正为短按后，Nubia 60 → Windows 71 整屏中的系统最小化成功，独立窗口测试输入框也记录了真实按下／抬起；文字提交仍须独立验收。
- Linux 71 的单屏主动投送已在 Book 71 收到并提交画面。Book 再将该观看窗口推给 Redmi 60 时，初始为黑屏；实际 WindowServer 将窗口显示为约 122×146 的缩略图，应用没有隐藏。正常打开已安装 Weavelo 后，手机实际显示了 Linux → Book → Redmi 两层画面。正常激活的对照结果不是未激活窗口采集问题的产品修复。

## 音频

在现有 Linux 整屏 → Book 会话中，Linux 使用已安装的 `pw-play` 播放 4 秒、440 Hz、低幅度双声道测试音，不改系统音量。Book 原先非零样本计数为 0，测试后为 377,435，播放回调错误为 0。该结果验证这条链路的非零回放样本，不代替人工听感、频率比对或其他设备方向的音频验收。证据为 Book 的 `audio71-linux-book-before.json`、`audio71-linux-book-after-tone.json`。

## 本轮修复与持续测试

`03f0f20` 修复连接活跃判断只依赖时钟回复的问题：已接受的当前对端 Session、HostTelemetry、PipelineTelemetry 现在共用活跃时间更新；认证拒绝路径和原有超时时长不变。真实 UDP 回归在修复前 6.57 秒失败，修复后两项通过，非法明文不能污染认证会话的既有测试仍通过。macOS 生产配置检查及现有发布回归通过（1,039 项、19 项跳过）。

此前 Nubia 主动摄像头投送出现过 8 秒确认超时。Android 59 起保留结构化确认失败信息，`CameraService.dump` 可通过普通 ADB 读取，不修改系统日志策略。Android 60 → Studio 71 本轮连续观察约一小时，最后记录 188,331 解码帧、0 解码错误，未出现自主结束或确认超时；随后主动停止了这组测试。`phone60-root71-watch.jsonl`、`phone60-camera-one-hour-result.json` 与 Book 的 `phone60-source-service-diagnostics.jsonl` 为证据。

这轮并发负载中途有人为停止、更换 Windows 来源及正常打开应用，不能算作持续最大负载一小时通过，也不能证明先前确认超时已彻底消除。最后仍记录了 574 次参考帧缺口和 9,778 次队列丢弃，性能与画质稳定性尚未通过。

Android 60 已由正式发布脚本上传 NAS，并核对公网下载元数据与文件 SHA：[当前 APK](https://weavelo.hackerlife.fun:8443/download/Weavelo-Android-2.0.0-alpha.9-03f0f20-release.apk)。桌面安装与 NAS 发布分别记录，本条不声称桌面 71 已完成 NAS 发布。

## 尚未完成

持续投送确认超时的最终定位、完整连续输入与中文实测、定向投送中的本地／远端前后摄替换、桌面同源采集编码共享、屏幕音频实测、图片剪贴板与通知往返、完整窗口／全屏／输入法／旋转／DPI 验收，仍须继续。

Redmi 无障碍授权暂时无法由机主操作，未代为启用；普通手机屏幕投屏的新会话仍遵守系统确认。摄像头、文件与手机作为控制端的验收继续独立进行。USB 托管采集实现未获采用。

后续方案保持在 [摄像头共享](../plans/CAMERA-SHARING.md)、[统一共享与权限](../plans/UNIFIED-SHARING-PERMISSIONS.md)、[主动投送／画板／教学](../plans/ACTIVE-PUSH-WHITEBOARD-TEACHING.md) 与 [原生交互](../plans/NATIVE-REMOTE-INTERACTION.md)。本记录不将这些规划项目标为已完成。

## 帧队列过载策略

压缩帧不能逐帧随意淘汰：普通预测帧也可能是后续帧的引用。当前修改使桌面接收队列与 Android 后级压缩队列共用同一淘汰规则：新关键帧到达时替换整个旧 GOP；容量耗尽时保留尚未解码的关键帧，淘汰其普通帧尾部，并请求新关键帧，在此之前拒绝依赖已断的普通帧。已经解码的画面继续保留最新帧。切换来源时清除旧来源积压，不把旧帧重新标为新来源。

队列操作不会等待解码器，也不会阻塞网络控制循环。该修改没有新增协议、依赖或媒体后端。本节记录实现原则；本轮提交后的设备安装、真实呈现、延迟和持续负载结果须单独记录，不能使用前述 71/60 版本验据替代。
