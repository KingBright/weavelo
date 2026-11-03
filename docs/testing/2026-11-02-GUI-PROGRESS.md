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
| 全屏与窗口入口 | Studio 77 已正式签名安装；接收主窗 / PiP 移除最小化，保留全屏 / 退出；macOS 全屏完成 / 退出后隐藏 / 恢复三按钮 | 39 项 GUI 检查及签名发布回归通过；实际全屏进出仍待验收 |
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

`7267eae` 已推送核对并通过正式包装 / 安装流程升级 Studio 77（20261103.77）。GUI 进程 9836、后台 9830，可执行 SHA 为 `4ba2130cc5b1daf5ef102138067aa62ba555b48de7532532fa18e42f4312bac9`，渲染器仍为 `restored-original-gpui`；签名、成员、配置和启动策略保留，未新增回退副本。证据 `studio77-package.log`、`studio77-desktop-apply.log`、`studio77-installed-status.json`。升级前仅停止了本轮自有失败测试连接，实际全屏视觉与输入验收仍独立保留。

Android 63 接续小步修复：已结束的定向接收清除画面，所有者在该连接文件任务完成、近期文件命令安静后释放并退出结束页，后台休眠时也会核对结束状态；文件仍忙时保留当前文件上下文，旧页面 / 服务的迟到清理不能关闭新设备或新请求。普通拉取和仍活跃的投送保持原生命周期。被拒绝的普通拉取现按具体画面显示原因，提供同源重试并分配新订阅编号，避免旧错误污染新尝试；拒绝页面不再空转解码器。尚未用此改动声称 ChatGPT 无视频包已修复。

努比亚只读检查发现 `log.tag=S`，普通日志被系统静音，未修改任何日志、权限或安全设置。63 在 Android 已有 Service `dump` 接口提供连接编号、代次、连接 / 结束状态和画面计数，用于关联 Studio 既有 `InspectConnection` 标量诊断；不输出票据、会话密钥、窗口标题、地址、文本或文件。首轮增量 Kotlin 编译不能解析未修改文件的顶层声明，保留证据后仅对本次检查使用 `-Pkotlin.incremental=false`，完整 Kotlin 编译和 76 项 JVM 检查通过，未清理缓存或改永久构建设置。新增两项所有权生命周期回归覆盖文件未完成保留、完成后新投送、迟到旧清理与普通拉取隔离；最终 `android63-kotlin-check-final.log` 与 `android63-release-regression-final.log` 均通过，后者 1,041 项通过、19 项跳过。63 尚未安装。

63 已正式签名、发布并安装两部手机：源 `1bbf090`，Book 构建 `cbb0a26a-6aee-4174-83c8-f92985b2b488`、发布 `2f699be9-4ebf-4ad2-b097-3fe84b77ef21` 均退出 0，APK SHA 为 `1237daf0e0c9ddc1951a0cd3175569fc72e1a2b7330671d6e9d427c34a83f507`，沿用原 Android 签名。`nubia63-install.json` / `redmi63-install.json` 确认 62→63。只读服务诊断实际返回 connection_id `3707669065`，Studio 既有诊断可按此编号看到该连接采集 / 编码帧数；观察到自有 Weavelo 接收窗均为 36，不把它当作失败来源的采集证据。

Studio 77 全屏实机：Book 73 自有窗口 `Window(8995)` 投送到 Studio，生成固定接收窗 `Window(17330)`；再由努比亚观察并操作该窗。实际进入全屏后来源从 1280×833 变为 1920×1080，画面继续，操作栏无自定义最小化，全屏系统按钮未显示（`nubia62-studio77-toolbar-detail.jpg`、`nubia62-studio77-receiver-fullscreen.jpg`、`nubia63-studio77-focused-fullscreen-toolbar.jpg`）。退出点击尚未验收：一轮明确返回 `INPUT_UNAVAILABLE: Restore the selected window on the host desktop before controlling it`，对应接收窗已在另一桌面而管理窗活跃；从管理页 Activity 的 Open 实测能重新激活接收窗（`nubia63-studio77-manager-activity.jpg`），再点仍需核对落点。保留输入安全判断，未把问题先归为权限或坐标。

为区分真实全屏尺寸与恢复用尺寸，私有 GUI 状态改读实际全局 `window.bounds()`，增加 full-screen 标记、视口尺寸和鼠标相对位置；只在请求状态时读取，不添加逐帧日志或 UI 诊断入口。这是下一次签名安装后的定位依据，尚不宣称退出全屏或窗口拉流 bug 已修复。

另已实测目录混入原生标题栏按钮小窗：Studio 同进程的 `Window(17387)` / `Window(17386)` / `Window(17391)` 都显示为「Window」、66×20，且错误宣称可输入（`studio77-caption-catalog-sources.json`）。现在来源目录复用现有输入焦点校验中已确认的标题栏几何规则：同进程、普通窗口层、66×20、父窗左上角 6 像素；不按名称或尺寸单独过滤，不排除正常离屏应用窗口、实际小内容窗或弹窗。每次目录只读取一次 WindowServer 快照，在 AX 尺寸及图标查询前排除这些内部表面。输入命中及焦点保护保持原规则，新增检查覆盖离屏父窗和真实内容 / 弹窗保留。`native-window-context-tests.log` 的 22 项宿主窗口 / 输入检查及 39 项 GUI 检查通过，1 项默认跳过；`native-window-context-release.log` 为 1,041 项通过、19 项跳过。真实目录验证仍待安装，不把此项当作所有无首帧问题已修复。

Android 63 接续实机两轮：Studio 77 向红米发送管理窗口，真实出画（`redmi63-incoming-cycle1.png`）；停止请求 `25740-1793640621651529000` 后红米自动回设备列表，原 Book→Studio 接收仍为 1。无需手动 Disconnect 发起第二请求 `27444-1793640825600149000`，再次进入接收，手机没有等待首帧提示；停止后再次自动回设备列表。Book 自有发送 `28784-1793637996674764000` 也已定向停止。这两轮没有活动文件任务，文件保留仍由生命周期回归覆盖。

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

## 2026-11-03 全屏坐标接续

Studio 78 已按原身份签名安装（源码 `53f195b`，构建 `20261103.78`）。红米 63 拉取 Studio 的固定接收窗 `Window(17394)`，触摸实际激活窗口、更新本机鼠标位置；键盘绑定可打开和关闭。点本地操作栏全屏后，实际全局范围和视口均为 1920×1080，画面继续且系统标题栏按钮隐藏。图像与状态证据为 `redmi63-studio78-fullscreen.png`、`studio78-fullscreen-own-geometry.json`。退出仍未验收：可见按钮的点击没有退出，已核对输入实际到达的窗口坐标；在窗口已全屏时重新开流仍出现相同现象，不能先归因于动态解码尺寸或系统权限。

全屏目录实测仍混入 `Window(17419)`：66×20、位置 (6, -26)，属于停放在内容窗上方的隐藏 32 点原生标题栏。现补齐目录过滤，要求同进程、普通层、精确标题栏几何、透明隐藏条及相邻内容窗全部匹配；不改变输入命中或焦点安全规则。新增检查验证真实小窗、缺失内容窗、不同进程及可见标题栏仍保留。

既有已知连接诊断增加原生帧格式变化时的尺寸、有效内容矩形和缩放标量；不记录像素、标题或输入，不新增网络协议或采集后端。每帧只检查一次原子格式值，附件读取仅在首帧或格式变化发生。下一次签名安装后用实际采集矩形继续定位，不把诊断当作已修复。针对性检查为宿主 23 项、GUI 39 项通过及 1 项跳过；签名发布回归 1,041 项通过、19 项跳过（`fullscreen-capture-geometry-tests.log`、`fullscreen-capture-geometry-release.log`）。本轮手机拉流和 Book→Studio 投送均已按自身连接与请求停止，Studio 活动计数归零。

Studio 79 已签名安装（`24ebeb3` / `20261103.79`）。努比亚 63 拉取接收窗 `Window(17427)` 并真实点击进入全屏，来源 1920×1080；全屏目录仅保留两个真实 Weavelo 内容窗，隐藏按钮小窗已消失。连接 `854573969` 的原生帧数据为输出 986×554、有效内容宽 956.55、缩放 0.4982；它对应包含额外 32 点高度的缩放，而输入仍使用真实 1920×1080。普通窗口为 1140×740、有效内容约 1137.09×740，没有同样的边距。`nubia63-studio79-window-counters.json` 和 `nubia63-studio79-fullscreen-counters.json` 保留原始标量。

Apple SDK 和当前运行时均确认 `includeChildWindows` 默认为 true。现对固定单窗口来源显式关闭关联子窗采集，以避免隐藏标题栏改变画面范围；整屏采集设置保持原状。补充原生帧的 bounding / screen 矩形，便于新版对照。当前仅通过本机原生采集检查与 1,041 项发布回归（19 项跳过），真实全屏退出仍待下次签名安装验证，不能以配置修改代替实机结论。

Studio 80（`73e7179` / `20261103.80`）按原签名正式安装后，努比亚 63 和红米 63 均实际拉取固定接收窗 `Window(17456)`、进入全屏、点击画面上可见的退出按钮，并恢复到 1280×833。两部手机同时观看时，任一端切换均未结束另一端或 Book→Studio 的原始流；最后状态为共享 2、接收 1。努比亚全屏采集恢复为 buffer 986×554、有效内容约 984.89×554、screen rect 1920×1080，额外 32 点标题栏不再改变缩放。编码取偶数尺寸仍有少量边缘像素舍入，未声称亚像素精度或物理显示延迟已经验收。

实际画面为 Book 证据目录中的 `nubia63-studio80-fullscreen.png` / `nubia63-studio80-restored.png`，以及 Studio 证据目录中的 `redmi63-studio80-fullscreen.png` / `redmi63-studio80-restored.png`。两次本机状态分别保留在 `studio80-after-visible-fullscreen-exit.json` 与 `studio80-redmi-after-visible-fullscreen-exit.json`；原生采集标量见 `nubia63-studio80-after-enter-counters.json`、`redmi63-studio80-after-enter-counters.json`。全屏系统按钮未出现、恢复后标题栏出现，操作栏仅保留全屏按钮，没有自定义最小化。本结论覆盖上述自有应用窗口和两部手机，不关闭此前特定应用 0 首帧、Linux / Windows 完整交互、U5 托盘与 U6 移动页面等剩余事项。

努比亚 63→Studio 80 另已直接拉取系统设置「储存空间」窗口 `Window(17033)`，真实出画（Book：`nubia63-studio80-systemsettings.png`），连接 `2904842676` 的捕获与编码均持续增长。这补充普通应用窗口验证，仍不代替此前特定应用 0 首帧的结论。Studio 80 正式 NAS 发布及公网包校验通过，macOS 包 SHA-256 为 `672019bc4444f35eec1d50e0289b196c89280ef7ed92ab1f115a245791a65424`，Linux / Windows 发布条目保持原值（`studio80-nas-verified.json`）。

## 原生输入过期与控制可用性

此前实机已记录 `input expired after target validation` 被当作 `INPUT_UNAVAILABLE`，使一次输入超时变成整个控制暂停。现使用共享的原生输入过期错误类型，涵盖 Mac 目标校验、激活等待和原生分发的既有截止时间。宿主取消当前手势、释放按住状态并撤销该输入租约；过期事件不重放，由既有绑定心跳获取新空租约。真正的权限 / 目标错误，以及无法确认的清理失败，仍按现有错误路径上报。没有延长 250ms 预算、跳过窗口校验或新增协议。

针对性检查以原生替身制造目标校验后的过期，验证旧租约撤销、按键释放、不重放、下一次新绑定可用，同时验证真实权限错误仍上报；该项及 23 项窗口 / 焦点检查通过。签名发布回归 1,041 项通过、19 项跳过（`native-input-expiry-tests.log`、`native-input-expiry-release.log`）。尚待新包的真实拖动与持续输入验收；已提交文本的部分完成 / 未知结果仍保留单独拒绝与防重复处理，不能概括为所有输入中断均已解决。

Studio 82（`c7e2d05` / `20261103.82`）已按原签名安装。努比亚 63 拉取专用原生输入窗口 `Window(17514)`，一次触摸仅产生一次真实按钮动作；连续拖动正确释放且未显示控制不可用，但终点停在 (410,185)，预期为 (420,190)。同一流的原生采集为 640×452，content / bounding 均完整覆盖，无缩放边距。这复现了独立于采集的最后采样丢失：触摸 Up 只发松键，未发抬手新坐标。证据为 `input-fixture-baseline.json`、`input-fixture-after-click.json`、`input-fixture-after-drag.json` 和 `nubia63-studio82-after-native-drag-counters.json`。夹具仅观察自己的窗口，不捕获或注入全局输入；不是产品外观验收。

现将最终坐标放到松键之前：共享移动端直触映射补齐 Up 坐标，桌面接收窗也使用 MouseUp 自身的位置，并仅对已按下的按钮执行；离开画面和取消仍保持原释放规则。已有移动手势回归改用不同于上一 Move 的 Up 点；真实 GPUI 事件路由 / 本地协议记录夹具同样验证末点紧邻松键，并保留面板隔离和键盘检查。15 项共享手势检查、1 项显式 GPUI 回环及 1,041 项发布回归通过（19 项发布检查跳过；`final-pointer-position-tests.log`、`final-pointer-position-release.log`）。Android 版本码升为 64，仍待原身份签名安装后的实际终点与键盘挤压验收。

Android 64 已用原签名正式发布并升级两部指定手机，APK SHA-256 `66912fd28b0691ef59cdddbcc493ee94fc5ebf17967d773d409b6b7e6da9164e`，来源 `ff8e66a`。Studio 83 与 Book 83 均已按各自原身份正式安装；Book 实际 GUI 为 640×600 视口（`book83-installed-status.json`），尚需真实外观 / 接收窗交互复核。努比亚 64→Studio 83 实际拖动终点 (519.998,229.999) 对应目标 (520,230)，末点修复已实测生效（`input-fixture-nubia64-after-drag.json`、Book `nubia64-studio83-inputfixture.png`）。

原生窗口还验证了输入法提交：努比亚 63→Studio 82 输入 `abc`，拼音组合阶段不改远端字段，确认后准确出现 `abc你好`，退格得到 `abc你`，隐藏键盘没有重复提交。努比亚 64 可见键盘对照复用了此前授权，临时选已启用百度 OEM 输入法，视频由 y=1139 上移到 y=642，宽高不变；键盘可见时点同一远端按钮，真实次数从 1 变 2，随后视口恢复且已核对默认输入法恢复微信（Book `nubia64-inset-ime-restore.json`）。未把输入事件计数当作远端 Return 或 emoji 验收。

红米 64 出画与键盘布局通过，视口 y=704→379、宽高不变（`redmi64-fixture-keyboard-inset.png`）。系统拒绝了自动触摸注入；标准 ADB 触点连应用自身键盘按钮也未触发，因此不把红米触摸记为通过。键盘结束还出现 `remote keyboard source changed`，属于另需修复的产品路径，不归咎于触摸工具。

## 输入法与局部布局取消

桥接层原先把局部手势取消翻译成整份输入租约释放，会使同一来源的输入法上下文失效。现保留既有消息协议，局部布局 / Touch Cancel 只发送实际按住鼠标的松键，保留同源文字归属与键盘状态；冻结超时、来源隐藏、采集重置 / 结束仍释放全部输入并拒绝旧键盘。Android 版本码升为 65，待原签名升级实测。

12 项工作区回归通过，包含本地完整协议夹具：取消拖动能松键、同源文字归属不变、界面冻结仍由 watchdog 释放、Surface end 仍使旧键盘失效；签名发布回归 1,041 项通过、19 项跳过（`ime-layout-owner-tests-final.log`、`ime-layout-owner-release.log`）。夹具的取消事件读取已改用正确的 Event 过滤，并加 2 秒等待上限，避免测试自身无限等待。真实红米文字问题是否因此解决仍待新包复测。

## 浮动窗口与实际键盘焦点

Android 65 已正式发布并原签名升级两部指定手机，APK SHA-256 `4e37c7539279ee6de3ed744f2e2058b26fccc068bf9b42f71ab924ae8c89d4bf`、来源 `380d3e8`。红米仍未提交测试文字，隐藏键盘后显示来源变化；努比亚通过真实画面点击同一输入框后也未提交，不能将前一处手势取消修复概括为键盘已通过。

这次读到更早的宿主拒绝：`INPUT_UNAVAILABLE` 指出目标没有键盘焦点。原生枚举显示同进程浮动小窗 `17515`（layer 3，84×77）排在真正的输入窗口 `17514` 前面；原校验直接以窗口层序推断键盘焦点。现只对这种浮动层增加系统焦点确认：前台应用的 `AXFocusedWindow` 与焦点编辑器的 `AXWindow` 必须相同，进程一致，公开 AX 几何还必须唯一匹配当前 WindowServer 窗口。普通路径不增加 AX 查询；缺失或歧义仍拒绝，普通对话框、菜单及鼠标命中规则保持原校验。24 项原生窗口检查通过，包括浮窗、模态窗口、相同几何歧义和普通路径查询次数；真实输入仍待签名升级对照。

手机把输入拒绝显示在对应画面，保留最初的可操作原因，避免随后旧输入法回调用“来源变化”覆盖它。定向接收初始化也从现有原生连接取得真实 connection ID；先前 claim 已消费 Opened，诊断错误地保留 0。现有完整协议夹具验证该 ID 与宿主实际收到的 Open 相同，不新增网络字段或权限。Android 版本码升为 66，仍待正式构建与实机验收。关联夹具 1 项、原生窗口 24 项及签名发布回归 1,041 项通过（19 项发布检查跳过；`claimed-connection-id-tests.log`、`floating-focus-tests.log`、`floating-focus-release.log`）。

Book 83 主界面实际像素已通过努比亚查看，640×600 视口的设备、活动、传输、设置与紧凑操作入口均可见（Book `nubia64-book83-manager-active.png`）。首次投送该窗口时，手机虽接受来源却没有收到视频；激活同一 Book 窗口后立即出画，尚未确定首帧停在哪个原生阶段。另一次普通拉取已能显示被覆盖的发送控制窗，连接 `3616853057` 捕获 / 编码均为 1,866 帧，因此不能仅将故障归因为遮挡。保留该首帧问题继续定位，未记录为已修复。


Studio 84（60cae98 / 20261103.84）已原签名安装。保留同一努比亚 65、同一原生输入窗口 17514，真实点输入框并提交 pqr，字段从 abc你准确变成 abc你pqr；红米 65 再准确提交 stu，字段为 abc你pqrstu。两边收起键盘均无重复或来源变化错误；原浮窗 17515 / layer 3 仍在，说明验证覆盖了这次焦点修复。红米使用了已由努比亚点中的原生输入框，自动触摸限制仍单列未知。原始证据为 input-fixture-two-phone65-studio84-accepted.json、redmi65-studio84-committed-input.png。Book→Studio→努比亚两层实际画面也已截图：Book 的 nubia65-book83-studio83-nested-manager.png。

Studio 84 正式 NAS 发布已安装条目并校验公开 ZIP，SHA-256 8b688f7a73d580e44268a51df3690f7661cd7dd7ff269bdc1694f367eb1cd868。发布脚本随后一次元数据连接超时（退出 28）；已只读补验公开清单、验收页和索引，不重放上传，记录在 studio84-nas-verified.json。

Android 66 已按原签名发布并升级两部手机，APK SHA-256 e33ccf003e70ebb9d1a7b0a15233e7b402302de5bb3e67946952ea82f4e7f7bc。Book 83→努比亚 66 的单应用投送正常首帧，定向连接 ID 3603936051 已正确保留，首次采样捕获 / 编码均 69 帧，后续 1856 帧；这验证诊断关联及正常路径，仍不关闭先前偶发零首帧问题。

红米 66 的拒绝路径复测发现自动收起键盘可使后续回车触发本地返回。67 移除这项自动界面跳转，保留原始错误、严格文字归属与当前键盘入口，由明确的用户动作重新获取编辑上下文。正常文字测试的追加断言曾失败：原生字段最终为 vw，已记录实际值，不将未确认的原生选中态归为重复输入；下一次用显式右方向键定位光标，再检查恢复和不重放。

## 四台电脑与 Android 67 接续验收

两台 Mac 的 20261103.84、HO5 与 cube 的 `60cae98` 均已通过各自正式包及兼容升级入口安装。Mac 保留各机原授权签名；Linux 使用已验证的 FFmpeg / Opus 本机 SDK，Windows 沿用原安装位置与成员身份。实际 GUI 均为 restored-original-gpui，管理窗口视口约 640×600；不是把编译成功记为安装成功。安装记录分别为 `studio84-installed-status.json`、Book 的 `book84-installed-status.json`、HO5 的 `ho5-current-installed-status.json`、cube 的 `cube-current-installed-status.json`。

两部指定手机已原签名升级 67，来源 `34a74c1`，APK SHA-256 `75d09a53975000f233d565424e041602373766f2259ace676fd5a4358ffe1800`。努比亚通过普通设备、来源选择入口拉取 Studio 的自有原生输入窗口，已看到实际画面。故意把 Studio 焦点切至另一自有 Weavelo 窗口后，文字被 `INPUT_UNAVAILABLE` 正确拒绝，按回车仍留在原流、保留键盘入口和具体原因，原生字段不变。明确隐藏键盘、点击远端输入框并重新打开键盘后，右方向键定位，再提交 `789 `，字段精确等于先前值加这四个字符；被拒绝的 `456 ` 没有重放。微信输入法保持原选择。证据 `nubia67-rejected-input-explicit-recovery-accepted.json`，这覆盖 67 的实际拒绝与恢复路径。

HO5 沿用已授权的唯一 `Display(2147483649)` 主动投送至 Studio，实际接收窗 `Display 1 · bazzite · Receiving — Weavelo` 已出画；连接 `3446458774` 的原生解码及场景提交持续增长。该接收窗再投至努比亚，实际截图 `nubia67-ho5-current-manager-actual.png` 可见 Linux 桌面及发送控制窗。它证明这条主动发送与两层链路出画，不等于 Linux 全部窗口授权、输入或性能验收。

红米 67 通过普通来源选择拉到 cube 的 Weavelo 管理应用窗口，截图 `redmi67-cube-current-manager-actual.png`；随后用同一来源主动投送也已实际出画，截图 `redmi67-cube-active-manager-actual.png`，对应连接 `656168565`。Windows 的协议来源是目录分配的 `Window(4)`，不是原生 HWND；先前 QA 误传 HWND 得到“来源结束”已明确排除，不记录为产品采集故障。没有把这些成功窗口当作此前特定应用零首帧问题的普遍修复。

发送控制窗残留的自定义最小化按钮已移除，保留全屏 / 恢复，与接收窗一致。两项发送窗口生命周期回归通过，签名发布回归 1,041 项通过、19 项跳过（`sending-fullscreen-only-tests.log`、`sending-fullscreen-only-release.log`）。四台电脑现均已安装来源 `933a0ae`；两台 Mac 为 20261103.85，并分别保留原签名。实际 GUI PID 为 Studio 49792、Book 75449、HO5 2297682、cube 21500，均为 restored-original-gpui。安装后状态证据为 `studio85-installed-status.json`、Book 的 `book85-installed-status.json`、HO5 的 `ho5-fullscreen-current-installed-status.json`、cube 的 `cube-fullscreen-current-installed-status.json`。

85 的 macOS NAS 包已正式发布并公开校验，SHA-256 `7cf0405f5cab01973ca070ca4fc2fd2a7ec204b219f6472a59038eece8d5e173`（`studio85-nas-verified.json`）。NAS 的 Windows / Linux 旧条目保留，不能将各机新版安装概括成这些条目也已更新。

红米结束 cube 投送后，未操作 Disconnect 或重进页面，立即接入 Studio 的自有原生窗口，连接 `3663228716` 实际出画（`redmi67-sequential-incoming-actual.png`）。努比亚也通过普通来源选择直接拉到 Linux 唯一 Display 1，原始 PNG SHA-256 `aa246ad0220353ffbc9a5892f75bb7e02230768e445bb8b43be1c13a02cc2284`；Linux 新版安装后再次出画，努比亚 Disconnect 后 HO5 的 Sharing 自动归零，未手动刷新状态（`ho5-fullscreen-after-nubia-disconnect-status.json`）。

努比亚实际看到了 Studio 85 的发送控制窗，操作栏只保留 Files、New source 和全屏。双重投送测试创建的第二个发送窗与第一窗完全重叠；点击第一窗的全屏按钮被正确拒绝为“另一窗口覆盖”，没有向覆盖窗口注入输入。该场景不计作发送窗全屏 / 关闭的物理通过，也未放宽命中规则。原始图为 Book 的 `nubia67-studio85-sender-windowed-actual.png`。CUA 对新进程仍返回 -10005，未继续重复或通过禁止的应用画面绕行。

静止的自有 AppKit 窗口正常采集约 60 FPS，背景返回后也有实际画面，尚未复现先前特定应用零首帧。最初未带活动复用标志的 ADB 启动创建了重复 Activity，引入连接更换；已区别于产品验收，后续使用明确的现有 Activity 复用。没有基于“静止帧被丢弃”这一未确认假设修改媒体代码。

## 明确停止发送时关闭对应控制窗

此前 Stop this send、活动页停止及本机状态停止只取消了发送任务，留下结束的控制窗。现共用按请求 ID 停止的方法：取消对应发送并关闭它的展示窗；普通系统关闭预览仍保持后台发送。其它目标、管理窗口、文件上下文及输入命中校验保持原有所有权。现有真实 GPUI 夹具已扩展为同时打开两个发送窗：停止一个后仅其窗口关闭，另一路及文件上下文保留；停止全部后仅管理窗保留。

两项发送生命周期检查及 1,041 项签名发布回归通过（19 项发布检查跳过），证据 `sending-stop-closes-window-tests.log`、`sending-stop-closes-window-release-final.log`。

该修复 `c810e13` 现已正式构建并兼容安装四台电脑，两台 Mac 为 20261103.86；原签名、成员、配置与既有启动方式保留。实际 GUI 为 Studio 67996、Book 7438、HO5 2330547、cube 20568，均核对到同一源码及 restored-original-gpui。状态记录分别为 `studio86-installed-status.json`、Book 的 `book86-installed-status.json`、HO5 / cube 的 `*-stop-current-installed-status.json`。

努比亚 67 通过普通来源选择拉取 Studio 的唯一发送控制窗，避免额外定向投送控制窗造成重叠。实际点击全屏后为 1920×1080，系统标题栏按钮隐藏；再点可见恢复按钮，回到 1280×833（内容 1280×801）。红米同时接收原管理窗口，画面继续。随后努比亚实际点 Stop this send，该控制窗关闭；Studio 仅剩管理窗，Sending 为空、Sharing 为零，红米自动返回 Devices。努比亚收到来源结束后仍可选择其它来源；最后明确断开了这条自有测试连接。物理图在 Book 的 `nubia67-studio86-sender-fullscreen-actual.png` / `nubia67-studio86-sender-restored-actual.png`，状态为 `studio86-sender-after-first-fullscreen-tap.json`、`studio86-sender-after-restore.json`、`studio86-after-physical-stop-current-status.json`。

Linux 20,263,013 字节和 Windows 51,521,864 字节的正式运行包均通过 Weavelo 本身发送至 Studio；接收端另行计算 SHA-256，分别为 `85444faca57dddb9f7adbe5b0d65aa5360c140672126df86507ca376c455b66e` 与 `1998b4071ab14994d46cd6c14c6a876acb331877a2885ca5739d85aa89ca86f8`，与正式来源包一致。普通发送记录的 `remote_content_verified` 为 false，没有将其当作接收端独立校验；独立证据为 `ho5-current-package-native-receiver-independent-sha.json`、`cube-current-package-native-receiver-independent-sha.json`。

三个桌面 NAS 条目现均已正式发布到同一 `c810e13` 并公开下载校验（`native86-nas-verified.json`）。macOS 为 `Weavelo-desktop-macos-arm64-20261103.86-c810e13.zip`，SHA-256 `90e9c319dac513b810d5eecf4cb4859384138dd21ed589479ad0cfa82c48a11a`；Windows / Linux 为对应的 `*-c810e13.zip`，使用上述独立校验过的原包。初次临时目录因 `/var` 链接路径在上传前被拒绝，解析自有目录真实路径后正常发布，未绕过签名检查或修改已发布字节。

## 原生最小化恢复对照

新增可独立运行的 `native_window_activation` 示例，只创建一个 400×180 的原生窗口：先显示、最小化，再调用产品共用的 `activate_window` 并读取真实平台可见性和焦点。不启动产品配置、网络、录屏或输入注入，也不作为产品外观验收。当前未改动的 macOS 后端实测为 true → false → true，恢复后有焦点（`native-activation-baseline.json`）；因此没有凭源码表象增加 `deminiaturize` 或修改媒体路径。最小化后找回产品会话仍需结合实际活动入口验收，不能用这个夹具替代。

## 原生托盘的上下文动作（U5，2026-11-03）

macOS 菜单栏、Windows 托盘和 Linux D-Bus 菜单共用当前设备与会话动作，包含设备的拉取 / 投送 / 文件、接收窗口的打开 / 暂停 / 结束、发送任务的重开 / 停止以及活动、传输和设置入口。菜单打开时使用后台缓存，不在系统 UI 线程等待 IPC，也不随视频帧刷新。动作保留窗口实体、设备、来源及 GUI 代际；设备排序变化不会复用旧菜单 ID。关闭发送预览后保留来源名称和发送任务，重新打开继续使用原任务。

7 项针对性检查通过：设备排序后的旧 ID、来源和代际绑定、失效 / 过期 / 重复动作、两路窗口的单独结束、关闭并重开发送窗及文件上下文保留、真实协议回环接收连接中的暂停 / 恢复。断开窗口不接受暂停，打开已有流不创建第二条连接。现有发布回归 1,041 项通过、19 项跳过，启用签名集成（`native-menu-tests.log`、`native-menu-release.log`）。随后仅删除两项未使用的测试导入。

以上是源码检查结果；当前安装仍为 native86，三个平台的新菜单尚待正式构建、安装和真实点击验收。此次 CUA 读取 SystemUIServer 仍返回 -10005，未通过其它 API 绕行本机 UI 控制。Android 页面 / 多会话适配 U6 尚未完成；此前特定应用零首帧与全方向音频、控制、性能的未知项保持独立记录。

U5 后续交付：三平台正式构建完成，四台电脑均已兼容升级至 `b7949c9`（两台 Mac 为 native87），各自原有签名、配置和启动入口保留。运行进程与新菜单 IPC 已分别核对。三个当前原生安装包已由正式入口发布到 Weavelo 域名，公开下载字节验证通过（`native87-nas-verified.json`）。Linux / Windows 包经过真实 Weavelo 文件发送，Studio 独立校验接收 SHA 和归档内编译身份；常规发送端的 `remote_content_verified=false` 未作为失败判据。

红米和努比亚均通过普通来源选择收到新版 HO5 的实际 Display 1 画面。努比亚长按托盘图标打开了真实 KDE 菜单；随后直接读取原生菜单，不强制刷新，确认已自动更新为 Sharing 1，设备、传输中心入口存在。两部手机断开后 HO5 共享数归零（`native-menu-ho5-after-both-phones-disconnect.json`）。红米的自动触摸注入被手机系统拒绝，未修改手机安全设置，也未将其归为产品输入失败；努比亚上的同类注入可用。Windows 首次升级前检查失败来自测试脚本误传 GUI PID，改用实际后台 PID 后完成升级，未修改产品逻辑。

## 手机操作栏不遮挡视频（U6 局部，2026-11-03）

实际努比亚画面显示：浮在视频右下角的键盘按钮遮住了远端托盘菜单。现把键盘开关和缩放复位移至该路视频之外的紧凑操作栏，视频与触控仍共用现有 RemoteViewport、可见矩形和输入归属；只读来源不显示不可用的键盘按钮。不改变原生媒体、IME 提交及拒绝后显式恢复逻辑。Android 版本码为 68；编译、Lint、76 项现有单元检查与 1,041 项签名发布回归通过（19 项发布检查跳过，`android68-toolbar-checks.log` / `android68-toolbar-release.log`）。待原身份签名升级两部手机后，继续验证无遮挡菜单、键盘显隐及坐标。

U6 的多设备会话池和活动页面仍待实现，不能把本次操作栏调整称为 U6 完成。macOS / Windows 原生菜单的真实点击，以及全部方向音频、控制和性能仍按原未知项分别验收。
