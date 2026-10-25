# Studio 独立 Desktop 本地签名身份

2026-10-25 用户明确批准在 Mac Studio 创建新证书，不依赖旧证书/Book，适用
新品牌独立 appID。此前的旧密钥迁入/原机接续不再作为交付前提；本轮没有访问
Book、重复导出/迁移原密钥，或借用 Remote Hosts 身份。

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

## 独立接续入口

只读检查：`python3 -B scripts/setup_macos_desktop_signing.py status`。
源/包身份仍分开：已有 Mac observed-build 来自
`d722c5a9a2a974e1ceb0ab7e0712f699fe11e021`，binary SHA-256
`87769a12b4bbaa1e6487b0044aef5fbc51ba202ce738bc8e468f5ea0eb310b22`，
build receipt SHA-256
`430db7f1839ca5e66b24e3a6737a532a3ce5cd549f7448526ea28254be98a6b7`。
本阶段只改签名 tooling/公开策略，后续应先独立核对该既有二进制和回执，复用
正式 `package_macos_unified.sh --desktop`，而非重做已经通过的产品构建。
未完成 native 签名授权/验收之前不启动生产打包，不安装或发布 NAS。

旧 Book 请求 `b1f512c2-4cd9-4ec1-af38-1483ca235a2e` 只提交了 workspace-open，
没有附带终端、签名或后续命令。当前工具仅支持 terminal_cancel/transfer_cancel，
没有 workspace-open 的取消接口；未误用其它取消操作，未重发该请求。
