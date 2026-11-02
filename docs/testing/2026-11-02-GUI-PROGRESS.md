# GUI 重构进展与接续点

2026-11-02。用户已批准主管理界面、独立单流窗口、统一操作栏和系统托盘方案；当前优先实际实现 GUI，Computer Use 与联合验收随后安排。

方案：[窗口职责与完整功能去向](/Users/jinliang/workspace/remote_hosts/docs/weavelo-window-ui-plan.md)。本轮短 checkpoint：`.rust-storage/validation-logs/weavelo-gui-20261102/CHECKPOINT.md`。代码始终在 primary `main` 的同一个 worktree 修改；每个完整小步检查后提交并推送。

## 当前开发

| 项目 | 状态 | 下一步 |
| --- | --- | --- |
| 管理与单流窗口入口 | 首步源码及检查通过；管理分支已接入真实产品 render，设备 View 改为独立窗口 | 继续固定来源、设备操作与实际 executable 视觉验收 |
| 管理窗口生命周期 | 管理实体保留、关闭 / 重新打开及其他窗口存在的 GPUI 回归通过 | 后台投送与真实文件连续性仍须实机验证 |
| 固定流身份、来源选择与 PiP | 查看设备先读取目录、选来源后才订阅；普通新来源另开窗，同源聚焦，PiP-only 恢复同一控制器；明确 Replace 才替换。软件回归通过 | 推送控制窗口及真实断线 / 来源结束仍需验收 |
| 管理导航、统一单流操作栏 | Studio 76 已签名安装；单流栏分组、当前设备文件上下文和显示参数范围的源码检查通过 | 实际操作栏展开、任务连续性与其他平台验收 |
| 紧凑设备操作及重复来源 | View / Send / Files / More、名称详情、已有来源聚焦和空浏览窗释放通过专项检查；Studio 76 管理行真实截图已核对 | 设置页及实际按钮操作 |
| 独立投送生命周期 | Studio 76 已安装；发送及接收均按请求保留，停止 / 续期 / 过期 / 丢失确认只处理对应请求 | 多设备并行及停止其中一路的实机验收 |
| 发送控制窗口 | Studio 76 实际打开独立发送窗；GUI 多选拆成独立来源请求，关闭 / 重开专项检查通过 | 实际控件点击、后台连续性与其他设备验收 |
| 紧凑管理与会话边界 | Studio 76 主窗实测外框 640×633（内容高 600）；会话页面移除全局设备设置入口，断开后保留该来源及重连入口 | 设置滚动及会话操作检查 |
| 关闭和 End stream | Studio 76 已安装；显式关闭和 End 共用当前流清理并退出窗口，39 项 GUI 检查通过 | 实际点击与远端连接结束验收 |
| 全屏与窗口入口 | 已移除接收主窗 / PiP 操作栏中的最小化；保留全屏 / 退出全屏，macOS 在系统全屏完成 / 退出事件中隐藏 / 恢复三按钮 | 39 项 GUI 检查及签名发布回归通过；等待正式构建、安装和实际全屏验证 |
| 原生 tray / menubar | 待完成；目前仅 Open / Quit | 设备、流、传输、设置的明确目标快捷操作 |
| Android 管理 / 单流页面 | 待完成 | 保留原生视频、手势、IME、旋转与服务 |
| Android 输入失败反馈 | Android 62 已由原签名正式构建、发布并安装到红米与努比亚；被拒绝的触摸 / 键盘原因按画面显示，保留来源与时效校验 | 单次点击与键盘绑定已有实机结果；持续输入及全部方向仍待验证 |
| 实机视觉与交互 | 红米 62 主动拉取 Studio 76、Windows 73 的 Weavelo 单窗口，努比亚 62 拉取 MacBook 73 的 Weavelo 单窗口均有真实画面；红米触摸成功切换 Studio Settings | 操作栏、持续输入、流关闭及其它来源继续验证；手机接收画面不等同宿主物理屏幕验收，不据此跨机部署 |

首步检查：原生生产配置构建通过，实际编译的默认入口仍为 `restored-original-gpui`；真实 GPUI 窗口关闭 / 重新打开 / PiP 引用测试 1 项通过；相关输入回归 37 项通过、1 项跳过。五处旧测试夹具已改用当前视频通道 API，未改动媒体内核。检查完整日志位于上述本轮 checkpoint 同目录。此处不宣称媒体连续性、物理呈现、安装或发布通过。

下一源码小步已完成来源动作接入：`restored_streams.rs` 共用当前目录校验与窗口查找；单流标题保留设备 / 来源并标识 Receiving；多选 Open 不再改写原流，Replace 为明确单选操作，失败保留原选择。真实 GPUI 回归包含同源聚焦、设备 / 来源改写拒绝及 PiP-only 恢复时不增加连接所有者。`restored_ui` 回归 38 项通过、1 项跳过；原生生产构建和签名发布回归 1,039 项通过、19 项跳过。尚未实机安装这批改动。

来源浏览小步已接入实际 View 入口：控制连接读取屏幕、应用窗口和相机目录，空目录与实际错误分别呈现；确认首个来源后复用该窗口及控制连接，其余来源独立打开。目录连接不受文件开关限制，重连保持用途；复用连接时明确应用所选来源，避免误拉默认桌面。刷新保留选择，已结束的来源不自动替换为桌面。目录响应代际只是本地状态，没有更改网络协议、鉴权或媒体内核。

新增真实回环控制消息＋GPUI 控制器回归：空目录、失败、重试及重连期间订阅数始终为零；确认后请求为所选 `Window(77)`，重连后的控制连接直接升级为视频连接，设备及窗口保持不变。该测试没有采集或显示真实画面，不是跨机鉴权验收。三组回归合计 64 项通过、2 项跳过；原生生产构建与签名发布回归已通过。完整证据为 `source-browser-flow-test-3.log`、`source-browser-regressions-3.log`、`source-browser-native-build.log` 与 `source-browser-release-regression.log`。

三个 GUI 提交 `0a02e5d`、`d2c22f1`、`0abafe7` 均已推送并核对远端 `main`。Studio 74 正式打包首次遇到本地依赖 `regex_automata` 加载错误；无并发编译时重建通过，未清理共享缓存。随后产物检查因无关 Sunshine LaunchDaemon 的符号链接拒绝继续。已将项目产物使用检查收束到实际引用产物的进程和打开文件，沿用项目生产者锁与原子替换，不扫描无关系统服务或构建；签名及安装身份核验保持原入口。针对性生命周期测试 63 项通过，完整签名发布回归 1,041 项通过、19 项跳过；修复提交 `5fa881f` 已推送核对。正式重打包通过，并用既有 Desktop 升级入口安装到 Studio 74，保持实际签名、配置与启动策略，不新增回退副本。实际进程 85768 的文件 SHA 为 `dd799123dd88d50cb8593e0f26ec0576c8509d4487c753825cd02ebc079ac2a1`；其余设备未部署 GUI 候选。

`studio73-via-redmi.png` 是 Studio 73 的真实 GPUI 窗口经实际视频链路显示在红米 Android 61 的截图。对应投送已按请求 ID 停止。此前红米停在“Sharing ended”仍拒绝新请求为“Phone is busy with another connection”；在应用内 Disconnect 后同一来源投送成功。这是明确的 Android 会话结束 / 新请求接续问题，留待 U6 修复。未改系统权限。

单流操作栏小步源码已完成：控制、文件、新画面、声音、显示、更多按当前窗口设备执行；原有文字 / 物理键、麦克风、剪贴板、来源替换、PiP、分辨率 / 帧率 / 码率、暂停、Fit / Fill 与遥测保留。文件复用现有连接与传输引擎，由应用级的设备上下文保留；全局传输页聚合原任务，关展示窗口不改目标；空闲文件连接可明确断开，活动传输仍保留原取消入口。流内参数不覆盖管理页默认值。PiP 在主窗口最小化时仍计入可见需求，重连时重新绑定原生帧槽与统计；未改压缩帧队列或原生媒体后端。

验证：`stream-controls-current-tests.log` 中普通 GUI / 会话 / 输入回归分别 40 / 12 / 12 项通过，默认分别跳过 1 / 1 / 0 项；其中完整 GPUI 输入回环已额外显式通过 1 项。该夹具使用原有合成 HEVC 素材的首个访问单元（`app/tests/fixtures/input-first-frame.h265`，5,013 字节）得到真实解码帧，模拟展示与窗口焦点，仅证明本地事件路由、按下 / 拖动 / 释放和面板隔离，不证明 OS 注入或物理呈现。旧夹具的文件连接、无帧及未激活窗口前提已校正；新增诊断曾重复加锁，已据调用栈修复。签名发布回归 `stream-controls-release-regression.log` 为 1,041 项通过、19 项跳过。提交 `339b719` 已推送核对；独立多路发送、原生菜单与 Android 页面仍未完成。

Studio 75 已通过正式打包与既有 Desktop 升级入口安装，实际运行版本 `20261102.75`、源码 `339b719`，可执行文件 SHA 为 `c44e9c93fea7191a224d1d2298bea1c49d92eb3626527354d7da0161465d6cb8`；签名、配置及启动策略保留，未新增回退副本。Book 73 的真实窗口通过独立接收窗送达 Studio，接收状态报告有解码与场景提交记录，截图另存 `studio75-receive-toolbar.jpg`；该截图中的新操作栏未明确展开，不能写成操作栏交互验收通过。Studio 整屏同时送到努比亚进行观察，主界面截图为 `studio75-manager-front.jpg`。两条流仅为本轮定向测试，结束时按各自请求 ID 清理。

设备操作小步将默认行收紧为四个图标，名称打开详情，更多入口保留原多来源与接收命令；已有观看不再禁用查看其他来源。未开始的同设备来源浏览器复用，同一已打开来源再次选择只聚焦原窗并关闭空浏览窗。真实 GPUI 回归覆盖列表重排后的设备目标和无第二次订阅：`device-actions-tests-2.log` 中 2 项控件、5 项模型、40 项 GUI 检查通过，1 项默认跳过；`device-actions-release-regression.log` 的签名发布回归为 1,041 项通过、19 项跳过。此处不宣称实际紧凑布局或其他设备部署完成。

提交 `0e748fe` 已推送并核对远端 `main`。随后独立投送小步将单一发送句柄和单一接收租约改为按请求保留，来源选择只观察自己的请求；管理实体关闭后仍维持各发送的心跳。定向停止、续期、到期和丢失确认不再清除其它请求；本地状态中的 `leases`、`sending`、`last_deliveries` 为数组，既有后台也按请求及来源匹配首帧与失败，未保留旧本地状态分支。网络媒体协议、鉴权和原生采集编码未改。每次来源选择仍遵循现有 1–8 个来源约束，取消了错误地把文件上下文也算入的全应用观看窗口限制。`independent-delivery-tests.log` 的后台状态 7 项、接收 / 生命周期 14 项、GUI 40 项检查通过，1 项默认跳过；包含同设备两路不会互认首帧、互相取消或续期的检查。签名发布回归 `independent-delivery-release-regression.log` 为 1,041 项通过、19 项跳过。每来源发送控制窗、托盘和 Android 页面仍待完成，尚无这批源码的实机部署结论。

独立投送提交 `ace72a1` 已推送核对。发送控制窗小步随后接入实际 GUI：多选的每个来源创建独立请求，连接、首帧前结束及异步失败在对应窗口显示；Activity 可重新打开并明确停止某一路。窗口标题、来源图标和文件入口固定该请求与设备，预览没有远端输入层。关闭窗口继续发送，文件上下文保持；命令行原有批量请求仍明确标为多来源，并以“Stop these sources”说明停止范围。来源目录及图标读取复用现有后台路径，缺少元数据时后台读取一次，不阻塞 UI 线程或引入视频 CPU 转换。`sending-window-tests-5.log` 中 GUI 38 项、接收 14 项、后台匹配 7 项通过，1 项默认跳过。真实 GPUI 窗口测试点击重开窗的停止按钮，确认另一发送及文件上下文保留；发送工作器为可取消的测试夹具，没有采集、OS 注入或物理显示验收。旧的“等待投送”对话框测试已随职责迁移为发送窗口状态 / 生命周期检查。

开发期间发现并修复了管理状态汇总的重复加锁：结构体字段表达式的锁临时值存活到整个结构体构造结束，又读取逐路状态时发生自锁。`sending-window-test-hang.sample` 给出调用栈；只终止了本轮已核对的测试进程组，随后同组检查通过，未影响已安装应用。后续接续时不得把这一未安装测试失败误记为线上问题。

发送控制窗签名发布回归 `sending-window-release-regression.log` 为 1,041 项通过、19 项跳过。用户随后明确要求管理窗口更小、拉流 / 被投送窗口仅含本会话内容；下一步将默认尺寸按窗口职责拆开，并移除会话面板中的全局设备设置入口，再正式构建检查。

发送控制窗提交 `049398d` 已推送核对。用户补充要求后，管理页的默认及最小尺寸已经单独设置，并受当前显示器工作区限制；关闭后重新打开仍按管理页尺寸。拉流窗口保留原视频尺寸，更多面板不再出现“Devices & settings”，发送控制窗也移除全局 Activity 导航，主窗口继续提供这些功能。未连接的拉流页显示本窗口身份、状态和“Reconnect this stream”；未选来源时重新连接目录，不隐式选择桌面。`compact-session-tests.log` 的 38 项 GUI 检查通过，1 项默认跳过；尚未以真实 executable 截图宣称视觉验收。

紧凑管理 / 会话边界签名发布回归 `compact-session-release-regression.log` 为 1,041 项通过、19 项跳过。用户进一步要求 End stream 和关闭拉流窗口均直接退出该窗口，并反馈 Android 拉取具体应用窗口失败；这两项继续处理，不能以既有整屏成功记录关闭它们。

紧凑管理提交 `38dd45b` 已推送核对。随后将显式关闭及 End stream 接入同一当前流清理：取消其连接尝试、输入及定向接收，释放该所有者的会话，关闭该流所有 PiP 和主拉流窗口；管理窗口、发送控制窗及别的流 / 文件不受影响。异常丢失展示窗口时仍可从保留的 PiP 恢复原控制器，显式关闭则按用户最新要求结束该流。`end-stream-window-tests-2.log` 的 39 项 GUI 检查通过，1 项默认跳过；新增真实 GPUI 关闭事件和 End 按钮点击，持有其他接收绑定及文件上下文验证没有误清理。本夹具没有真实采集或 OS 输入，不作为跨机结束验收。

本轮 End stream 签名发布回归 `end-stream-window-release-regression.log` 为 1,041 项通过、19 项跳过。正式构建前读取 Studio 75，旧的两条自有 QA 流均已结束，当前 sharing / receiving 为 0；仅剩管理窗口，不能继续把旧请求写为活动连接。Android 单窗口排查已从努比亚应用内主动打开 MacBook，真实目录连接成功，下一步选择可见的自有应用并检查首帧；这一阶段尚无应用窗口拉流成功结论。

`193002c` 已推送核对，并正式构建、兼容安装 Studio 76（`20261102.76`）。实际 GUI 进程 28811、渲染器 `restored-original-gpui`，可执行文件 SHA 为 `75e5fe4bfb66c079224534749842dde86342d07a9c93f8e53dcae84988f8b24d`；签名身份、配置、成员及启动策略保留，未新增回退副本。证据 `studio76-package.log`、`studio76-desktop-apply.log`、`studio76-installed-status.json`。主窗实际外框为 640×633；`studio76-manager-via-redmi.png` 核对了紧凑设备行和四个操作图标，发送的是明确的自有应用 `Window(17273)`。真实发送窗标题带来源、Redmi 目标及 Sending，后台显示该请求 Sending。没有据此宣称设置页、输入或全平台视觉验收通过。

Android 应用窗口新证据：努比亚 61 在应用内主动选择 MacBook 73 的 Weavelo `Window(8995)`，宿主目录为 2560×1664、后台有 1 路真实采集。最初手机提示 `Waiting for video from this device.`（该提示表示没有收到压缩视频包）；激活宿主窗口后，`nubia61-book-window-current.jpg` 已显示该应用真实像素。红米 61 接收 Studio 76 的上述单窗口也已出画。两者证明特定已验证窗口可拉 / 可推，不能据此关闭用户未指定目标的应用窗口失败；初始等待和恢复仍须独立定位。红米上的旧 Sharing ended 会话仍需应用内 Disconnect 才能接新投送，未改系统权限或背景偏好。

输入中间边界排查沿用现有会话：Android currentView / 事件时效 → 原生触摸视图、首帧与能力 → 当前输入绑定 → 发送 → 宿主校验 / 注入 → 实际界面。现有代码会吞掉键盘绑定错误，并在原生触摸前提不成立时静默丢弃新按下；这一反馈缺口已修正为当前画面的短提示，成功新操作或新视频表面会清除旧提示。提示使用已有视频覆盖层，不挤压画面、不改变指针变换；迟到事件仍取消、错误来源仍拒绝，清理先于返回错误。`android-input-readiness-tests-3.log` 的 9 项原生触摸 / 实际回环绑定检查通过，Android Kotlin 编译与 74 项 JVM 检查通过（`android-input-readiness-kotlin-check-2.log`）。版本代码准备为 62，尚未签名打包或安装；它用于暴露实际失效前提，尚未证明触摸生效。

输入反馈小步的签名发布回归 `android-input-readiness-release-regression.log` 为 1,041 项通过、19 项跳过。没有新增输入权限、降低保护时效或改变媒体协议。系统与应用授权、主机端过期输入误报及真实点击的验证仍分别保留。

Android 62 后续交付：`c8315ac` 已在 MacBook 的原 `main` checkout 正式构建（操作 `884cdebd-b26e-4541-8e17-791f8397ca22`，退出 0），使用该机现有私钥与 Keychain，未复制密钥。APK SHA-256 为 `fdc33557c5f34130b6762230a61e9168fb350156848333846821305e3b11e4a7`，原签名 SHA-256 为 `215aedabf17f40a454ad381d9027772eef9446346cae3db031cac2a5ae51c3a5`。正式发布脚本成功（操作 `633ee059-e3af-4799-bab6-2cd162b2051c`），同一 APK 经公开 Weavelo NAS 下载校验后升级红米；努比亚经 Book USB 升级，均由 61 变为 62、`adb install -r` 成功，保留成员和数据。Studio 本机没有 Android 发布私钥，不再重试本机签名。

62 实机复测：`nubia62-book-window.jpg` 是努比亚主动选择 MacBook 73 的 Weavelo 单窗口后的真实画面，普通点击键盘按钮后为 Hide keyboard，证明取得键盘绑定，不证明 IME 可见或文字注入。`redmi62-studio-window.png` 为红米主动拉取 Studio 76 管理窗；`redmi62-studio-after-settings-tap.png` 显示手机实际触摸已将宿主从 Devices 切到 Settings。`redmi62-cube-window.png` 是红米主动拉取 Windows 73 的 Weavelo 单窗口，已有真实画面。Linux 来源目录当前只返回 Display 1，`redmi62-linux-screen.png` 确认整屏成功；代码只发布该配对网络内实际获准的 portal 来源，不能把整屏权限当成窗口权限。用户原先失败的目标 / 应用仍未补充，未关闭该问题。列表使用 LazyColumn，仅首屏标签不能证明其它窗口不存在；搜索可找到 Studio 的自有窗口。

用户已补充目标为努比亚 → Mac Studio，并要求自行复现。62 在同一条链路上拉取 Studio 76 的 Weavelo 与系统设置「储存空间」均有真实画面（`nubia62-studio-window.jpg`、`nubia62-studio-systemsettings.jpg`）。继续选择 ChatGPT 时稳定提示 `Waiting for video from this device.`；对应 Studio 来源为 `Window(16937)`、进程 50594、1138×2069，宿主 outgoing_streams 为 1（`nubia62-studio-chatgpt-host-state.json`）。没有视频包不能归因为手机解码；窗口可见性、采集与编码的具体断点仍待核对，未关闭此 bug。

用户进一步要求取消拉流操作栏的最小化，仅保留全屏切换。已移除主接收窗与 PiP 的自定义最小化按钮，全屏提示区分进入 / 退出；macOS 在 AppKit `windowDidEnterFullScreen:` / `windowDidExitFullScreen:` 后隐藏 / 恢复系统三按钮，不增加逐帧原生调用。`fullscreen-window-tests.log` 的 39 项 GUI 检查通过、1 项默认跳过；`fullscreen-window-release-regression.log` 为 1,041 项通过、19 项跳过。源码已验证，尚未安装或声明实机全屏通过。

Studio 74 的 after 为 `studio74-via-redmi.png` 与 `studio74-via-nubia-before.png`；后者完整截图 SHA 为 `cee4cf7fdbb2c9ece293594f9da1ba8b7cad83f4c46b47895d88b50f7fac9ba7`。努比亚的实际触摸切换 Activity 后，接收端出现 `INPUT_UNAVAILABLE: input expired after target validation`（`studio74-nubia-activity-preview.jpg`，全分辨率原件保留在 Book 同名任务证据目录）；当前仅定位到原生目标校验后的截止时间，不推断为权限缺失。两次 Studio → 红米、一次 Studio → 努比亚测试投送已按各自请求 ID 停止。红米的原始 ADB 触摸注入被系统拒绝，未修改系统安全设置。

## 已知问题与接续条件

下列现场来自[六台设备互通记录](2026-11-02-DEVICE-INTEROP-ACCEPTANCE.md)，不因本轮 GUI 编译成功而关闭。原始证据在该记录引用的各机 `.rust-storage/validation-logs/weavelo-interop-20261029/`。

| 问题 | 已知事实 / 证据 | 下一步 |
| --- | --- | --- |
| 离屏或被系统缩成缩略图的 macOS 应用窗口无首帧 | Book 曾显示约 122×146 缩略图；Studio 73 主窗口 OS 不在屏幕上，30 秒未得到首帧；对应整屏成功。`video73-round1-studio-offscreen-main-failed.json` | 单独核对窗口实际可见性与采集回调；正常激活后的成功不能充当修复证明 |
| Mac 单窗口触摸在目标校验后过期 | 努比亚 61 → Studio 74 的 Weavelo 窗口，实际切到 Activity 后显示 `input expired after target validation` | 核对原生校验耗时及过期输入结果处理，保持截止时间、来源与权限校验；尚未稳定验收 |
| Android 接收整屏时输入状态不明 | 努比亚 61 接收 Studio 75 整屏，应用内触摸未改变目标页面，随后键盘按钮不再满足可用条件；视频仍有真实画面 | 分别核对输入授权、桥接绑定、画面就绪和事件送达，不先归因坐标或系统权限 |
| Android 拉取具体应用窗口失败 | 用户目标已确认努比亚 → Studio；62 的 Weavelo / 系统设置窗口成功，ChatGPT `Window(16937)` 稳定无视频包。Linux 当前只授权整屏 | 读取对应连接采集 / 编码计数并核对窗口状态；不把其他窗口成功当成修复 |
| 管理设备行的活动汇总 | Studio 76 顶部与发送窗已显示真实 Sending，设备行仍可能显示 Available | 后续共用逐设备活动摘要，避免把管理实体自身角色当作全部会话 |
| Windows 单窗口输入被覆盖判断拒绝 | 返回 `INPUT_UNAVAILABLE: The selected window is covered at this pointer position`；不能先归因为 DPI / 权限 | 保留窗口与点位证据，独立复现遮挡判断；整屏成功不代替单窗口 |
| 全方向输入、音频、图片剪贴板与通知 | 有部分实际链路记录；尚无最后版本全部方向连续验收 | 按版本与方向补测；解码计数不代替物理呈现或听感 |
| Android 普通被动整屏与控制授权 | 两机安装、型号身份、相机与文件已有记录；无障碍授权仍缺，普通新投屏会话仍需真实系统同意 | 用户方便时完成系统授权；不得自动确认或绕过安全设置 |
| Android 已结束的接收会话仍阻塞新投送 | 红米 61 显示 Sharing ended，新的 Studio 请求被拒绝为 Phone is busy with another connection；应用内 Disconnect 后同源重试成功 | U6 修复已结束流的会话状态与接收准入，保留仍在运行的文件和其它流 |
| Linux 编码性能 / 同源多接收端 | 有拉流与单屏主动投送成功记录；CPU 编码和全部并发场景未完成性能验收 | GUI 稳定后针对平台测量，保留原生媒体与共享逻辑 |
| 画板与教学模式 | 已规划，尚未实现 | GUI / 基础互通后按既有计划推进 |

帧队列优化已另行提交和安装，源码及设备版本见互通记录；本 GUI 任务不重复修改媒体内核。所有未测项目维持“未知”，不把源码提交或 NAS 发布写成实机交付完成。

## 功能迁移与所有权

| 当前实现入口 | 目标位置 / 所有者 | 保留条件 |
| --- | --- | --- |
| `run_original_dashboard`、`Instance`、`directed_receive::Adapter` | 应用级管理协调实体 | 管理展示关闭后继续接收激活与投送请求；不再启动网络运行时 |
| `drawer_devices_tab`、`DeviceListState`、`dispatch_device_action` | 主设备页与定向设备快捷操作 | 每个动作携带设备 ID，不随列表排序改目标 |
| `drawer_device_group_tab`、配对码 / QR / 图片、名称编辑 | 添加设备与设置 | 复用已有密钥、成员及名称编辑器，不覆盖本地草稿 |
| `drawer_security_tab`、本机权限、Linux 共享 | 设置 → 共享与权限 | 权限入口明确属于本机，实际 OS 同意不代替用户确认 |
| `OriginalOwner`、`SourceViewBinding`、`source_selection` | 固定设备与来源的单流 | 保留来源 revision、输入确认与取消；新来源另开窗，替换必须明确 |
| `floating_control_island`、文本 / 物理键、`RemoteTextInput` | 当前流统一操作栏与输入层 | 本地面板拿到焦点后不透传；保留文字提交、组合键与手势取消 |
| `media_controls`、显示设置、遥测、Fit / Fill、PiP | 单流显示 / 声音面板与展示引用 | 原生帧、参数和呈现绑定同一来源；PiP 不改变动作目标 |
| `connect_files`、`file_transfer_panel`、`FileViewBinding` | 当前设备文件上下文与全局传输中心 | 文件不依赖拉流；发送、发布、分页、目录、取消与校验均保留 |
| `directed_send`、`Lease`、已有订阅 | 推送控制窗口与活动页 | 关闭预览不等于停止；停止一个来源 / 对端不影响其余任务 |
| `tray::{mac,windows,linux}` | 原生菜单栏 / 托盘 | 共用定向动作与已有后台，保留完整退出的真实清理确认 |
| Android `MainActivity`、`WorkspaceScreen`、`MobileViewportScreen` | 管理页及固定逻辑流页面 | 保留直触、触控板、IME / 旋转、文件与原生服务，不模拟桌面多窗口 |
