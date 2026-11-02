# GUI 重构进展与接续点

2026-11-02。用户已批准主管理界面、独立单流窗口、统一操作栏和系统托盘方案；当前优先实际实现 GUI，Computer Use 与联合验收随后安排。

方案：[窗口职责与完整功能去向](/Users/jinliang/workspace/remote_hosts/docs/weavelo-window-ui-plan.md)。本轮短 checkpoint：`.rust-storage/validation-logs/weavelo-gui-20261102/CHECKPOINT.md`。代码始终在 primary `main` 的同一个 worktree 修改；每个完整小步检查后提交并推送。

## 当前开发

| 项目 | 状态 | 下一步 |
| --- | --- | --- |
| 管理与单流窗口入口 | 首步源码及检查通过；管理分支已接入真实产品 render，设备 View 改为独立窗口 | 继续固定来源、设备操作与实际 executable 视觉验收 |
| 管理窗口生命周期 | 管理实体保留、关闭 / 重新打开及其他窗口存在的 GPUI 回归通过 | 后台投送与真实文件连续性仍须实机验证 |
| 固定流身份、来源选择与 PiP | 查看设备先读取目录、选来源后才订阅；普通新来源另开窗，同源聚焦，PiP-only 恢复同一控制器；明确 Replace 才替换。软件回归通过 | 推送控制窗口及真实断线 / 来源结束仍需验收 |
| 管理导航、统一单流操作栏 | 管理四项导航、搜索和添加设备已接入；单流栏待完成 | 保留文件、输入、声音、显示、诊断与所有现有功能 |
| 原生 tray / menubar | 待完成；目前仅 Open / Quit | 设备、流、传输、设置的明确目标快捷操作 |
| Android 管理 / 单流页面 | 待完成 | 保留原生视频、手势、IME、旋转与服务 |
| 实机视觉与交互 | 已保存 Studio 73 实际窗口经红米原生接收端显示的 before；新 GUI 尚未安装。桌面自动观察仍超时（`-10005 timeoutReached`），原因未定 | 安装 Studio 单机候选后保存 after；手机接收画面不等同 Studio 物理屏幕验收，不据此跨机部署 |

首步检查：原生生产配置构建通过，实际编译的默认入口仍为 `restored-original-gpui`；真实 GPUI 窗口关闭 / 重新打开 / PiP 引用测试 1 项通过；相关输入回归 37 项通过、1 项跳过。五处旧测试夹具已改用当前视频通道 API，未改动媒体内核。检查完整日志位于上述本轮 checkpoint 同目录。此处不宣称媒体连续性、物理呈现、安装或发布通过。

下一源码小步已完成来源动作接入：`restored_streams.rs` 共用当前目录校验与窗口查找；单流标题保留设备 / 来源并标识 Receiving；多选 Open 不再改写原流，Replace 为明确单选操作，失败保留原选择。真实 GPUI 回归包含同源聚焦、设备 / 来源改写拒绝及 PiP-only 恢复时不增加连接所有者。`restored_ui` 回归 38 项通过、1 项跳过；原生生产构建和签名发布回归 1,039 项通过、19 项跳过。尚未实机安装这批改动。

来源浏览小步已接入实际 View 入口：控制连接读取屏幕、应用窗口和相机目录，空目录与实际错误分别呈现；确认首个来源后复用该窗口及控制连接，其余来源独立打开。目录连接不受文件开关限制，重连保持用途；复用连接时明确应用所选来源，避免误拉默认桌面。刷新保留选择，已结束的来源不自动替换为桌面。目录响应代际只是本地状态，没有更改网络协议、鉴权或媒体内核。

新增真实回环控制消息＋GPUI 控制器回归：空目录、失败、重试及重连期间订阅数始终为零；确认后请求为所选 `Window(77)`，重连后的控制连接直接升级为视频连接，设备及窗口保持不变。该测试没有采集或显示真实画面，不是跨机鉴权验收。三组回归合计 64 项通过、2 项跳过；原生生产构建与签名发布回归已通过。完整证据为 `source-browser-flow-test-3.log`、`source-browser-regressions-3.log`、`source-browser-native-build.log` 与 `source-browser-release-regression.log`。

三个 GUI 提交 `0a02e5d`、`d2c22f1`、`0abafe7` 均已推送并核对远端 `main`。Studio 74 正式打包首次遇到本地依赖 `regex_automata` 加载错误；无并发编译时重建通过，未清理共享缓存。随后产物检查因无关 Sunshine LaunchDaemon 的符号链接拒绝继续，尚未安装或发布新 GUI。已将项目产物使用检查收束到实际引用产物的进程和打开文件，沿用项目生产者锁与原子替换，不扫描无关系统服务或构建；签名及安装身份核验保持原入口。针对性生命周期测试 63 项通过，完整签名发布回归 1,041 项通过、19 项跳过；实际重打包继续记录。

`studio73-via-redmi.png` 是 Studio 73 的真实 GPUI 窗口经实际视频链路显示在红米 Android 61 的截图。对应投送已按请求 ID 停止。此前红米停在“Sharing ended”仍拒绝新请求为“Phone is busy with another connection”；在应用内 Disconnect 后同一来源投送成功。这是明确的 Android 会话结束 / 新请求接续问题，留待 U6 修复。未改系统权限。

## 已知问题与接续条件

下列现场来自[六台设备互通记录](2026-11-02-DEVICE-INTEROP-ACCEPTANCE.md)，不因本轮 GUI 编译成功而关闭。原始证据在该记录引用的各机 `.rust-storage/validation-logs/weavelo-interop-20261029/`。

| 问题 | 已知事实 / 证据 | 下一步 |
| --- | --- | --- |
| 离屏或被系统缩成缩略图的 macOS 应用窗口无首帧 | Book 曾显示约 122×146 缩略图；Studio 73 主窗口 OS 不在屏幕上，30 秒未得到首帧；对应整屏成功。`video73-round1-studio-offscreen-main-failed.json` | 单独核对窗口实际可见性与采集回调；正常激活后的成功不能充当修复证明 |
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
