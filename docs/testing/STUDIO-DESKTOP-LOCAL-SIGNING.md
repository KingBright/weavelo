# Studio 独立 Desktop 本地签名身份

2026-10-25 用户明确批准在 Mac Studio 创建新证书，不依赖旧证书/Book，适用
新品牌独立 appID。此前的旧密钥迁入/原机接续不再作为交付前提；本轮没有访问
Book、重复导出/迁移原密钥，或借用 Remote Hosts 身份。

**当前结果：Studio 原生签名和标准签名 ZIP 均已通过独立验收，原公共事务已
提交并完成清理。** 版本 `2.0.0-alpha.9` / build `20261025.9`。下文保留创建和
短窗口超时的历史，最终产物见“标准包完成”。未安装或发布 NAS。

## 固定身份和边界

| 项目 | 值 |
| --- | --- |
| 新品牌 appID | `fun.hackerlife.weavelo.desktop` |
| 新证书 SHA-1 | `d38bedce6830fb01240f97c11c7984514ab715c0` |
| 新证书 SHA-256 | `0072916fa61b6596695338fbf1ebb52a6f22d018791a1acbfa9c5ef1d8480201` |
| 证书名 | `Weavelo Desktop Studio Code Signing`（名称不是授权证据） |
| 新策略 | `deploy/macos/desktop-signing-policy.json`，`identity_kind=studio-local-desktop` |
| 指定要求 | `identifier "fun.hackerlife.weavelo.desktop" and certificate leaf = H"d38bedce6830fb01240f97c11c7984514ab715c0"` |
| 本机位置 | 登录钥匙串；公开证书/状态在 `~/Library/Application Support/Weavelo/Signing/` |
| 有效期 | 2026-10-25 至 2036-10-22，RSA 3072，digitalSignature/codeSigning，CA:FALSE |

这是持久本地代码签名身份，不是 Apple Developer ID，没有公证，也没有添加
用户或系统根信任、修改 Gatekeeper/TCC 或申请 Apple 账户。
[Apple TN2206](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)
区分代码签名/指定要求的有效性与各系统子模块的信任政策；本地身份可用不代表
Gatekeeper、分发信任或系统权限已获准。

旧策略 `deploy/macos/signing-policy.json` 仍固定
`8eaa97365d97b8a987cdf1232430eddf9aba8ecf`，旧 appID 不接受新本地模式。
已经安装的 Desktop `20261023.16` 也使用旧证书；虽然其 appID 同为 Desktop，
本次新证书包**不是它的身份兼容升级**。没有替换该安装、修改其组/profile/
launch agent 或搬迁其数据；后续安装必须经过原碰撞/身份比较门禁，不能覆盖旧包
或自动承诺继承旧系统授权。

## 创建和恢复结果

先核对现有钥匙串公开元数据：只有 Remote Hosts 身份，无 Weavelo 证书或本项目
签名目录，才执行已批准的显式 `setup_macos_desktop_signing.py create`。
私钥在 `0700` 本地目录中的 `0600` 临时文件生成，经 Security.framework 存入
登录钥匙串；新增私钥访问指定系统 `/usr/bin/codesign`。密码只在进程内和本地
pipe 中使用，不进入 argv、环境、stdout、聊天、Git、Library 或 Sites。

首次导入返回 `-25264`：原生解释为 PKCS#12 MAC verification failed；独立观察
确认未出现匹配身份。`resume-format` 在核对同一证书指纹和公钥匹配后，只修正
本地临时导入封装格式（3DES PBE/SHA-1 MAC），没有生成第二个证书/密钥。
修正后的导入成功。随后独立查询及再次调用 create 都返回同一公开指纹，未生成
新身份。临时私钥 PEM 和两个 PKCS#12 文件已删除，私钥只保留在登录钥匙串；
没有保留可导出的私钥文件或密码文件。

`create` 不会覆盖已有身份或未知局部状态；再次调用只检查已有记录。格式修复
本身也有单次 intent，不能循环重放。拒绝/未知状态先核对原结果，不通过新建
第二套身份、借其它项目证书或修改 trust/partition 设置来绕过。

## 已验证与尚未通过

- 新证书/key pair 的原生 metadata 匹配、公开指纹、权限和重复使用同一记录已验证。
- 新策略预检定位到该精确身份，无 ad-hoc fallback；这是签名输入检查，不是签名完成。
- 固定 `./scripts/test_release.sh`：816 项，23 项按条件跳过，零失败。
  包括防重复创建、失败保留、pipe 密码、同一证书格式恢复、独立 appID 限定，以及
  新策略不接受旧证书或 Remote Hosts 证书。未运行真实产品重新构建。
- 第一次对自有、不运行的微型 C fixture 进行实际 codesign 请求，35 秒超时。
  原 codesign 进程随后已不在；临时 fixture 已由其 owner 清理。没有签名成功回执，
  也没有据此生成 signed release ZIP。
- 正式 Computer Use 对 `com.apple.SecurityAgent` 返回安全策略拒绝，无法查看或
  代操作该系统界面。没有改走截图/AppleScript/其它 UI 通道，也没有自动接受提示。

需要本机确认是否存在 codesign 使用本项目新私钥的钥匙串提示；如有，由用户在
系统窗口完成正常授权，密码不要发送给助手。确认后才做两次真实签名、精确 leaf/
DR 验证和旧要求拒绝测试，再进入正式打包。没有看到提示时如实反馈，不能假定
提示存在或重复签名制造更多请求。

用户随后回复“好了”。在独立确认旧 codesign 已退出、自有 fixture 不存在、原
编译/包/回执和已安装 binary 摘要不变后，仅恢复了一次新的正常签名请求。
这次第一步签名仍在 45 秒后超时，未进入第二次复用验证。原 runner 已杀死并
等待该子进程，自有 fixture 随后清理；再次观察没有同用户 codesign 进程、
没有新 signed release ZIP，也没有 native passing receipt。
只读 Security.framework 访问列表元数据显示新密钥确有 `/usr/bin/codesign`
条目；没有导出私钥、读取 SecurityAgent 界面或改变 ACL/partition-list。
因此不能将这次超时解释成“证书未创建”，也不能将用户回复本身当作签名成功。
失败不发布、错误身份拒绝和子进程退出后 abort 的三个固定 fixture 已通过。
后续需要本机确认私钥访问步骤，并在下一次单个正常请求仍活动期间完成；
不要改根信任、自动宽放 key ACL 或重复并发请求来尝试消除等待。

### 300 秒单次受控请求已通过

按新的明确安排，原操作确认结束后只启动一个 300 秒请求，立即报告开始时间
05:28:41 UTC、codesign PID 18488 和原执行会话 55520。该请求在
05:29:42 UTC 退出 0，约 61 秒；精确新 leaf/DR 验证退出 0，旧 leaf 要求被拒绝。
没有访问 SecurityAgent 或修改 ACL/partition-list；fixture 没有执行，进程和自有
临时目录均已确认清理。此前 35/45 秒超时不能直接归因为用户未操作。

原生回执位于
`.rust-storage/validation-logs/studio-desktop-controlled-signing-300s-20261025.json`，
SHA-256 `acd7e9e730603dc59097466ed9577c031577f56d19014c8c09fe52d5f416bf36`。
正式 Desktop 本地身份签名也采用 300 秒有界等待，旧应用签名仍为 45 秒；其它
验签、包内容与身份门禁没有放宽。发布 fixture 初次发现原归档 stub 不接受
`run(timeout=...)` 关键字，修正 stub 并断言旧路径仍为 45 秒后重新检查。
最终固定发布检查：817 项，23 项条件跳过，零失败。
标准包将复用已验证 compiler output，构成同一身份的第二次实际使用，不重复
产品构建；包验收和安装/发布状态另行记录。

### 标准包完成

通过 `bash scripts/package_macos_unified.sh --desktop` 原入口完成正式包；六个
observed-build pins 保持原值，使用既有 compiler output，没有重新构建产品。
这构成同一固定新身份的第二次实际签名，原生 fixture 与正式 App 的 leaf/DR 一致。

| 产物/证据 | 结果 |
| --- | --- |
| 默认 ZIP | `~/rust-target/package/macos/Weavelo-desktop-macos-arm64.zip` |
| 版本 / build | `2.0.0-alpha.9` / `20261025.9` |
| ZIP bytes | `10487048` |
| ZIP SHA-256 | `e9858f65390b0c5ed5f33a0a0cd1e75ca7d80153efb083569d423aed76be7165` |
| 包验证回执 SHA-256 | `4431a48124b1b3daa5007af4b2d8082bfc60cbd63abcb5591469f240988402de` |
| manifest SHA-256 | `0b7753249ba5512169dedae044121fe9c1864768e712d96f95abd9bd819b33ba` |
| 已签名 executable SHA-256 | `301f80c39e9b8077194f037937c814861a474e794f3cc7e05707f84eaad791b2` |
| 编译源码 commit | `d722c5a9a2a974e1ceb0ab7e0712f699fe11e021` |
| 打包 tooling commit | `154f01cd75d1618cac0f2ebbc6fe751b5c4cb7d0` |
| 公共 component | `macos-desktop-arm64-release` |

独立 `verify-archive --desktop` 同时提供原 source commit、compiled identity 和
manifest pins，验证归档里的实际签名、精确新 leaf/DR 和清单通过。
然后单独从原公共 helper `release-preflight --observe-only` 读回，current 文件组/
验证 digest 匹配，`current_update_state=idle`，无在途事务或 pending 路径。
原生产者返回 `current_committed=true`、`cleanup_status=completed`。
验收回执保留在
`.rust-storage/validation-logs/studio-desktop-signed-package-independent-readback-20261025.json`。

编译输出仍为原 `87769a…`，已安装 Desktop 仍为 `20261023.16`、binary
`c50d0c…`，没有被签名包替换。包的本地签名通过不等于 Apple Developer ID、
公证、旧证书安装的兼容升级，或 GUI/视频/输入/音频/长时验收完成。

## 独立接续入口

只读检查：`python3 -B scripts/setup_macos_desktop_signing.py status`。
源/包身份仍分开：已有 Mac observed-build 来自
`d722c5a9a2a974e1ceb0ab7e0712f699fe11e021`，binary SHA-256
`87769a12b4bbaa1e6487b0044aef5fbc51ba202ce738bc8e468f5ea0eb310b22`，
build receipt SHA-256
`430db7f1839ca5e66b24e3a6737a532a3ce5cd549f7448526ea28254be98a6b7`。
本阶段只改签名 tooling/公开策略，后续应先独立核对该既有二进制和回执，复用
正式 `package_macos_unified.sh --desktop`，而非重做已经通过的产品构建。
该流程现已完成；安装和 NAS 发布仍未执行，需分别满足其既有门禁。

旧 Book 请求 `b1f512c2-4cd9-4ec1-af38-1483ca235a2e` 只提交了 workspace-open，
没有附带终端、签名或后续命令。当前工具仅支持 terminal_cancel/transfer_cancel，
没有 workspace-open 的取消接口；未误用其它取消操作，未重发该请求。
