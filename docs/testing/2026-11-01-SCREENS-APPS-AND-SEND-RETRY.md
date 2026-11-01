# 屏幕／应用窗口选择与投送重试

2026-11-01。使用单一 main 和各设备原有主工作目录；普通打包、发布和兼容升级沿用已授权流程。

## 本次交付

- 桌面与 Android 的拉流来源按 Entire screens、App windows 分组；窗口项显示应用名称。发送和拉取沿用现有来源与独立订阅，不新增媒体后端或协议兼容分支。
- 投送选择器在连接时保留进度。收到对应请求的首帧后关闭；拒绝或失败时保留来源选择、显示错误并允许直接重试。取消只结束对应请求。
- Android 48 修正横屏来源列表裁切；具体显示器存在时隐藏默认 Desktop 别名，同一块屏幕不会显示两次。输入法出现时仍保留可滚动的来源选择。
- Windows 部署识别同安装程序、正确角色、已验证父进程与创建时间的原生剪贴板子进程；Linux 部署通过已验证的标准用户 systemd 服务正常停止后台，不再依赖 GUI 退出后的 IPC 停止请求。

相关源码均已提交并推送：2189ae93、df817d44、e6ee8bfa、de0bb6b2、61a3c5af、06ace914。后两项 Android 修复与部署脚本修复不改变已打包的桌面媒体程序。

## 已安装的实际程序

四台桌面均从 e6ee8bfa 正式构建、打包并原位升级。两台 Mac 分别保留自己的既有签名；Book 未使用 Studio 签名的安装包。

| 设备 | 实际 GUI PID | 已安装可执行文件 SHA-256 |
| --- | ---: | --- |
| Mac Studio | 22693 | 47607509927fcc8aa4f2e564bd31843da6146e832e8e95f47f39adbe05592663 |
| MacBook | 91040 | 4e55152a7a79cd149ddd3e5a8ae8453512ca4dc72d89e062f75a87e64c2ba6ad |
| Windows cube | 32124 | 9fc876fdc8f471de54955d5ea12c8e66c88b1b58dd16738a25ddcfc93e324bfd |
| Linux HO5 | 158598 | 7d89aa8e6adcfe81fb9c7135a33fccb61b5ec89cf8ca953364009143c9c47615 |

这些 PID 是本次观察值，不是后续运行要求。四台桌面的控制和发现中继均已连接，实际 GUI 与后台投影均看到四个在线远端；测试结束后共享数、接收数均为零，没有遗留本次接收租约。

努比亚 NX741J 的 Android 48 从 06ace914 在 Book 使用现有签名正式构建，并通过对应 USB 设备安装，读取系统包信息确认版本。没有创建替代签名或复制私钥。

## 真实界面与视频验证

通过努比亚运行的 Weavelo 实际观看、触摸操控 HO5；ADB 仅作为已授权的手机界面操作入口，未用桌面系统 SDK 替代产品投送。

1. 在 Linux 的真实界面选择发送至 Studio，选择唯一 Display 1。让本次自有接收测试占用 Studio，实际收到明确的目标忙碌拒绝；来源选择保留，错误可见，没有虚假活跃投送状态。
2. 结束占用测试，直接重试同一选择。Studio 实际接收 Linux 画面，累计解码 12,792 帧、解码错误 0，并有原生场景提交。Linux 的共享数为 2，包括手机的独立观看。
3. 只结束本次 Studio 接收；Linux 的共享数自动由 2 变为 1，主动发送记录清除，手机独立观看继续。最终结束手机观看后归零。
4. Android 48 实机横屏显示完整来源标题和唯一 Display 1 行；随后实际显示 Linux 整屏画面。
5. Android 48 在应用窗口组搜索并选择 Studio 上专用测试应用的单独窗口，手机截图实际显示该窗口与对应唯一标题。可见输入法下来源列表仍可操作。

视频解码计数和场景提交不等同于物理显示测量；手机截图直接证明对应手机画面的呈现。专用应用窗口在后续输入尝试前按预定 600 秒期限退出，客户端显示 selected window is no longer available；该次尝试不计作新增窗口输入验收。测试窗口已结束，手机临时横屏恢复竖屏，临时百度输入法恢复原微信输入法。

私有截图：phone47-send-busy-error-e6.png、phone47-send-retry-success-e6.png、phone48-linux-picker-landscape.png、phone48-linux-landscape-frame.png、phone48-app-window-choice-ime.png、phone48-owned-mac-window.png。截图与完整诊断保存在 .rust-storage/validation-logs/weavelo-interop-20261029 及 Studio 的产品接收目录，未上传源码仓库。

## 部署与检查

- Windows 实际四进程清单包含 GUI、后台及各自剪贴板工作进程；改进后的标准升级成功识别它们并完成兼容更新。剪贴板子进程仍是产品进程，不作为测试残留清理。
- Linux 先前在 GUI 退出后发送后台停止请求失败。验证精确旧进程与既有服务后，正常 systemd 停止即可退出，未使用 SIGKILL。公开部署实现随后已改为直接停止已验证服务，并在手机有一条观看会话时完成真实停止、启动测试；实际后台和 GUI PID 更新，安装字节未变。
- 原生来源选择相关 7 项、输入／状态 23 项通过；Windows 部署相关 15 项、Linux 部署相关 58 项通过。
- Android Kotlin 编译和单元检查通过，22 个 Gradle 任务完成；正式 Android 48 构建和安装完成。
- 主 main 签名机器运行现有发布回归：共 1,039 项，19 项跳过，其余通过。未把平台跳过项计作物理验收。

## 已发布

通过 scripts/publish_desktop_nas.sh 与 scripts/publish_android_nas.sh 发布，正式脚本独立下载并核对公共安装包 SHA。现有版本文件保持不变。

| 平台 | 不可变文件 | 安装包 SHA-256 |
| --- | --- | --- |
| macOS ARM64 | Weavelo-desktop-macos-arm64-e6ee8bfa.zip | ca13fd94c14f9fa8aac86d3cbbf504c8bd49a07f273b5d884c1ceb6281034d7e |
| Windows x64 | Weavelo-Windows-x64-e6ee8bfa.zip | e6cb31851745532cadd93bf0db1d6e5cce3e0f70072b8071546e0973634c169e |
| Linux x64 | Weavelo-desktop-linux-x64-e6ee8bfa.zip | c97c79b4111aba590eacadb54c6b5201696f0889e4f64db9a4c36fc3e8b5bbdb |
| Android 48 | Weavelo-Android-2.0.0-alpha.9-06ace91-release.apk | 1ca7d792231dcdff98fa1bb239c6c000509649e2fb9fa062aea036571a979f5e |

[下载页](https://weavelo.hackerlife.fun:8443/download/)；[本次公开验收](https://weavelo.hackerlife.fun:8443/download/acceptance-20261101-screens-apps.html)。最新验收页明确测试总数包含跳过项；此前已发布页面和安装包未修改。

## 继续保留的实际限制

努比亚向外投屏、控制和音频仍需机主完成对应真实系统授权；Studio 所接红米的安装此前停在机主身份验证，未绕过。Linux 至 MacBook 整屏标题栏拖动、完整图片剪贴板与跨平台音频矩阵仍需分别验收。历史已完成的其他设备链路不冒充本次对所有组合的重新验收。
