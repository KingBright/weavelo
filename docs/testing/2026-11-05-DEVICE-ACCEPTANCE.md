# 设备验收与接续记录 · 2026-11-05

最新一轮图标与取消修复见文末「桌面138 / Android91」。历史记录保留各自版本，不作为新版本的完整实机验收。

## 已交付

截至 2026-11-06，四台电脑已正式升级本轮桌面138（源码 `8f98bd6`，Mac构建号 `20261106.138`），两部原签名手机均已安装 Android91（源码 `6eb30da`）。三平台桌面包与 Android APK 均已发布并核对公开下载字节。Mac Studio 与 MacBook 保留各自签名；设备身份未改变。努比亚在此前测试中被强制停止后，系统移除了 Weavelo 的无障碍启用项，正在等待本人恢复；没有自动修改系统授权。下文历史验收须按各自版本读取，最新详情见末尾。

- 来源选择器：固定列、图标与左对齐，同应用分组缩进，名称筛选及稳定排序；桌面实际签名程序与两部手机界面已检查。目录没有可靠父窗口关系，未伪造进程树。预览仍按 [选择器方案](../plans/SOURCE-PICKER-POLISH-20261105.md) 等待用户选择。
- 文件：四台电脑的全部 **12 个发送方向、12 个 Fetch 方向**通过，每个文件 262,181 字节。接收端独立读取并计算 SHA；没有用发送端完成通知代替接收内容验证。Fetch 使用实际完成记录的 `shared.received_path`。
- Mac 原生 CUA：Studio 接收 Book 的专用应用窗口，135 个解码帧、零错误；原生观察、连续快照、缩略区域、点击、按键、滚动通过。Book 独立记录恰好一次点击、一次 a、一次滚动及释放；中心落点误差小于万分之一。旧引用返回 `stale_snapshot_ref`；查看端转到自身管理窗口后，等待中的后续 z 未执行。
- 主动投送与音频：Book 主动投送专用应用窗口到 Studio，无用户确认步骤；约九分钟累计 16,005 个视频帧、零解码错误，音频输出回调记录 27,347,670 个非零样本、零错误。原生观察图片通过。停止投送后接收窗口退出；测试音源已停止。仅证明软件音频链路，未做物理听感验收。
- Android：努比亚 NX741J、红米 2409BRN2CC 都已实际显示 Studio 的独立应用窗口；打开设备不会先拉桌面，串流横屏、HUD 收起，结束后回到竖屏设备页。两部手机安装后的 base.apk 独立 SHA 与正式 APK 一致。
- 努比亚输入：直接点击准确触发一次；临时百度输入法提交的 `NU80ABC` 准确出现一次。测试后恢复微信输入法，并独立核对恢复结果。

上述文件证据位于任务目录的 `matrix121-all-desktop-summary.json` 和各端 `matrix121-*` 记录。CUA 证据为 `cua121current-real-book-window-proof.json`、`cua121cancel-real-book-window-proof.json` 及 Book 的原生接收事件记录。两部手机图片为 `android80-{nubia,redmi}-studio-owned-window-actual.png`。发布证据为 `native121-nas-verified.json` 与 Book 的 `android80-nas-publication.log`。

## 软件画面延迟测试

[VisualClockFixture.swift](../../scripts/acceptance/VisualClockFixture.swift) 在 Studio 显示 60 Hz 时钟和冗余像素标记；Book 拉取该窗口，Studio 再拉取 Book 的这一个观看窗口。已经实际同时出画，返回图片中能识别原时钟；全程不拉取其它应用。时钟原点和采样端在同一台电脑，避免另建跨机校时逻辑。

[measure_visual_clock.py](../../scripts/acceptance/measure_visual_clock.py) 通过现有原生 CUA 观察读取，使用 [decode_visual_clock.swift](../../scripts/acceptance/decode_visual_clock.swift) 解码标记。编译两个 Swift 工具后，为 Python 显式提供正式程序、解码器、已连接返程窗口的设备／来源／连接标识及输出目录；默认采样八次，当前 PNG 原位替换，仅保留小型样本记录。两条测试订阅须在同一轮建立并在 120 秒租约内完成；旧窗口标识与过期租约不得重用。

第一轮已取得八个样本，但观察请求约耗时 250 ms，所得 P50 区间约 0.5–252.6 ms、P95 区间约 86.3–336.2 ms，不能据此判断 25 ms。当前修复给观察结果增加所持帧的本机 `decoded_at_utc_us`，在导出前投影时间；鉴权、有效期和输入仍使用原有单调时钟。工具将记录从源窗口开始绘制到返程解码纹理的时间，另保留观察请求区间供诊断。正式 122 的八个样本已完成：源窗口开始绘制至返程解码纹理的 P50 为 138.6 ms、P95 为 1,083.5 ms。另一次采样取到在请求时已经至少 1,850.5 ms 前解码的帧；工具同时记录帧龄，避免只看解码耗时而漏掉画面停滞。长尾及陈旧帧仍需定位，尚未确认是否与窗口活动、可见性或调度有关。样本数仅八个，不能宣称稳定的总体 P95。

这是两段媒体路径的软件测量。没有测物理屏幕呈现，不能除以二宣称单程 glass-to-glass；当前链路使用 Relay，不能代替局域网直连验收。

## 仍需接续

- HO5 实际桌面曾处于 KDE 锁屏：真实截图为黑色，即使有解码与提交记录也不算图像通过。需机主解锁后继续实际画面及输入验收，不能绕过锁屏。
- Linux 编码：130 已完成 4K GPU 输入到 1080p 编码的 1,200 帧原生路径测试，平均 6.532 ms、最高 16.205 ms，详见末尾。真实 PipeWire 桌面长时间分布仍待解锁验收；历史实际双路测试仍有超过 20 ms 的帧，不能据合成测试宣称日常表现已全部达标。
- 微信输入法：努比亚恢复微信后仍未输入测试文字；此前普通系统搜索也存在一像素键盘现象。百度对照通过不代表微信已修复。
- 红米输入：已通过系统允许的界面操作选来源并显示画面。部分自动化失败是多词参数引用错误，已修正测试调用；真实用户触摸和键盘尚未实机确认，不能把测试工具失败认定为产品输入失败。
- Android 普通新屏幕共享仍需要机主的 MediaProjection 系统确认；已有相机权限不能代替屏幕会话。无障碍权限按真实系统状态记录，不自动批准。
- Linux／Windows 原生 GPU 图像的 CUA 导出及远端真实 AX 语义树尚未接入；当前明确返回不可用，不生成虚构节点。
- 全设备音频、摄像头、通知／控制往返、主动与被动多窗口组合、旋转／IME／缩放下输入，以及长时间稳定性仍须逐项完成。文件矩阵通过不代表这些媒体项目已通过。

升级接续：122 的原生程序已在 Studio、HO5、cube 兼容安装；Book 打包成功，升级前检查遇到其正常 Dock 图标子进程。升级器现仅允许已认证 GUI 的直接子进程、精确 `--macos-dock-tile` 参数作为该 GUI 的辅助进程；仍要求正常退出后所有实例结束。其它父进程或参数不放行。45 项升级检查、1,046 项签名发布回归通过（19 项跳过），Book 已通过精确复用暂存正式包完成安装，原 GUI 的 Dock 子进程随正常退出结束。

窗口标题按用户最新要求统一为“应用名 · 主机名”：接收、画中画、选择来源及投送窗口不再附加产品名或模式标记，Dock 菜单跟随相同标题。现有 Dock 编码和投送窗口生命周期检查通过；1,046 项签名发布回归通过（19 项跳过，34.748s）。正式 123 的 Mac 与 Windows 原生窗口标题已检查，均为应用名与主机名。

## ChatGPT 窗口与首帧超时排查

Book 同时接收 Studio 的两个 ChatGPT 来源，复现出明确差异：真正的聊天窗口（18227，layer 0）累计 1,478 帧、零解码错误并有画面提交；名为 ChatGPT 的另一个来源（18235）始终没有采集或编码帧。原生 SC、CG 与 AX 对照证明，后者是未显示的 layer 3 辅助面板，没有对应的普通 AX 应用窗口。它被错误地列入了应用来源，不能据此判断 ChatGPT 应用整体无法采集。

修复同时作用于目录和实际采集入口：隐藏的非普通层级辅助面板不再出现；旧选择若已失效，明确返回原因。仍保留其它 Space 上的普通窗口与正在显示的浮动面板。来源分类回归 1 项、接收端首帧及多窗口生命周期 7 项、公共投送逻辑 14 项通过；1,046 项签名发布回归通过（19 项跳过，36.794s）。正式 124 的原生目录中已没有辅助面板 18235，只保留实际聊天窗口 18227；两台 Mac 实际运行的编译身份均核对为 `5863a5b`。

Book 正式 124 实际接收该聊天窗口，63 个解码帧、零错误，有画面提交；该次接收只包含指定窗口，没有默认桌面。通过产品原生菜单的 Open 操作展开接收窗口后，取得并检查 2,784×1,888 的系统窗口截图，实际聊天内容、标题 `ChatGPT · MacStudio` 和远端应用图标均显示正常。最初截图只取到 Stage Manager 缩略图，未将它当作正常窗口验收；已用完整窗口截图替换。专用接收租约正常停止或到期，没有关闭其它用户会话。

用户截图中的 CUBEX 错误对应本轮标题验收的投送请求 `59912-1793848236464738000`。接收端连接成功但未在 30 秒内回报首个画面提交，自动失败；没有人工“拒绝”步骤。提示改为接收端投送失败和视频启动超时，不增加确认步骤，也不放宽 30 秒上限。使用新的、仍存活的相同测试程序窗口复测，Windows 持续约四分钟，12,362 帧、零解码错误，有原生画面提交及正确标题；发送端状态为 Sending。测试已定向停止，专用源窗口已退出。这次复测不能证明前一次失败的根因，未虚构 Windows 解码器修复。

正式 124 另按原失败前的顺序复测同一来源：Studio 先拉取专用 Book 窗口并显示 29 帧，停止后原连接的采集工作数、原生流与待停止数均为零，原生停止完成数为 2、错误数为零；随后 Book 将同一窗口推给 Windows，收到 173 帧、零解码错误且有画面提交。未复现采集停止／复用失败。原失败来源当时仍存活，不能归因于测试窗口到期；原次超时根因继续保留为未确认。投送及专用测试程序均已定向停止。

完整证据保存在任务目录的 `chatgpt123-*`、`bug124-*` 和 `native124-nas-verified.json`；Windows 复测原生记录位于 cube 的 `title123-windows-native-and-desktop-inspection.json` 与 `bug124-reuse-receiver-current.json`。临时跨机包在公开发布字节核对后按精确 SHA 清理。

构建效率接续：Book 的远程命令达到工具默认 600 秒时限，原生编译子进程仍在运行。确认它完成后，以更长命令时限恢复正式打包与安装，未并行启动另一编译器。恢复过程重新生成了 `build_identity_gate.py::BuildInputs` 的固定临时身份文件，同一源码身份又触发应用编译；这条重复编译路径仍需优化，不能把本次打包成功当作迭代效率问题已解决。

## macOS 128 的单例与 Dock

MacBook 的重复主图标来自菜单栏后台。旧版只有一份界面和一份后台，但 macOS 重新打开应用时把后台也设为普通应用。修复后，后台收到打开请求时恢复菜单栏模式并唤醒原有前端。

两台 Mac 已各用原签名安装 `20261105.128`，实际运行源码为 `0c56213`。连续两次通过系统入口打开均复用同一个前端。空闲时通过前端退出请求关闭界面，后台进程及其会话标识保持一致，Studio 和 MacBook 分别仍能列出 42 和 56 个共享来源。重新打开只创建一份新前端。

两台 Mac 分别拉取对方的 ChatGPT 独立窗口，Studio 收到 44 帧，MacBook 收到 26 帧，解码错误均为零，均已提交画面。测试结束后对应 Dock 辅助进程退出。Studio 的实际 Dock 截图显示底部名称 ChatGPT、设备角标 MBPM2MAX。MacBook 启用了 Dock 自动隐藏，未把隐藏窗口的缓存图当作实时画面。

独立验证发现，系统在注册会话 Dock 项时会异步载入签名包图标。200 毫秒内更新仍可能被覆盖，600 毫秒后更新成功。正式版用单次 750 毫秒的原生补绘应用当前图标。Studio 正式包在仅送入一次图标数据时已验证正确显示。这项调度只涉及 Dock。macOS 报告的系统名称仍为 Weavelo。

签名发布回归运行 1,046 项，跳过 19 项。macOS 128 已通过 NAS 正式发布并独立下载校验。实机证据位于任务目录的 `dock128-lifecycle-acceptance.json`、`dock128-stream-acceptance.json`、`dock128-native-chatgpt-dock-detail.png` 和 `dock128-nas-verified.json`。

## 高清外观与 Android 标志：开发验证

本轮实现统一协议 8 的按需来源外观请求：保持轻量目录，选中并建立订阅后读取高清应用原图和设备角标。回包绑定请求、订阅与来源；切换、结束和重复回包均有回归。外观读取失败不会结束视频。桌面使用真实机型或本机选择的自定义标志，Android 增加手机标志、型号预览、导入设置和动态最近任务图标。

本地已通过协议 60 项、图像资产 6 项、会话 35 项回归；Android 发布服务 17 项在 release 配置下通过。Android Kotlin 编译、桌面应用与 JNI 编译检查通过。签名发布回归 1,047 项通过，跳过 19 项。调试发布服务曾触发默认测试线程栈溢出。系统 LLDB 定位到大型测试协程：多次接收调用各内嵌 64 KiB 数据报缓冲；将测试的接收与连接任务存放到堆后，默认线程栈下 17 项全部通过，未提高线程栈或改变生产接收缓冲。可选外观消息单独放入堆，公共控制消息恢复为 120 字节；协议回归确认序列化字节不变。

重复构建优化保留现有临时输入清理和来源核对，只在现有元数据中保存已验证内容的时间戳。相同输入重建不再无故使 Cargo 缓存失效；内容变化仍触发编译。62 项身份构建检查通过，其中 1 项需显式原生集成环境而跳过；上述全量签名回归已启用原生集成。

四台电脑均已正式安装 `20261105.129`，两部手机均安装 Android 81，源码为 `cc8c631`，保留各自签名。两台 Mac 相互接收 ChatGPT 单应用，分别 36／26 帧、零解码错误并提交画面；Studio 实际 Dock 已检查，Book 的 Dock 自动隐藏未算作可见截图验收。两部手机实际拉取 Studio 的正常 ChatGPT 窗口并显示内容，横屏 HUD 含真实应用图标，结束后均回到竖屏 Devices。两部安装后的 base.apk SHA 均为 `9ec9b1b47e9de693785165746b6883420461e3726acb1af37526080dd709da4e`。Android 最近任务元数据已启用动态图标，但系统最近任务画面、自定义标志跨端同步仍需实机检查。129／81 的 NAS 发布尚未进行。

HO5 当前仍锁屏。红米的 Weavelo 无障碍服务已经启用，当前没有运行中的屏幕共享授权会话。部分自动化的 Killed 输出已定位为未正确引用多词参数后抛出的异常；不能一概归因于系统安全限制。实际用户输入与微信输入法问题仍按独立验收记录。

## 正式 130 与实机接续 · 2026-11-06

四台电脑的实际运行身份均为 `1843578 / 20261105.130`。Windows 的一次打包因外层 PowerShell 把普通编译 stderr 当成异常而中止；确认编译器结束并验证三个精确临时输入的归属与内容后清理，改由 Python 收集日志，原打包入口通过。没有改变系统或用户执行策略。

两台 Mac 再次通过主入口单例、关闭界面后后台保留、重新打开一份界面的检查。Book 关闭界面期间仍列出 53 个来源；Studio 为 41 个。Book 实际接收 Studio ChatGPT 窗口 22 帧、Studio 接收 Book 11 帧，均零解码错误、有画面提交；各自停止后 Dock 辅助进程退出。另把 Book 的设备标志临时设为一个 253 字节自定义 PNG，Studio 的实际 Dock 显示了该标志与 ChatGPT 原图；随后按字段精确恢复原有设置，没有覆盖其它偏好。

Studio 同时向 Windows 主动投送 ChatGPT 和 DeepSeek Harness 两个独立应用窗口，持续超过五分钟，分别累计 16,469／16,145 个解码帧，解码错误均为零。已检查 Windows 实际窗口截图，均显示相应应用内容、应用图标与“应用名 · MacStudio”标题。停止该投送后，接收会话清空，两个接收窗口退出，仅保留主界面及系统 IME 辅助窗口。截图证明可见内容与窗口身份，不代表完整 DPI／全屏／输入矩阵通过。

HO5 仍处于 KDE 锁屏，未绕过。使用当前 `host/native/linux_encode.c`，一次创建 3840×2160 GPU 输入，连续平移裁剪并以 1920×1080、60 Hz 编码 1,200 帧：平均 6.532 ms，P50 6.614 ms、P95 7.746 ms、P99 8.420 ms、最高 16.205 ms；没有超过 20 ms，2 帧超过 10 ms。VPP 平均 1.056 ms，编码平均 5.462 ms。这是 DMA-BUF／VAAPI 原生编码测试，不含真实 PipeWire 采集及物理显示，不替代解锁后的产品性能验收。

软件时钟重新使用持续存活且可见的测试窗口，Studio → Book → Studio 两段 Relay 路径取得 12 个完整样本，P50 91.474 ms、P95 189.413 ms，本次观察时持有帧的额外帧龄为 0 ms。测试租约和专用时钟均已结束。这仍不是物理 glass-to-glass，不除以二推算单程，也未证明 25 ms 局域网目标。前轮长尾的产品根因仍未确认。

发布证据为 `native130-nas-verified.json`：三个公开包分别重新下载核对 SHA。图像为 `native130-cube-ChatGPT.png`、`native130-cube-DeepSeek-Harness.png`；停止证据为发送端 `native130-two-windows-sender.json` 和 Windows 原生窗口检查。Linux 原始数据位于 HO5 的 `native130-dmabuf-bench.{c,csv,json}`；软件时钟为 `visual-clock130-fresh/report.json`。当前签名发布回归 1,047 项通过、19 项跳过；协议 61 项、会话 35 项、Android 发布服务默认调试线程栈 17 项通过。

Android 82 已在努比亚 NX741J 与红米 2409BRN2CC 兼容安装，两台安装后的 base.apk 独立 SHA 均为 `1a4c9a4159ad4fb41f65a4a441f7283f062996d0ab6382b7725938a7628b1095`，与正式 APK 一致。Book 正式发布入口完成 NAS 上传、公开元数据和 APK 下载校验。证据为两台的 `android82-*-installed-proof.json`、Book `android82-nas-publication.log` 和 Studio `android82-public-manifest.json`。82 的界面复测仍等手机完成另一任务的无线 ADB 验收；安装没有抢占系统页面，81 的画面验收不冒充 82 新的完整实机验收。

130 已另外验证 Studio 拉取 MacBook 已授权的 FaceTime 摄像头：102 个解码帧、零错误、有原生画面提交，窗口名为“FaceTime高清相机 · MBPM2MAX”，定向停止成功。该记录不替代实际摄像内容或多设备摄像头切换验收，证据 `native130-book-camera-proof.json`。

用户于 11 月 6 日明确：20 ms、5 ms 的低延迟目标有同一 Wi-Fi、良好链路的前提，不能要求跨网或弱网达到相同数字；编码阶段与端到端指标分别记录。新增手机多点直触需求及具体方案见 [局域网低延迟与手机多点直触](../plans/MOBILE-MULTITOUCH-LATENCY.md)，当前手机控制仍会映射为鼠标，尚未交付真正多点。

## 11 月 6 日：协议 9、多点触控与 131 / 83

产品源码 `42c110a8859bf2129336cfcab1f3ae74fcc79c72` 已推送；四台电脑正式安装桌面 131，两部手机正式安装 Android 83，保留各机原签名。两台手机安装后独立读取 base.apk，SHA 均为 `79e0497f0174573ab83e7ed88b91e6afaeeb3bf0d7ed3290e274c9f02c5192d6`。桌面三个公开包和 Android 正式发布均通过公开下载校验。证据为 `native131-nas-verified.json`、各机升级记录、`android83-*-installed-proof.json` 和 Book 的 `android83-nas-publication.log`。

本轮已接入同帧原生触点、实际能力协商、触点所有权与独立释放、过期和断线取消、相邻纯移动合并，以及手机的多点模式和本地“调整画面”。旧画面回调不能覆盖新手势的几何。协议 64、桥接 102、公共输入 12、Android 80 项单元测试及 lint 通过；签名发布回归 1,047 项通过、19 项跳过。

努比亚通过正式安装应用上的全部 17 项手势 instrumentation 测试，包括五指身份、双指送往远端、本地调整、边界、取消与旧回调隔离。测试改为使用真实 MainActivity 承载临时手势夹具，不再依赖只存在于调试构建的测试 Activity。原始证据为 Book 的 `multitouch-nubia-viewport-all.log`；这验证发送端手势映射，不替代接收端原生注入或游戏体验。

红米的 Android 83 正式安装通过；系统以 `INSTALL_FAILED_USER_RESTRICTED` 拒绝独立测试 APK，未修改系统安装策略。随后通过正式界面实际拉取 Studio 单应用窗口，横屏画面与 HUD 已目视核验，证据 `android83-redmi-window.png`。红米的五指 instrumentation、两手机接收端同时按住／点击以及游戏和良好同 Wi-Fi 延迟仍待验。

用户明确要求模拟控制归属 Weavelo 移动端。自动整屏及独立后台应用接续 [自有显示与控制方案](../plans/ANDROID-OWNED-DISPLAYS.md)；Octendra 仅为消息触达／维护通道，不成为运行时控制依赖。

## Android 84：真实多点注入与断开回执

产品修复 `b592e10`、`a37c4ff` 均已提交并推送。努比亚与红米原位升级 Android 84，保留原签名；两台安装后独立读取 base.apk，SHA 均为 `55841a28bffaf5e9cc85a90f5e24245d8bdf054ce4668337f4af347e97e0190d`。NAS 发布和公开下载校验通过。四台电脑维持协议 9 的正式 131，本轮修复位于 Android 接收服务。

83 的原生接收测试确实失败：第一指 DOWN 后约 53 ms 收到 CANCEL。Android 的 `MotionEventInjector.prepareToContinueOldGesture` 要求续接第一步只包含原先按住的触点；新指同在 t=0 加入会取消整组手势。84 将新增触点推迟 1 ms，保留原触点连续笔画及同一结束时刻。另修复 Android 收到 Close 后没有返回 Closed 的问题：结束订阅、撤销输入租约并等待所持有连接任务退出后，在撤销会话密钥之前发送完成回执。

努比亚上的正式 84 通过实际接收端测试：8 个原生 MotionEvent，顺序为 DOWN、MOVE、POINTER_DOWN、MOVE、MOVE、POINTER_UP、MOVE、UP；同时存在两个触点，单独释放第二指后第一指继续移动，没有 CANCEL，首尾坐标正确。发送端同时获得 `cleanup_confirmed=true`。测试耗时 1.425 s，nonce 为 `e7b14038-d409-4ccb-b8a8-0742f96a92bf`；证据在 Book 的 `multitouch-nubia-native-events.json`、`multitouch-nubia-native-instrumentation.log` 和 `multitouch-native-sender.log`。此测试用既有鉴权及实际无障碍输入服务，只向专用前台触点页送输入，没有申请或模拟屏幕采集授权，不代表手机到手机视频、手游或端到端延迟已验收。

Android 80 项单元测试、lint、真实宿主测试 APK 编译通过；连接清理相关 18 项测试通过，发布回归 1,047 项通过、19 项跳过。设备 instrumentation 使用 `am instrument --no-restart` 并保留无障碍服务；普通 instrumentation 强制停止应用会干扰已有系统绑定。测试夹具等待发送端写回本次 nonce 后才销毁发布器，避免测试自身抢先关闭连接。红米独立测试 APK 的安装限制仍未解除，不能以努比亚结果代替其原生多指验收。


## 手机自主整屏与点击修复 · 2026-11-06（桌面135 / Android86）

四台电脑已通过各自原有签名／安装入口升级到本轮源码 `18424ed`，两部手机已兼容安装 Android86。两台安装后的 `base.apk` 独立 SHA 均为 `aa8d722b7c165ed734b10f25ec13bd32d4a523647c9e67916eb8e6ba08977842`；原有设备身份、签名与无障碍授权保留。Linux 首次编译漏传既有 FFmpeg SDK 路径，补齐已存在的 SDK／Opus 环境后编译、正式打包和安装通过，没有新增依赖或修改系统设置。

手机自主整屏由 Weavelo APK 内的本机辅助进程提供，使用已经授权的调试连接启动，媒体持续经手机原生 Surface、GPU 与 MediaCodec 进入现有鉴权会话。启动命令退出后辅助进程持续存活。Octendra 不参与画面或输入的运行时转发。两台 Mac 已实际显示手机画面：Studio 同时显示两部手机，Book 通过 LAN 显示努比亚；均有可见截图及零解码错误证据。这不代表重启后无条件恢复、自动音频或独立后台应用已经实现。

修复了 Mac 黑屏的具体原因：Android 的 HEVC 矩阵值5在 VideoToolbox 输出中表现为 `YCbCrMatrix#5`，原渲染器拒绝该明确的 BT.601 值，界面却已确认画面提交。现在识别明确的5／6值，并在解码返回前验证颜色可呈现；未知值仍报错。原始压缩流、FFmpeg、原生 VideoToolbox 像素和实际窗口截图均已交叉核对，未通过猜测矩阵隐藏错误。

点击失败也有原生前后对照：85版收到鼠标“移动、按下、原地移动、抬起”后，Android 只记录 `ACTION_DOWN`，60秒内没有抬起。原因是 Android 拒绝没有像素变化的续接手势，导致当前输入链中断。86版只跳过同一物理像素内的移动，保留原手势及所有按下／抬起；多指的全静止帧同样处理。相同实机测试在1.084秒内通过，记录 DOWN→UP，发送端确认连接清理。旧／新 nonce 分别为 `fa1754b2-04bf-4324-b92e-1286ff09db31`、`e1aed76f-2685-40fe-a901-be80a94ba8f3`。

Studio135→红米86已实际验证普通鼠标点击切换页面、按住拖动滚动内容，以及键盘提交 `abc你好🙂`。手机原生输入框显示完整文本，没有重复；控制保持开启、松开后无残留按键、零解码错误。键盘弹出后的画面仍保持物理屏幕坐标，取消测试输入不会提交网络邀请。此结果不代替完整多指游戏、所有平台输入或完整手机互控矩阵。

相关检查：Android81项单元测试、lint、原生测试编译通过；签名发布回归1047项通过、19项跳过；Mac颜色针对性测试3项通过。所有代码已提交并推送 main。Android86与桌面三个包已发布并核对公开下载 SHA；桌面发布最后读取元数据时发生一次连接超时，随后只重新核对原始预期元数据的精确字节，没有重发或改写版本包。

证据：`android86-*-installed-proof.json`、`android86-nas-publication.log`、`native135-nas-verified.json`；Book `mouse85/86-nubia-native-events.json` 与 instrumentation 日志；Studio `gui135redmi86-settings/drag/text/cancel-*`；Mac图像 `native132-nubia-visible.png`、`native132-redmi-visible.png`、`native132-book-nubia-visible.png`。原始材料保留在对应机器 `.rust-storage/validation-logs/weavelo-gui-20261102/`。

继续项：手机之间的真实串流、多点触屏与输入；独立虚拟显示中的单应用及其显示绑定输入；自动恢复与音频；HO5解锁后的实际 PipeWire 性能；完整六设备摄像头／音频／通知组合。此前白板、教学模式及剩余界面问题继续按已保存规划推进，未标记为完成。

## 独立应用与状态恢复 · 2026-11-06（桌面137 / Android89）

四台电脑和两部手机升级、正式发布均已完成。Android89 两端安装后的 base.apk 与公开下载 SHA 均为 `7ec409cf46467f4d15aa68f1d855d66f06dd62aa60d86c6703a9fc03f16cfd23`。桌面公开包及来源身份见 `native137-nas-verified.json`，Android 见 `android89-public-download-verified.json`。所有产品包来自 `3e27bc3`；独立测试工具修复已推送为 `5315231`。

- 努比亚独立应用：计算器、原生输入测试窗口均实际显示，零解码错误。系统活动记录同时确认物理屏幕为启动器、显示47为独立测试应用；两者不共用显示焦点。实际截图为 `android88-nubia-app-reject-native.png`、`android89-nubia-app-keyboard-native.png`，Book 记录为 `android89-nubia-independent-app-activities.txt`。未将辅助进程启动成功当作实际出画。
- 失败状态：Studio137 实际选择失效来源后记录错误，再选择手机整屏时清除旧错误并进入接收状态，结束后所属租约释放；证据 `native137-retry-state-proof.json`。错误记录按设备和配对网络隔离，设备卡片不会保留上一来源的历史失败。
- 来源目录：包安装／卸载事件触发更新，空闲的248项应用目录不再20Hz序列化；只轮询当前已订阅来源，暂停仍保留应用任务。来源使用实际应用名称。手动点共享会保留当前已获得的无障碍控制能力。
- 输入限制：计算器拒绝了文本提交，标准编辑器尚未完成端到端输入验收。测试 APK 独立启动曾缺 Kotlin 运行库，已改为平台 Java 并真实出画。努比亚被本次测试强制停止后丢失无障碍启用项，已告知用户并等待本人重新启用；不得自动写回授权。红米原授权仍在，独立测试 APK 却被系统拒绝安装，不重复相同失败，也不以其它手机结果代替。

本轮82项 Android JVM 检查、lint、测试 APK 编译通过；35项 publisher 检查、6项设备状态检查通过；签名发布回归1047项通过、19项跳过。测试工具修复另经实际 APK 编译、触点发送工具编译及同一发布回归验证。全套当前版本文件／摄像头／音频／通知矩阵、独立应用文本／多指／旋转、高清图标、重启恢复和同Wi-Fi低延迟仍未验收完成。

操作接续：仅用包事件刷新目录，不强制停止手机产品；instrumentation 使用 `--no-restart` 与 `FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES`。对已在运行的应用不迁移主屏任务。恢复努比亚授权后，从标准测试窗口继续，先确认原生接收记录，再扩展实际应用。所有临时观看租约均已关闭。

## 手机应用图标与原生取消 · 2026-11-06（桌面138 / Android91）

Android91 产品源码为 `6eb30da`，桌面138为 `8f98bd6`，均已推送 main。两部手机均已原签名安装91；Studio、MacBook、HO5、cube 均已完成138兼容安装。Android与三平台桌面包均已通过既有发布入口上传，并核对公开下载 SHA。源代码提交、实际安装和真实画面证据分别记录。

两台手机各自拉回安装后的 base.apk，均与正式 APK SHA `bec671ee9151c662e46d74320ce5abf827f03f6363cf6b70cbebf7af1bd7ce2d` 一致（19,662,659 字节）；保留原签名和升级前实际无障碍启用状态。努比亚尚缺 Weavelo 无障碍授权，不能把捕获成功解释为完整控制可用。

- Android图标：90版的 Skia PNG 被原生严格校验拒绝，实际网络结果 application=null。91仅规范化自身生成的PNG，保留压缩像素和CRC；同一努比亚原生验证由失败变为 `OK (1 test)`，无障碍列表未改变。网络探针实际收到256×256、46,313字节的计算器图标及Phone标识，8帧视频、连接清理成功。
- 桌面图标：手机SVG固有128像素被GPUI栅格化为512像素，超过原生窗口及Dock上限。修正为与其它设备预设一致的32像素固有尺寸，保留viewBox与矢量路径。回归测试修复前在Phone上失败、修复后全部5项通过。
- 真实显示：Studio138实际显示努比亚计算器，窗口标题为应用名和设备名，标题栏及Dock均显示真实应用图标与手机角标。Dock激活对应窗口；结束后所有会话和租约为空、Dock辅助进程退出，只剩主界面。没有把主界面的空viewer容器误报为残留串流。
- 原生取消：90版将多点取消从逐指抬起改为一次ACTION_CANCEL。努比亚原生事件构造测试2项通过，包含原触点延续；这不是手机游戏或全网络多点验收。
- 文件补验：桌面137四台电脑完成全部12个发送方向和12个Fetch方向，每个文件262,181字节，24份接收文件均独立计算SHA并匹配。此版本限定的结果不冒充手机文件、音频或全部六设备验收。
- 红米补验：当前系统提供18个launcher活动，Weavelo发布16个去重独立应用及1个整屏来源；此前只见少量来源的现象未再出现。实际拉取“文件管理”独立窗口，10个解码帧、零错误，原生截图可见应用首次使用页面及正确图标，未点击同意或申请权限。已有主屏任务的“文件”应用被明确拒绝迁移。观看结束并已确认清理；不把首次使用页面出画当成完整文件管理或输入验收。

检查：Android82项JVM测试与lint、测试APK编译，publisher36项、桌面图标5项及签名发布回归1047项通过，19项跳过。当前源码、签名与设备身份保持；91的原生PNG测试在原签名正式产品上运行。

证据均在各机 `.rust-storage/validation-logs/weavelo-gui-20261102/`：`android91-*-installed-proof.json`、`android91-native-artwork-after.log`、`android91-nubia-artwork-relay.{json,png}`、`android91-nubia-app-artwork-native.png`、`android91-redmi-filemanager-app-artwork-native.png`、`android91-redmi-screen-catalog.{json,err}`、`native138-phone-dock-item-{1,2}.png`、`native138-phone-dock-final-cleanup.json`、`matrix137-all-desktop-summary.json`、`android91-public-download-verified.json`、`native138-nas-verified.json` 及各平台升级记录。

继续项：努比亚本人恢复无障碍后验证独立应用标准编辑器、源绑定键鼠和多指；红米测试APK安装限制；应用旋转／输入法／恢复／自动音频；HO5解锁后的真实PipeWire与同Wi-Fi延迟；当前六设备文件、摄像头、音频及通知矩阵；白板与教学规划。Octendra分工的本次建议见 [Android自有显示方案](../plans/ANDROID-OWNED-DISPLAYS.md)，尚未进行跨项目归属迁移。


## 共用遥测与 Linux 帧率、输入修复 · 2026-11-06（待实机升级）

用户新报告涉及码率／帧率超限、拖动中断和遥测低估，按跨平台共用链路排查。已确认并修正：Linux 原生 PipeWire 采集漏用目标帧率；HostTelemetry 和 PipelineTelemetry 串到同一连接的其它窗口；静止画面停止发送遥测；Linux 将过期输入误报为不支持事件，及像素格式重新协商误判为几何变化。保留真实几何变化、来源撤销和过期输入取消；不重放旧输入。

遥测改为每秒按订阅上报，静止时明确显示零活动；去除编码耗时100ms上限。原生GPUI首次提交每帧时记录等待时间，HUD展示采集到UI提交估计值并标注时钟不确定性。GPU完成、显示器扫描和物理glass-to-glass仍未测量，不以此宣称达到5／20ms。码率的实际约束仍须原生编码和真实链路验证，尚不能仅凭修复帧率宣布完成。

检查通过：Linux capture20项（1跳过）、Linux portal input13项、host input96项、remote_core telemetry10项（包含实际UDP双窗口隔离）、protocol telemetry1项、GUI telemetry10项和presenter2项；签名发布回归1047项通过、19项跳过。当前四台电脑仍运行138，HO5处于锁屏；本段不是139部署或真实拖动验收。

Android93已安装努比亚，独立核对base.apk SHA为`8711da23f92d2507825d1cc800dafb8a1a4be8f6be4ac2f42a3efa859edf0eac`，保留既有签名及用户恢复的无障碍。真实独立编辑器出画、触摸到达，但文字出现“text input owner changed”，接收端原生文件仍为空；继续定位，未标记为修复。93增加异步64px真实应用目录图标，界面验收和红米升级待完成。Linux真实应用列表与图标仍未实现；已确认KWin只读窗口目录可用，正确采集绑定待完成。

证据：各机同一任务目录`native139-*`、`linux139-reported-journal.log`、`android93-nubia-installed-proof.json`、`android93-native-independent-input.json`、`android93-restored-nubia-text-native.png`。

## 共用修复部署与 Linux 码率验证 · 2026-11-06

上述共用修复已提交、推送并升级：Studio、Book、cube 运行 desktop140（`3349b30`），HO5 运行 Linux141（`3180b46`）。三种桌面安装包已正式发布，公开下载内容与发布 SHA 一致；签名、设备身份、既有系统授权与启动方式保留。139 的“待升级”是前一阶段记录，不代表当前安装状态。

继续排查发现 Mac/Linux 的关键帧与可视区域重绘没有共用采集帧率预算，Android GPU 重绘也漏了帧间隔；140／Android95 起统一约束这些路径。实际 Studio 动态测试窗口设置 7／30 FPS，接收平均为 7.04／28.17 FPS；曾有一个短统计区间显示 8.60 FPS，不能据此宣称任意瞬时区间都严格不超过目标。Windows 原生路径已有对应帧率约束。

Linux141 将硬件编码 VBV 从一秒改为一帧，消除低帧率下过大的码率突发；不丢弃压缩参考帧，不降低分辨率或改变颜色。原生 VAAPI 对照实验（7 FPS／1 Mbps 与 30 FPS／2 Mbps）中，持续变化噪声的最高一秒码率由 12.16／15.99 Mbps 降至 1.94／2.94 Mbps，稳定段为 1.515／1.870 Mbps。因此这是改善突发与目标跟随，**不是任意内容的硬性带宽上限**。实际移动图形 testsrc2 的稳定码率为 1.003／2.062 Mbps；编码中位数 4.84／3.21 ms，p95 约 5.98／4.07 ms。该对照只测实际硬件编码，不包含采集、网络、解码或显示，不等同于同 Wi-Fi glass-to-glass 延迟。

Studio140 的原生窗口已显示 Windows140 实际桌面且解码错误为零。HO5 当前 Wayland 会话锁定，Linux 流虽解码并提交两帧，实际截图为黑屏；不把这次结果算作可见桌面或拖动验收。还需在 HO5 解锁后核验实际移动桌面的帧率、码率、拖动连续性和遥测稳定性。Linux 遥测的采集起点是 PipeWire 到帧，未包含此前合成器等待；UI 提交也不是屏幕实际发光。5／20 ms 仅作为良好同 Wi-Fi 条件下的优化目标。

检查通过：共用 FramePacer 4 项、Linux capture 20 项（1 跳过）、签名发布回归 1047 项（19 跳过），以及前述输入与遥测针对性检查。证据：`native140-streams-live*`、`native140-synthetic-rate.json`、`native141vbv-synthetic-rate.json`、`native140-testsrc-rate.json`、`native141rt-testsrc-rate.json`、各机正式升级记录、`native141-nas-verified.json`。测试会话均已关闭。

## Android98：连续文字、应用目录图标与两机升级 · 2026-11-06

努比亚 NX741J（Book USB `324662166861`）和红米 2409BRN2CC（Studio USB `UGXC8DQGDYBATKB6`）均已正式升级 Android98，来源 `0359d29`。原签名与各机既有无障碍启用状态保留；分别拉回已安装 `base.apk` 独立核对 SHA，与正式包和公开下载相同：`89e099f0f57a28b91c3e3c734247d5be4ee4667c1eb1865be7d29a135676aa10`，19,679,906 bytes。两机 helper API4／version98 已启动，NAS 当前 Android 也已发布98。

独立应用文字输入修复包括：空编辑器的 -1 光标与提示文字处理；避免重复设置已在末尾的光标引起虚假拒绝；提交前刷新原生编辑节点，再校验焦点、显示器与应用归属。97 曾出现第二段覆盖第一段，不能沿用其“界面无错误”作为成功证据。98 最初两次测试还遇到测试窗口失焦／仅收到后一段，因此增加接收端逐次记录，并恢复测试工具在每次输入前的焦点校验。

真实 Studio GPUI → 努比亚独立原生编辑器先后记录了空文本、`abc你好🙂`、`abc你好🙂继续Z`；下一次独立会话再以约200ms间隔逐字追加七个字符，手机原生文件最终精确为 `abc你好🙂继续Z逐字输入`，nonce `c435762a-42ce-485e-8e50-e42fbd46c69c`、display82。该证据独立于发送端事件排队／界面提交；不代表游戏多指、任意应用编辑器或物理显示延迟的验收。

Android 应用目录异步提供真实64px PNG，选中应用仍使用高清图标路径。此前真实配对网络目录验证249来源、248图标绑定、245个可解码去重PNG；98又在正式Studio可见应用选择页验证了真实图标和“计算器”名称筛选，已查看实际截图。红米98的独立文件管理窗口已在Studio出画、图标和设备名称正确、零解码错误；画面停在该应用自己的首次同意页，未替用户同意，不能声称文件管理交互已验收。

Android98 JVM83项与lint通过，发布回归1047项通过、19跳过。证据：Book任务目录 `android98-build.log`、`android98-nubia-installed-proof.json`、`android98-input-transitions.json`、`android98-continuous-input-proof.json`、`android98-publication.log`；Studio任务目录 `android98-redmi-installed-proof.json`、`android98-public-download-verified.json`、`android98-continuous-keyboard-run.log`、`android98-catalog-20204.png`、`android98-catalog-search-native.png`、`android98-redmi-nubia-app-artwork-native.png`。测试会话和本次新开的目录窗口均已关闭。

红米“设置”应用的额外探测已连接到目录确认的 `Window(3359028817)`，但未获首帧，接收窗口随后自动关闭；原因未确认，不能归为系统权限不足。记录在 `android98-redmi-settings-run.log`、`android98-redmi-settings-app-artwork-all.json` 和 `android98-redmi-managed-after-settings.log`，后续需补齐具体失败原因及界面错误保留。

未完成项继续保留：HO5解锁后的真实拖动与移动画面码率验收；Linux真实应用窗口采集与目录图标（KWin目录可读，正确采集／控制绑定尚未实现）；红米设置应用首帧、独立编辑器测试及六设备其余音频、摄像头、多指与双向验收。以上部署不代表所有已知问题全部完成。

## 2026-11-06 当日反馈收口（未完成，不作为升级验收）

用户要求今天反馈全部修复后统一升级四台电脑和两部手机，不以说明、单项提交或局部检查结束任务。当前已提交的界面修改仍未部署。HO5已重新核对为 `LockedHint=no`（操作 `2b5e28dc-fa8c-4064-9fb9-e9e5b4702270`），锁屏不再是当前测试阻碍。

- Linux真实应用列表、图标及选中后的独立窗口采集：未实现。
- 连续输入中断／顶部操作栏弹出：未定位完。Book实际两条Studio应用连接在线，一条窗口未激活而暂停输入，另一条控制有效；不能据此把所有中断归为失焦。需抓取具体TextRejected／控制错误及发生时焦点。
- Text模式回车／Tab：已定位macOS原生事件携带`key_char=\\n/\\t`而被旧字符判断截走。修改为可打印文本或IME组合期间才留本地；其它控制键发往远端；不发送没有对应远端按下的抬键。针对IME确认、回车、Tab、空格、中文emoji的原生Rust检查通过，发布回归1047项通过、25跳过；真实多平台输入验收和部署待完成。
- 滚动明显不如原生：新增优先项。已发现Windows接收端把像素滚动乘120、忽略横向滚动；macOS/协议未携带滚动阶段。尚未修改或测量效果，不宣称已优化。
- 整屏主名称使用电脑名，不重复画设备名：`3b0ccc5`代码完成，待实机。
- 设备图标左上角、设备名左下角：`3b0ccc5`代码完成，待实机。
- 遥测面板固定宽度、数值对齐、简洁文案：`3b0ccc5`代码完成，待真实动态截图。
- Dock悬停气泡应显示应用名，整屏显示电脑名；其他系统原生入口同样处理：未完成。`bce4f42`只删除错误绘制在图标下的应用名，当前helper仍用Weavelo主bundle，现有update_presentation仅改菜单，不是系统气泡。
- macOS侧边收起窗口图标与Dock一致；同时检查Windows/Linux：未完成。现有Mac适配已设置miniwindowImage，但不能据此宣称Stage Manager或其他系统入口一致。
- Linux帧率／码率／拖动及共用遥测：前述140/141已部署；解锁后的实际滚动、拖动和持续采样验收仍待完成。
- 红米设置应用首帧失败及其他前述遗留问题：继续保留，不因本轮界面修改关闭。

最后必须完成正式打包、NAS发布、兼容安装、实际运行版本及实机行为核验；不得只把main提交当作全设备完成。


## 2026-11-06 桌面142／Linux143继续验收

Studio、Book、cube 已正式升级142（`743aec1`）；两台Mac分别保留原有证书，正式安装记录确认配置和启动方式保留。HO5已升级Linux143（`67db7d9`）。Android98本轮源码没有改变，仍是当前手机版本。NAS尚未发布本轮桌面包，不能将安装与公开发布混写。

- Book142同时打开Studio的Gemini主窗20363、ChatGPT主窗18227、DeepSeek Harness主窗19556，分别解码12／27／20帧，错误均为0，三路均有UI提交帧。来源不再默认添加桌面，测试连接全部关闭。证据：Book `native142-book-studio-three-apps.json`。这是接收和UI提交证据，不是物理显示测量。
- Studio142拉取cube142桌面，解码9帧、错误0并提交画面；自身测试连接已关闭。证据：`native142-studio-cube-display.json`。
- 两个原生AppKit编辑器的A/B/A/B输入测试通过，包括中文、emoji、Return和旧窗口modifier清理不抢焦点；不同进程重叠窗口与同进程双窗口分别验证。随后实测Book142两个接收窗口→Studio142同进程双编辑器，A/B/A/B各输入`net0`至`net3`并回车，接收端独立读回A为`net0\nnet2\n`、B为`net1\nnet3\n`，完全匹配。后两次切换没有再点击编辑器。自建流和夹具全部清理。证据：Book `native142-book-studio-input.json`、Studio `native142-input-acceptance/network-independent-input-proof.json`。
- 上述输入会切换远端前台窗口，不是保持远端窗口顺序不变的后台控制。有效编辑器基线后的`postToPid`后台对照没有实际事件／文字到达；该实验路径未作为产品能力交付。
- Windows像素滚动换算、横向滚动、分数累积和窗口焦点交接已实现；18项录制API契约检查通过，包含拒绝焦点时不发输入、释放旧按键不抢焦点。实际Windows窗口滚动体验仍须单独检查。
- Linux KWin独立窗口采集、真实图标路径已实现，但143的真实Studio来源选择页仍只有Display1，不能标记完成。已修复KWin宽高实际返回double造成的过滤；继续排查运行时目录与后台IPC仅返回共享屏幕的问题。证据：`native143-linux-source-picker.png`、HO5 `/tmp/weavelo-linux143-sources.json`。仍无KWin选中应用的首帧验收。

本轮发布回归1047项通过、25跳过，Linux KWin针对性检查6项通过。Dock真实名称、侧边窗口图标、跨平台持续滚动／输入和未完成六设备事项继续验收；这里不代表全部问题关闭。


红米设置首帧失败的原因现已确认：设备卡片的真实接收错误为 `App is already open; close its existing task before opening an independent app`，系统任务列表也显示设置仍有现有任务。失败没有归因于录屏授权；为避免搬走手机已有任务，当前独立空间明确拒绝这种启动。此前“原因未知”的记录由此更新，不代表已支持重复启动任意应用。错误保留在 `stream_activity.devices`，不能只读取已关闭会话的空数组。证据：`native142-redmi-settings-confirmed-rejection.json`、`native142-redmi-activities.txt`。随后Studio142拉取红米98整个屏幕，4帧、0解码错误并有UI提交，测试流已关闭；证据 `native142-redmi-screen.json`。


## Android99 设备类别图标与原生入口验收补充 · 2026-11-06

`a62cc74` 已提交并推送：设备发现复用现有能力字段的保留位传递设备类别，Android 设备卡片使用对应手机、MacBook、Mac Studio、Windows 或 Linux 图标；不依据名字猜测类型，不广播自定义图片。桌面读取其实际配置目录，手机自定义图片仍通过现有鉴权串流通道获取。发现序列化19项、桥接与发布端各1项、配置目录隔离2项检查通过；Android JVM 85项及 lint 通过。新增 Compose 仪器测试尚未运行，不能据此宣称实机界面验收。正式 APK 构建和两机安装仍在进行。

Book142 的两个接收窗口实际创建 Gemini／ChatGPT 独立 Dock helper；原生进程 localizedName 与 Dock AXTitle 对应，停止后进程和临时 bundle 均已清理。旧 ChatGPT Dock 缓存条目无进程、路径不存在，未修改用户 Dock 配置。自动隐藏 Dock 的截图未拍到气泡，最小化属性写入后仍读回0，所以可见悬停外观、侧边／最小化窗口图标仍未通过实机验收。

Android99 正式构建已完成，努比亚与红米均已安装，独立拉回 base.apk 核对 SHA 均为 `656c36458de837097bf2561fa4cf26f002c608249be9fe237b76a179b5408448`，原有无障碍启用列表未改变；helper API4/version99。NAS 当前版本99和公开下载亦已核对。红米实际设备页截图 `android99-redmi-device-cards.png` 已显示努比亚手机图标；其余电脑尚在142/143，未声称其新分类图标已验收。Compose 仪器测试源码补充编译通过（修正 assertExists 成员方法的错误导入，`3130c60`），尚未运行设备仪器测试。


## Desktop144 正式升级、Linux应用首帧与 Android 图标实机 · 2026-11-06

四台电脑已安装 Desktop144，运行时来源 `2d9b934`。Studio 和 Book 保留各自原签名；Windows、Linux 正式升级完成。NAS 三平台包已发布并独立下载核对 SHA；两部手机继续运行已核对的 Android99。Linux 另应用 `348d0e0` 的启动入口迁移修复：真实旧入口使用名称 Weavelo 和0600权限，现改为规范可执行路径及KWin接口声明，保留原权限与SELinux标签。所有改动已推送main。

- Linux后台实际列出6个窗口及1个屏幕；Studio来源选择器实拍可见Firefox与Bazzite Portal真实图标，按应用分组。证据 `native144-linux-repaired-sources.json`、`native144-linux-source-picker.png`。
- Studio144接收Linux Bazzite Portal独立窗口，2帧、0解码错误且UI提交；`native144-linux-application-visible.png`实际显示该应用内容。第一版通用检查要求超过3帧，因此对静止窗口返回false；随后按首帧要求独立截图核验，未把这个阈值失败伪装成持续动态帧验收。Linux该独立窗口仍明确不支持控制，整屏控制不受此能力标志影响。自建流均已关闭。
- Book144→Studio144 ChatGPT独立窗口复测11帧、0错误、UI提交成功，测试流已关闭；Book `native144-book-studio-chatgpt.json`。未重新宣称本轮完成全部平台输入或物理显示时延验收。
- 红米99实拍五台对端分别显示MacBook、Linux、Mac Studio、手机、Windows类别图标，证据 `android99-redmi-all144-badges.png`。
- Book原生Dock截图显示ChatGPT与Gemini图标、正确悬停气泡名称及MacStudio角标；源PNG为 `codex-book142-dock-edge-47e1366d306c460db2d3ae91c27e597e-{ChatGPT,Gemini}.png`。测试输出JSON曾因CGPoint序列化失败，但图片已存在且逐张查看；Stop确认无残留流。侧边收起窗口／Stage Manager图标仍未验收。

最新发布回归1052项、25项跳过；Linux来源合并原生测试2项和程序编译检查通过。公开包核对见 `native144-nas-verified.json`。持续滚动、完整多平台控制、实际端到端时延及先前未完事项继续保留，不能用本节代替完整六设备验收。

## Desktop145／Android100 待发布修复 · 2026-11-06

当前正式运行版本仍为 Desktop144／Android99。本节记录下一轮源码与检查，不代表已经升级或完成实机验收。

- 原生滚动：新增紧凑的手势阶段与惯性阶段，保留零位移结束事件；macOS使用正确的CoreGraphics阶段值，Linux按120像素对应一个滚轮刻度累计高精度及传统事件。窗口后台或指针移出时，过期惯性不能抢占控制。Windows契约19项、滚动单元5项及两条真实GPUI事件路由检查通过，来源切换与取消绑定已通过独立复核。
- 遥测：HUD固定244像素，Network标题保持不变，底部Link统一反映质量；Capture→UI只使用有时效且属于当前解码代次的提交样本。RTT显示最近不同采样的中位数，详情保留最新值和范围；修正主机发送时间戳计入排队等待的问题。缺失数据显示未知，不将它变为低延迟。遥测独立复核通过，真实界面及网络波动仍须升级后核验。
- Linux图标：实际串流截图确认接收窗口用了通用紫色图标；回退图标改用正式Weavelo品牌资源并保留来源设备角标，PiP同步原生图标。KDE托盘解析到的是正式资源，尚无足够清晰的实拍证明托盘图标错误，因此不把修复接收窗口等同于托盘和任务栏全部验收。
- 复制粘贴：默认GPUI连接接入原生文件剪贴板桥接；文字、图片和文件共用明确的剪贴板所有权。最近聚焦的远程会话在转到本地应用时继续同步，切换远程会话才交接。已补入旧文件组完成、命令积压、延迟启动和被撤销原生写入的针对性修复；43项核心剪贴板筛选及9项明确点名的定向检查通过；随后补充主机1→3→2乱序代次和双向跨格式复制的5项检查通过，当前应用编译检查通过。进一步以共享内容代次覆盖交错轮询，最新核心剪贴板筛选49项及macOS命名剪贴板原生优先级1项通过，交错轮询修复已通过独立复核。
- 已准备原生AppKit复制／粘贴夹具，分别读回文字摘要、图片像素和文件摘要；尚未启动本轮真实跨机验收。跨远程窗口原生拖放的目标、系统格式边界与分期见 `docs/plans/NATIVE-CROSS-DEVICE-CLIPBOARD-AND-DRAG.md`，没有将文件传输成功当作拖放完成。

发布回归本轮已运行：1052项通过、25项跳过。两台Mac原签名预检通过。还需完成收尾复核、源码提交推送、正式构建安装以及独立实机验收；完整六设备矩阵及既有未完成事项继续保留。

## Desktop145／Android100 实机续测 · 2026-11-07

`f3f05e3` 已提交、推送，四台电脑正式安装 Desktop145；两台 Mac 保留各自原签名。Android100 已安装到努比亚与红米，两台独立拉回的 base.apk SHA 均为 `1bebae33bbcf46f80a9ef2fe1dc63032d35d1fd3fced3c22f93e553ec1a14fc7`，原无障碍启用列表保持一致，helper API4/version100。Android100 已发布并独立核对公开下载；桌面 NAS 仍为144，不能混同已安装版本。

- Book145 在 Studio145 远程应用窗口中执行原生 Cmd+C，随后切回 Book 本地 AppKit 窗口执行 Cmd+V：中文／emoji 文本摘要匹配，文件接收摘要匹配 `bdfaaf4faf8d2ec0aafdeb1d74841d978022ef07a95a98dd7a8c9c942ed5a5c9`，64×64图片按同一原生颜色转换逐像素核对匹配。Book证据 `clipboard145/live-{proof,file-proof,image-proof}.json`；Studio保存 `clipboard145/pasted-book.png`。仅此方向、此格式和普通完成场景通过；大文件竞争、反向、第三设备焦点交接仍待实测。自建连接和夹具已关闭，测试剪贴板经内容核对及changeCount比较恢复。
- Studio145接收Linux屏幕约60FPS、0解码错误；实拍HUD固定窄宽，Network标题和底部Link状态正确。截图中Linux主窗口、任务栏、托盘显示当前品牌。证据 `native145-linux-steady-expanded-{0,1,2}.png`。但Capture→UI在5／10／15秒均为空，尚未关闭此问题；随后原生悬停提示明确显示“clock mapping expired”。该测试打开的是主接收窗口，并非PiP，不能把问题归因于PiP回报缺失。
- Book145→Studio145原生滚动收到34事件，触摸／惯性阶段已保留，但接收事件跨度3.551秒、最终偏移164，相比原生基线偏移219仍不达标。证据 `scroll-native144/after145-independent-proof.json`。发出端原夹具没有记录实际投递时刻，不能把请求12毫秒睡眠等同于真实发送间隔，也不能直接把全部间隔归因于网络。

随后加入发送端单调时钟测量：34事件发出跨度3549.678毫秒，事件post中位0.072毫秒，而发出间隔中位109.731毫秒，证明原夹具的12毫秒睡眠请求受到计时调度影响。不能用该夹具的慢速结果证明产品处理耗时。后续精确节拍对照继续；针对同一滚动事件重复原生命中检查做最小优化，并在现有本地诊断中暴露时钟及有效提交阶段以定位空值。发布回归再次1052项通过、25跳过；源码检查不代表滚动体验或Capture→UI实机问题已经解决。

校正夹具为有界单调时钟节拍后，Book145实际34事件发出跨度403.770毫秒、间隔中位12.117毫秒；Studio145收到22个系统合并后的事件、总位移−208完整保留，跨度505.193毫秒、事件间隔中位16.724毫秒，最终偏移217，接近原生基线219。证据 `scroll-native144/paced145-emission.json`（Book）与 `paced145-receiver-proof.json`（Studio）。这验证了该夹具链路的阶段和位移，尚不构成所有应用、网络或物理显示延迟的验收。

## Desktop146 诊断与时钟根因 · 2026-11-07

`0e4ffce`已提交推送，Studio已通过原签名正式安装146，其余电脑仍145。两个Android100整屏均从Studio146收到实际解码和UI提交帧（努比亚2帧、红米3帧，解码错误0）；仅证明此方向的屏幕首帧和协议可用，不代表音频或控制全部验收。自建流已关闭，证据 `android100-studio146-screen-proof.json`。

- 60秒实测确认Capture→UI空值来自时钟拟合：最近探测持续新鲜，但选中样本一直只有1条，年龄从6.7秒增长到58.6秒，状态长期Stale；旧样本离开60秒历史后恢复Synchronized。证据 `native146-long-clock.json`。这与旧低RTT样本长期排除新样本的机制吻合，不是单纯UI刷新问题。
- Book145→Studio146滚动对照收到24个合并事件、位移−208完整、跨度382.721毫秒、事件间隔中位16.640毫秒；最终偏移241。单次路径、系统合并与惯性表现存在变化，不把与145的数值差异直接当成优化收益或物理端到端测量。证据 `scroll-native144/paced146-receiver-proof.json`。
- 下一轮用现有10秒新鲜度窗口确定当前RTT范围，并允许60秒历史中同一范围的样本支撑拟合，避免周期性20／60／60毫秒探测引发拟合反复丢失；同时修正活跃串流每秒探测被定时抖动跳过的问题；不延长过期阈值、不伪造时钟准确度。真实升级后的稳定性仍须单独验证。

## Desktop147 时钟修复、发布与双向粘贴 · 2026-11-07

`7f0ae2e`已提交推送，四台电脑均通过正式安装更新到147，两个Mac保留各自签名。三平台桌面包已发布，公开下载SHA全部核对；证据 `native147-nas-verified.json`。时钟30项、客户端消费者23项及探测节拍1项测试通过，独立240次探测复核通过；发布回归1052项通过、25跳过。

Studio147接收Linux147约60秒，12次定时检查中首次为Acquiring，之后11次均为Synchronized且有有效UI提交样本；解码约60FPS，未见解码错误。本次没有再出现146中旧样本导致几十秒Stale的状态。证据 `native147-long-clock.json`、`native147-clock-summary.json`及 `native147-long-linux-11.png`。这是定时抽样与UI提交估计，不是逐帧无波动保证或物理显示时延测量。

复制粘贴补齐反向验收：Book145本地复制后，Studio146/147的原生测试窗口实际收到远程Cmd+V，文字（含中文）、64×64图片、文件内容分别核对通过。图片按原生sRGB RGBA逐像素SHA为 `50089497cf8929f9f196d0da39ca5e1036b774d5944d536565cbb426c30f0144`，文件SHA为 `bdfaaf4faf8d2ec0aafdeb1d74841d978022ef07a95a98dd7a8c9c942ed5a5c9`，重名接收生成 `fixture-file (1).bin`。初次失败因测试未真正激活Weavelo窗口，不能归为产品粘贴故障。证据 `clipboard145/diagnostic-reverse-20261107` 与 `clipboard145/reverse-formats-20261107`（两台Mac）。自建连接及夹具已关闭；Studio剪贴板按所有权恢复，Book变化未能确认写入时序，因此保留现状，未覆盖其当前内容。

上述补齐两台Mac普通场景的双向文字／图片／文件粘贴，不代表跨平台全矩阵、初始剪贴板竞争、大文件与新内容竞争、同路径文件修改后再次复制、或原生跨窗口拖放已完成。Book清理后的内容对应保存的原始文字或测试图片，但现有证据不能区分测试清理顺序与延迟写入；后续须在Stop确认及两端恢复前后记录格式、摘要和changeCount，不读取或公开用户内容。

Android 101 同源 `7f0ae2e` 已正式构建、发布并安装至努比亚 NX741J、红米 2409BRN2CC；公开 APK 与两机已安装 base.apk 的 SHA-256 均为 `755dbdede63361c363e591ce21cbc5c4883668328e9312746d2a5ec0a3b2f094`，原签名及无障碍启用列表保持一致。Studio 147 拉取两机 MainDisplay 均有解码帧、零解码错误和 UI 提交记录，测试自有会话已停止。证据：`android101-{nubia,redmi}-installed-proof.json`、`android101-public-proof.json`、`android101-studio147-first-frame.json`；这不代替物理呈现、音频或控制的完整矩阵验收。

## Desktop148 / Android102 改动验收中 · 2026-11-07

本轮恢复 Android 整屏会话的设备图标，加入手机与应用会话外框自动适配、手动调整保持及“适配画面”按钮，并接入 Android 独立应用的横竖屏请求。应用旋转同时更新原生采集与输入几何；客户端绘制版本参与输入租约申请，采集旋转后拒绝旧版本帧和输入重新授权。会话协议升至11，计划统一升级自有设备。

源代码阶段发布回归1052项通过、25跳过。原生窗口图标8项通过，窗口适配策略及Linux可用区域已有独立复核；这些检查不替代真实窗口显示。Android取消事件的时间戳与取消标志经复核，新增仪器测试尚未在设备运行。真实旋转、旧按住状态取消、四角输入、手动外框与恢复按钮仍待正式包验收；当前记录不宣称148/102已安装或发布。

补充检查发现，Android“按地址连接”旧播放器没有绘制几何回报，会被新的输入保护拒绝。本轮另以Android103将该入口接入已有工作区播放器；地址不冒充发现设备ID，取得来源目录后只自动打开一次MainDisplay，取消或另选来源即撤销待执行选择。“Open workspace”仍保留普通浏览行为。24项相关单元测试与Kotlin编译通过，真实连接与输入仍待验证。

## Desktop148 / Android103 正式更新与窗口旋转实测 · 2026-11-07

桌面源 `2231a7f`、Android源 `4e73f82` 均已提交推送。四台电脑正式安装148，两台Mac保留各自原签名；三平台桌面包已发布并独立下载核对SHA，证据 `native148-nas-verified.json`。两部手机正式安装103，公开APK和两机独立拉回的base.apk均为 `8fd03921dd715214033556be761d9105855dce4588dc578f16dc2254ee7c6e24`，原无障碍启用列表保持一致，helper API5/version103。努比亚安装后的证据写入路径曾因测试脚本替换版本号而出错，安装本身及SHA检查已完成；修正证据路径后独立复核，没有重复安装。证据 `android103-{nubia,redmi}-installed-proof.json`、`android103-public-proof.json`；努比亚原件在Book，另汇总于 `native148-book-rotation-cleanup-evidence.txt`。

- Studio148接收努比亚102独立测试应用，实际截图确认0°竖向、90°和270°横向呈现。竖向源1216×2688，横向2688×1216；接收内容区域随之从367×811调整至1568×709。已观察角落点击对应源坐标，但本轮没有形成所有角落、所有方向的完整自动断言矩阵。证据 `native148-rotation-{portrait,landscape}.png`、`native148-normal270.png`。
- 保持指针按下后旋转，应用收到ACTION_CANCEL，按下计数8、普通释放7、取消1；保持Enter按下后旋转，收到带FLAG_CANCELED的释放，取消1、激活0，保留downTime且eventTime更新。随后正常Enter可激活。原始记录在Book的 `rotation148-held-{pointer,enter}.json`，汇总副本同上。不能把取消测试当作物理显示时延测量。
- 手动修改接收外框后，后续旋转保持手动窗口尺寸；点击“Fit window”后恢复按来源比例自动适配。实际截图 `native148-manual-size90.png`、`native148-fit-restored.png`，恢复后绘制几何与当前几何一致。Android整屏窗口实际显示设备图标和设备标题，见 `native148-main-display-icon.png`。
- Book148原生测试窗口从480×640改为1100×500，Studio148实际接收内容区域从579×811变为1568×758；捕获范围包含远端标题区域，因此不是仅按应用内容尺寸计算。截图 `fit148-desktop-{portrait,landscape}.png`及两份status记录确认实际呈现和几何更新。此项为macOS接收macOS应用，不代表Windows/Linux工作区边界及多DPI矩阵全部验收。
- 升级103后，Studio148分别接收努比亚和红米整屏，2帧与5帧、零解码错误且UI提交。证据 `android103-studio148-first-frame.json`。尚未实测Android103手动地址入口的实际输入；VLC测试停留在引导与本地文件打开失败，未将其记为视频全屏验收。

本轮自建接收会话全部停止，Studio最终无接收或发送；Book桌面夹具通过自己的退出命令关闭，努比亚测试APK按安装收据和SHA核对后卸载，合成测试视频按SHA核对后移除。源代码发布回归1052项通过、25项跳过；本节只补充实际完成的窗口、旋转取消和部署证据，既有音频、多指、完整跨平台输入与物理延迟矩阵仍未完成。

## Linux148 系统图标显示修正 · 2026-11-07

用户指出任务栏、托盘和窗口左上角仍显示旧图标。原生3840×2160截图确认三处为旧交叉图形，不能以安装文件正确或低分辨率整屏截图宣称图标验收通过。当前GUI/background为4022446/4022371，来源2231a7f；托盘实际IconName为 `fun.hackerlife.weavelo.desktop`，并非旧入口提供。独立Qt及KDE主题读取器均把已安装SVG渲染为正确W形，文件SHA与正式包一致。系统确实另有旧入口，但不能将其当作本次三处显示问题的根因。

KDE图标变更通知、应用索引刷新及图标mtime刷新均未立即解决；随后重新加载KWin配置并重启 `plasma-plasmashell.service`，任务栏、托盘及标题栏实际截图均恢复为W形。Weavelo两进程PID保持不变，未重启业务程序、注销桌面或清除全局缓存。问题定位为当前桌面会话保留的旧图像，而非148未安装。HO5证据目录中的 `linux-icon149-native.png`、`linux-icon149-after-panel.png` 为前后原生截图，局部检查覆盖三个位置；此次仅环境修复，没有发布149。后续图标验收必须检查实际系统显示，不能只比较文件SHA。

## Linux151：小米整屏越界与底部手势实机修复 · 2026-11-07

用户指定在Linux查看小米整屏时，窗口显示不全、桌面翻页和底部上滑困难，并要求先复现、再区分共享逻辑与平台适配。本轮在HO5的Wayland桌面、红米2409BRN2CC/Android103上完成复现及正式包复测，没有按设备名称增加特例。

- `825df91` 修复Wayland程序调整窗口大小时，将全局位置和物理像素错误传入surface-local逻辑坐标接口的问题。149正式安装后仍出现底部越界，因此没有把第一项源码修复直接记为实机通过。
- `da0e304` 修复第二个原因：Wayland的configure_bounds只提供推荐尺寸，不是窗口当前位置以下的剩余桌面范围。平台不再伪造已知位置的工作区；共享适配策略在位置不可知时使用系统已分配的窗口范围，并保留该范围供横竖屏切换，避免重复缩小。150实拍由497×1131变为351×800逻辑像素，顶部状态栏与底部导航区全部落在桌面内。
- 150普通左右翻页已成功，但底部开始的拖动复现两类本地拦截：接近底边时800高的窗口被拖成651高，远端未收到按下；稍上方则被展开的网络浮层截获，记录只有松开而没有按下。`36cea8c` 将窗口模式的视频与输入共同内缩8逻辑像素，网络浮层从底边12上移至48；全屏不加窗口边缘内缩。修复位于共享GUI层，绘制与输入继续使用同一矩形，没有修改Android输入协议或绕过输入所有权。

151正式包源为 `36cea8c9ebd84fe69792bd6980434c1430cd2743`，HO5通过正式升级入口安装，实际GUI为4189776、generation `4189776-1794036576568744407`。正式包SHA-256为 `45efe3ddcacaf197ee18f79e605515d0744adfce3465d8bc311d925cc1280806`，源身份和安装保护均通过。正式NAS发布器已核对公开下载相同SHA；macOS及Windows的148包与发布条目保持原样，本次不宣称其他平台已安装151。

实际验收结果：

- 351×800窗口完整显示720×1640手机画面，顶底可见；全屏2648×1490及恢复351×800均实拍通过。原生3840×2160桌面证据为 `linux149-after.png`、`linux151-fit.png`、`linux151-fullscreen.png`、`linux151-restored.png`。
- 设置页面中，从远端底部导航条慢上滑（本地30次移动、约0.391秒）进入最近任务；快速上滑（10次移动、约0.088秒）返回手机桌面。两次均收到并排队Down/Up，窗口保持351×800，松开后held_buttons为空。手机结果分别见 `redmi151-after-bottom.png` 与 `redmi151-after-bottom-fast.png`。
- 密集鼠标拖动左右各80次移动，分别约0.370秒和0.369秒，手机桌面在两个页面间实际切换；接收端累计Down/Up为2/2，排队同为2/2，松开后无残留按键。对应手机截图为 `redmi151-after-dense-{right,left}.png`。这些时长是本地注入序列时长，不是端到端物理呈现延迟；此场景没有证据要求修改共享输入队列。
- 当前几何与实际绘制几何一致，source crop为完整720×1640。所有自建接收租约已停止，最终sharing/receiving均为0；自建ydotool进程按PID/命令行核对后停止，未更改系统鼠标加速度、无障碍授权或其他安全设置。

检查包括窗口适配策略8项、原生Linux编译及macOS编译检查、发布回归1052项通过/25项跳过。小范围HUD位置调整由最终Linux原生编译和151实拍再次覆盖。可复核汇总位于任务证据目录 `.rust-storage/validation-logs/weavelo-gui-20261102/linux151-acceptance-evidence.json`，发布记录为 `linux151-publication-verified.json`。本节覆盖用户指定的Linux→小米整屏问题；不把该结果扩展为所有DPI、X11、多显示器、音频或跨平台物理延迟矩阵全部通过。
