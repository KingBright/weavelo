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

## Studio 安装与 NAS 交付预检（2026-10-25 05:53 UTC）

本轮仅观察，没有 `--apply`、退出应用、切换 launchd、上传或权限修改。
实际 Info.plist 的版本为 `2.0.0`；上文 `2.0.0-alpha.9` 是产品版本，不应
将二者混为同一个字段。

| 当前实体 | 实测状态 |
| --- | --- |
| Desktop App | `~/Applications/Weavelo/Weavelo.app`，ID `fun.hackerlife.weavelo.desktop`，build `20261023.16` |
| Desktop 当前 leaf / DR | `8eaa97365d97b8a987cdf1232430eddf9aba8ecf`，DR 精确绑定该 leaf 和 Desktop ID，原生 strict/deep 验签通过 |
| Desktop 当前运行 | PID `27882`、`27890`；binary `c50d0cafc0cfa2055210fbe7565105c5580f15a95e1dc9f22256e2576ea76d9c` |
| 保留旧 App | `~/Applications/Weavelo.app`，ID `fun.hackerlife.weavelo`，build `20261017.1`，同一旧 leaf，验签通过，PID `41834` |
| Desktop profile / state | `~/Library/Application Support/Weavelo/Desktop` / `DesktopInstall`，均为本人所有的 0700 非链接目录 |
| launch policy | `~/Library/LaunchAgents/fun.hackerlife.weavelo.desktop.host.plist`，本人所有、0600、非链接文件 |
| 原安装事务 | `install.json: upgrade_completed`，`upgrade.json: completed`；原 source commit `e4a57619c4aea2e0d65790aa30a88414c6efc88b` |
| 新候选 | 同一 Desktop ID，build `20261025.9`，新 leaf `d38bedce6830fb01240f97c11c7984514ab715c0`，ZIP digest 与上文标准包一致 |

正式只读计划的实际返回（均 exit 1、`applied=false`、`not_applied`）：

- `python3 -B scripts/install_macos_desktop.py`：`Independent destination already exists; inspect its original installation, do not replay`。
- `python3 -B scripts/install_macos_desktop.py --upgrade`：原生 `codesign` exit 3，`test-requirement: code failed to satisfy specified code requirement(s)`。

因此这台 Studio 不是“新 ID 尚未安装”的情况：已有 Desktop ID 使用旧 leaf。
新建身份的批准与同签名普通升级不同；现有 upgrade 的 `inspect()` 对原 App
使用当前新 leaf 策略，按契约在任何安装写入前拒绝。现有
`migrate_macos_identity.py` 处理的是 `com.remoteplay.unified` 到旧
`fun.hackerlife.weavelo`，不能作为 Desktop 证书轮换入口。未改策略绕过检查，
未换目录启动候选，也未把旧 App 的实机表现当成新包验收。

原始精选公开元数据及两次只读拒绝回执：
`.rust-storage/validation-logs/studio-desktop-install-delivery-preflight-20261025.json`，
SHA-256 `b3cb8b20efa8bb1bd8a463c575540470f93a6fa8563167a9f8ad984497ee2e3e`。
未读取 membership secret 正文。

### 待评审的一次性 Desktop signer 迁移方案（尚未实现或执行）

AGENTS.md 要求 “Certificate rotation requires an explicit reviewed migration”
并要求签名/身份、部署实质变化前确认具体方案。本轮指令只允许现有正式兼容
路径；上述两个路径都拒绝，因此先提交如下具体方案供统一确认：

1. 在现有 `upgrade_macos_desktop.py` 事务中增加显式一次性迁移类型，保持普通
   升级路径不变；源只接受 Desktop ID + 旧 `8eaa…` leaf + 上表 binary /
   install / plist 实测摘要，目标只接受同 ID + 新 `d38b…` leaf + 已验证的
   `20261025.9` ZIP / manifest / compiler-source pins。拒绝任意 signer、
   appID、路径及环境覆盖，不将旧 leaf 加入普通新包允许列表。
2. 沿用本人锁、只读预检、正常 GUI/service quit、固定路径原子交换与版本绑定
   journal。更正前稿：现有 Desktop 升级成功后会删除交换留下的旧 staging，
   **没有现成的持久 rollback App**；迁移实现须补充固定的本人 0700 目录
   `~/Applications/Weavelo/.desktop-signer-rollback/`，保留未重签的旧
   `Weavelo.app`、原 install/upgrade journal、原 launch plist 及旧/新签名和
   文件树元组回执。该位置若已有未知内容则拒绝，不覆盖或清理；仅保留这一个
   回滚元组，不创建多代归档。迁移前独立验旧签名，提交前独立验新签名；
   崩溃/未知结果只能从原 journal 和实际 bundle 元组读回，不能重放 apply。
   rollback 明确使用源策略验证旧 bundle，不能用新策略误判或重签旧 App。
   交换后的受控失败可恢复此精确旧元组及原运行状态；未知结果保留新旧两端，
   先按原 journal 观察。完成后的回滚必须是独立显式、绑定该回执的操作，
   不自动删除已验证旧 App 或恢复应用运行后可能已变化的用户数据。
3. 保留 Desktop profile、设备成员身份和现有 launch plist；不复制/读出 secret，
   不修改旧 `fun.hackerlife.weavelo` App/数据。记录 membership 元数据及原
   launch 状态，拒绝并发变更，不新建第二 profile、网络 runtime 或 launch job。
4. fixture 覆盖错误旧/新证书、同名不同证书、manifest/source 冲突、并发 CAS、
   每阶段中断与 rollback、失败不发布、无 TCC/Gatekeeper/ACL 修改；按固定
   `./scripts/test_release.sh` 运行受影响检查。另做实际原生旧/新签名验证。
5. 经确认后，先输出含两端精确 pins 的只读迁移计划，再仅在 Studio 按原回滚
   流程执行一次并独立读回实际运行版本。新签名可能需要用户重新授权捕获/输入，
   也可能触发系统启动限制；若出现只报告真实系统界面并暂停该步骤，用户自行
   处理，绝不 reset TCC、免提示改 ACL、移除隔离属性或绕过 Gatekeeper。
   GUI/首帧/输入/音频/长时实机验收仍独立，不能由签名或 fixture 替代。

### NAS 非生产交付：未执行，路径尚未证实

固定 `scripts/publish_desktop_nas.sh` 默认是保留的旧公开下载通道
`/opt/weavelo/downloads` / `https://relay.hackerlife.fun:8443/download`；其
`verify-stage` 未传 `--desktop`，仍要求旧 leaf，不能把它当作新 Desktop 的
非生产交付。新 Weavelo relay 的现有记录为 `/opt/weavelo-relay`，`/download/`
返回 404，没有已确认的新 Desktop 下载映射。未调整公开目录、域名或服务。

一次固定 SSH 只读连接在执行远程命令前关闭（exit 255），没有上传。随后只经
本机 Octendra **正式默认 agent MCP 接口**读取已登记 NAS runtime 和定向知识：
2026-10-25 05:50:37 UTC，host `019ff60b-ecda-7311-9741-2605d723abf2`，
connector healthy，但已登记 `hackerlife.fun:222` 路由为
`ssh_handshake_failed`、transport disconnected、无 active session/workspace；
`weavelo desktop` 知识查询 0 项。没有重试不健康连接、重放旧未知写操作、
注册替代 route、读取 vault、创建远程 workspace 或发起任何上传。

待原 NAS 负责人恢复已登记路由并确认一个**既有非生产目录及对应 Desktop
包接收契约**后，才能按源/ZIP/manifest 精确 pins、拒绝覆盖、原操作状态读回
推进一次交付；若不存在，需先评审该独立契约，不能复用旧公开通道冒充交付。

## 一次性迁移候选实现（已获实现授权，未安装）

用户后续明确授权直接完成目标内实现细节；主任务据此批准先实现和测试上述
具体迁移方案，提交后独立评审，再决定实际安装。创建新证书的批准不重复询问。
上节“尚未实现”的文字保留原评审时点；当前状态以本节为准。

- `deploy/macos/desktop-signer-migration.json`：唯一 Studio home/UID、Desktop
  ID、旧/新 leaf、源/目标 build、两端 binary/树、旧 install/upgrade/plist 以及
  已交付 ZIP/manifest/compiler source 的精确公开 pins。无 CLI 或环境覆盖入口。
- `scripts/migrate_macos_desktop_signer.py`：单独的 plan/apply/status/rollback
  事务；共用原 installer 锁、原生签名/归档/实际产品身份检查、正常退出和原子
  exchange。旧 App 只用旧策略验证，候选只用新策略验证；普通 `check_upgrade`
  及 `upgrade_macos_desktop.py` 未放宽、未更改。
- `scripts/install_macos_desktop.py --migrate-signer`：显式分派到此迁移，禁止
  同时使用 `--upgrade`，保留迁移 CLI 的失败退出码。
- `scripts/tests/test_migrate_macos_desktop_signer.py`：25 项 disposable fixture，
  包括 9 个前向阶段、6 个回滚阶段的中断矩阵，原生 macOS 目录 exchange，
  交换成功但未写回执、registry 已提交但进程中断、拒绝正常退出、精确记录字节
  恢复、未知/篡改/硬链接/重定向/CAS 拒绝及普通升级仍拒绝更换 signer。

固定回滚目录在 exchange 前用于本次候选暂存，exchange 后保存原旧 App；完成
不会删除它。只保存一个 App、三个原元数据文件及原事务 journal，不复制
profile/secret，不扩展多代归档。正常安装仍是一套 canonical App。已完成后的
显式回滚保留当时用户已变化的 profile 和组状态，不用旧数据快照覆盖。

前向 apply 不支持 resume/replay。任何已存在的迁移 journal/回滚目录都会阻止
再次 apply；rollback 必须先读 status 获取原 transaction digest，再验证实际
新/旧 bundle 元组、原备份和 registry CAS。status 单独报告实际 active/rollback
文件树、原生签名和 registry 一侧，journal 阶段本身不充当实物证明。部分
pre-stage 写入尚未凑齐完整可验证备份时，保留现场并拒绝自动清理/回滚；此时
未请求应用退出或交换。其它未知/不匹配状态也保留，不猜测恢复。

以下为**待独立评审后的操作入口说明，本轮没有执行真实迁移 plan/apply**：

```sh
python3 -B scripts/install_macos_desktop.py --migrate-signer PACKAGE.zip
python3 -B scripts/install_macos_desktop.py --migrate-signer PACKAGE.zip --apply --expected-plan-sha256 REVIEWED_PLAN_DIGEST
python3 -B scripts/install_macos_desktop.py --migrate-signer --status
python3 -B scripts/install_macos_desktop.py --migrate-signer --rollback --expected-transaction-sha256 OBSERVED_TRANSACTION_DIGEST
```

新签名的系统权限和实际启动限制仍未知；实现没有 TCC/ACL/信任/Gatekeeper
写入或免提示逻辑，不代操作安全界面。不重签旧 App，也不调用签名私钥。
候选包的编译源码仍为 `d722c5a9…`；本轮只改部署机制，不重建/重包产品，不能
把新脚本提交号写成旧候选的编译源码。

验证记录：首次全量运行在受限执行环境出现 19 个 error、1 个 failure（home
临时 fixture、`ps` 沙箱限制及 `/var` 链接路径）；没有改测试来掩盖它们。
使用 `TMPDIR=/private/tmp PYTHONDONTWRITEBYTECODE=1 ./scripts/test_release.sh`
及获准原生 fixture 进程观察后，842 项、23 条件跳过、零失败。真实签名包的
独立只读验证还发现 archive report 多出 ZIP digest/size 字段，已修正为完整
App 签名字段逐项比对并单独验证固定 ZIP digest，fixture 同步使用真实报告形状。
随后加入原生核验所得完整目标树 pin，并重跑固定回归；最终日志为
`/tmp/weavelo-desktop-signer-release-tests-final.log`。

最终原生只读核验通过旧/新精确 leaf/DR、旧完整树、候选 ZIP/source/完整树及
实际 `--product-info-json`，未运行迁移 plan/apply。证据
`.rust-storage/validation-logs/studio-desktop-signer-migration-native-final-20261025.json`，
SHA-256 `e2c105e15df0e278e2cbb803181a42df8909e3eec906fb06bde29cbf45b37bdf`；
最终公开契约 canonical digest
`f5c2dc8a4dcf321b66d18859d4cff340f3caac15a10d0e59de43579d92b8dd44`。
本轮候选实现待独立评审；实际安装、启动后系统权限、GUI/视频/输入/音频及
真实回滚仍未验收。没有以 disposable exchange fixture 代替这些实机项目。

### NAS 私有候选准入更新（06:11 UTC，仅本地就绪核对）

协调者提供 06:07 独立读回的新私有接收契约，目标为
`/volume1/@entware-opt/remoteplay/release-evidence/desktop-candidates/macos-arm64-20261025.9-d722c5a9a2a9-e9858f65390b/`。
目录 0700、准入/策略文件 0600，无公开 Caddy 映射，overwrite=deny。
`ADMISSION.json` SHA-256
`f3ef665f3c602a915356af7ef24ef0f72c41449fad92d95d67c7b0fce709e77c`。
本地原 ZIP 10487048 bytes / `e9858f…` 与 verification JSON 1042 bytes /
`4431a4…` 均匹配该独立回执；仍是原 `d722c5a9…` 编译候选，不是本轮迁移
脚本提交的重编译包。

已发现正式 `octendra_upload_file` 支持 exact SHA、mode、overwrite=deny，但它
依赖原登记 pooled SSH 路由。06:11:20 只读 runtime 仍为
`ssh_handshake_failed` / disconnected，connector healthy，未证明恢复。
本任务因此没有读取远端最新准入、排队上传、重试失败 SSH 或替换 route；
远端 payload 缺失只在协调者 06:07 回执中得到确认，不声称当前已交付。
由 NAS 协调者使用其已验证路径接续，或先恢复原登记上传路线。

本地核对回执
`.rust-storage/validation-logs/studio-desktop-private-delivery-readiness-20261025.json`，
SHA-256 `41f0c2bb6cbc5f52f26e3cadf99507194188242324ac86eb2a3c5f43caac1001`。

## 323f1f1a 独立评审阻断项修复（候选，仍未安装）

原候选未执行真实 apply。主任务独立评审指出固定 O_EXCL pending 在函数内部
崩溃时可能阻塞回滚，以及 completed 后后台已退出无法保留停止态；另要求补全
目录持久化。本轮按该明确指示修复，继续等待独立评审，不自行开始安装。

- P1：`atomic_record` 保留普通路径遇已存在 pending 即拒绝的默认行为；拆出
  完整私有文件比较和原子 byte 写入，仅一次性迁移的显式 rollback 使用
  `recover_matching`。持原安装锁、核对原 transaction digest、新/旧实际签名树、
  备份和 registry CAS 后，才识别同一 plan/不可变字段/相邻阶段的完整 journal
  pending，并原子提交该**进度元数据**，不执行任何前向 App 动作。完整 forward
  registry pending 只按确切已绑定新记录字节退役，不顺向发布；完整原 registry
  restore pending 可直接完成恢复。部分写入、非 canonical、错事务/阶段/元组、
  未知字段、链接或非私有文件均保留拒绝，不清理未知内容。首次回滚的 pending
  运行形态还须匹配当前只读观察；外部活动已改变该观察时暂停，不猜测恢复。
- P2：运行快照分别保留 loaded、background 和 GUI。前向迁移仍只接受原有
  已登记运行后台；completed 后的显式回滚允许后台已停止，保留 loaded 或
  unloaded 状态，不 bootstrap/unload launchd、不为健康检查唤起已停止后台。
  有 GUI 才按原 `--gui` 入口恢复并独立观察，只有请求后台运行才调用原 start；
  未登记的手动后台或过程中 loaded 状态改变仍拒绝处理。
- 持久化：完整 copytree 验证后 fsync 每个 regular file，目录自底向上及其父
  目录；交换前同步保留的旧树，原生 exchange 后同步源/目标双方父目录。
  错误时保留 journal 和新旧实物，不宣称完成。没有 F_FULLFSYNC 或实际断电测试，
  因此只确认代码及 disposable fixture 的调用/中断语义，**不声称断电恢复已验收**。

新增函数内部低级 `os.write`、文件 fsync、replace、父目录 fsync 前后中断矩阵：
5 类记录 × 7 个完整边界共 35 个案例，以及 5 类部分写入案例；覆盖首次回滚
checkpoint、前向 checkpoint/registry、回滚 checkpoint/原 registry 恢复。
还覆盖 terminal rollback pending 只观察不重复启停、linked/错事务 pending
保留、停止态 loaded/GUI 四组合与中断恢复、文件/目录同步顺序、双方父目录同步。
这些案例不是物理断电或真实产品服务验收。

测试计数明确区分：定向 **total 37 / pass 37 / skip 0**；固定
`TMPDIR=/private/tmp PYTHONDONTWRITEBYTECODE=1 ./scripts/test_release.sh` 为
**total 854 / pass 831 / skip 23 / failure 0 / error 0**。旧一轮记录的 842 是
total，实际 pass 819、skip 23，不应称 842 全部通过。
最终日志 `/tmp/weavelo-desktop-signer-review-release-tests.log`，SHA-256
`84315ec7633e40b437c53bdcd341ad4a05b7fa8bd397a02bcfcd262b15e44559`。
源码摘要及回执
`.rust-storage/validation-logs/studio-desktop-signer-review-fixes-20261025.json`，
SHA-256 `32bcf9316165bf6a95fe7fc9096b5166f01afc35a9fb38057a07731d95954b44`。

仍未实际安装/回滚或操作系统权限；现有身份和数据目录绑定不变，不重建或重签
候选产品。父任务同时报告 NAS 协调者已在 06:17:49 完整读回交付的两个私有候选
文件；本任务依此停止上传方向，不重复传输，该交付读回不是本任务独立执行。

## 二次评审 GUI-only 最小补修与真实只读 plan

二次独立评审确认：旧 `e4a57619` 的 attached GUI 在建窗口前需要连接后台，
所以前轮 fixture 用 mock 将 GUI 直接改为存在，不能证明“后台停止但 GUI
存在”的状态可以恢复。此前“四组合均支持”的描述作废。当前只支持后台运行
时按原逻辑恢复 GUI，或**无 GUI、后台停止**时保留 loaded/unloaded；不扩展
产品启动能力。

`validate_runtime_record` 显式拒绝 GUI-only。rollback 在任何 pending 退役、
checkpoint 写入、应用退出或文件交换前验证当前运行形态；前向 apply 同样在
创建迁移 journal/staging 和退出前拒绝。删除停止后台分支中的 `open --gui`
与等待循环，不再试图恢复无法建立的窗口。
测试覆盖 loaded/unloaded 两种 GUI-only 状态及有/无原 pending，验证旧/新
bundle 树、install/upgrade/journal/pending/plist 字节、membership 元数据及
运行形态均不变，且没有 stop/start/exchange/launch。无 GUI 停止态的两种
loaded 状态和中断恢复继续通过。

本轮执行了新显式迁移机制的**真实只读 plan**，没有重试此前被拒绝的普通
install/upgrade plan，也没有 apply。归档、源码、原/新证书及固定当前安装 pins
全部通过。运行状态通过原认证 CLI 单独只读观察，不是截图、首帧或 GUI 点击
验收，也不是 plan 对未来运行状态的保证；执行路径仍会重新检查。

| 输入/观察 | 实际结果 |
| --- | --- |
| Bundle ID（两端相同） | `fun.hackerlife.weavelo.desktop` |
| build | 当前 `20261023.16` → 候选 `20261025.9` |
| 旧 leaf SHA-1 | `8eaa97365d97b8a987cdf1232430eddf9aba8ecf` |
| 新 leaf SHA-1 | `d38bedce6830fb01240f97c11c7984514ab715c0` |
| 候选编译源码 | `d722c5a9a2a974e1ceb0ab7e0712f699fe11e021` |
| ZIP SHA-256 | `e9858f65390b0c5ed5f33a0a0cd1e75ca7d80153efb083569d423aed76be7165` |
| 固定契约 canonical digest | `f5c2dc8a4dcf321b66d18859d4cff340f3caac15a10d0e59de43579d92b8dd44` |
| plan digest | `bc3c0af7b6c89df6ba29118f2dc72e88c15863bd055c079afc5f6a5bdc80e2ba` |
| 当时运行状态 | loaded=true，后台 PID `27882`，GUI PID `27890`，GUI-only=false |
| applied | false |

完整固定当前 executable/info/tree/install/upgrade/plist pins、候选
source/identity/manifest/完整树、代码文件摘要和实际 plan 保存在
`.rust-storage/validation-logs/studio-desktop-signer-gui-only-readonly-plan-20261025.json`，
SHA-256 `6ab7543e9500fb2e146dacefbeb63061c9c9a8c790a92ddacf25edc24c8c17a7`。
本轮源码摘要与该只读 plan 记录一致。

定向 **total 39 / pass 39 / skip 0**；固定发布回归
**total 856 / pass 833 / skip 23 / failure 0 / error 0**，最终日志
`/tmp/weavelo-desktop-signer-gui-only-release-tests.log`。没有实际迁移/回滚、
权限变化、NAS 操作或新签名/打包；候选继续等待独立评审。
验证回执 `.rust-storage/validation-logs/studio-desktop-signer-gui-only-fix-validation-20261025.json`，
SHA-256 `4dadbadd00bd2b2d53e82181f0180a2535f74492a266112a20aacb9fa29d1ce4`。

## Studio 显式迁移已执行（2026-10-25 07:04 UTC）

主任务独立复核 `648feccb` 后确认原 P1/P2 无剩余阻断，明确调度一次受控显式
迁移。没有重走此前拒绝的普通 install/upgrade。执行前持原安装锁重新检查无
未知 journal/pending/rollback 冲突，旧安装/双 cert、`d722c5a9` ZIP 及完整树
和 reviewed plan `bc3c0af7…` 完全一致。原 GUI/background 重新认证得到
`27890` / `27882`（不是直接沿用历史 PID）。

正式 `install_macos_desktop.py --migrate-signer … --apply` 只启动一次：
2026-10-25 07:03:58.225605 UTC，PID `54659`；07:04:02.517367 UTC，exit 0，
stderr 0 bytes，`phase=completed`、`applied=true`、`rollback_retained=true`。
原控制器确认正常退出后交换；未 force quit、重放 apply、执行 rollback 或修改
TCC/ACL/Gatekeeper/系统信任，未调用签名私钥。安装不等于新证书的捕获/输入/
音频权限已经获准。

| 独立读回 | 实际结果 |
| --- | --- |
| 当前 canonical App | `~/Applications/Weavelo/Weavelo.app`，build `20261025.9` |
| 当前 leaf / DR | Desktop ID + `d38bedce6830fb01240f97c11c7984514ab715c0`，原生验签通过 |
| 当前 executable SHA-256 | `301f80c39e9b8077194f037937c814861a474e794f3cc7e05707f84eaad791b2` |
| 当前完整树 | `b7b78985a5a3d8d170c7edabc1f9116e98e74b3f5c3b6f95b5a62f9a5e904abc` |
| 实际编译源码 | `d722c5a9a2a974e1ceb0ab7e0712f699fe11e021`，实际 `--product-info-json` 匹配 |
| 默认 GUI | `restored-original-gpui`，compiled=true，无诊断后端替代 |
| 实际运行 | loaded=true；后台 PID `54713`、frontend PID `54719`，均认证到新 executable |
| 原 profile / 组 | 原 membership inode/size/mtime/ctime/mode/uid 元数据一致；启动前后运行组摘要一致 |
| 原 launch plist | SHA-256 `113253c0990cdc224f1a25c9d9025bb0731dd6aabcbfe4dedb0abb509cc7ea54` 保持 |
| 保留旧客户端 | `~/Applications/Weavelo.app` 原 executable 摘要保持，未替换 |
| 持久回滚 | `.desktop-signer-rollback/Weavelo.app` 为原 `20261023.16`、旧 `8eaa…` leaf/DR；完整树 `2595dc59…` 原生验签通过；三个原元数据文件精确匹配 |
| 新 install registry | SHA-256 `f185a9301bd18a9c472f5623b6db4252a4b75a73e206727aeaa19aea96520668` |
| 原 transaction digest | `ca2c20824ab9cca46a35f28023dbbeb36a2c089b697638765c2022360733bfa0` |

### 实际 GUI 与基本连接观察

正式 CUA 绑定 **Desktop bundle ID** 的新 App，实际截图显示 Weavelo 窗口、
原深色 GPUI、Device & Network Control 抽屉、Devices/Pairing/Files/Security/
Settings 标签、`This device: MacStudio.local` 和
`Ready for incoming and outgoing sessions`。点击 Edit name 实际打开名称编辑，
可见原值、Save name / Use host name / Cancel；点击 Cancel 再回 Devices 成功。
没有保存或恢复名称值，之后再次独立检查 profile 元数据及 plist 未变。

**GUI 观察中的意外曝光：** Edit name 会转到 Pairing 页，该页同屏显示邀请凭据；
该次 CUA 截图意外包含它。立即取消并回 Devices，已观察到凭据页关闭。
没有复制、使用、转写邀请码或把该截图存入仓库/证据文件，后续不再截取该页；
没有擅自轮换组或凭据。这里不声称截图未产生，也不把该交互当远程输入验收。

后台及 GUI 的 owner/projection 一致：routes/unique_devices/online_devices 均
为 0，Devices → All 实际显示无设备。控制和发现 relay 均 connected/active、
HTTP 101，各 1 次 TLS/WebSocket upgrade，各 50 次 ping/pong，观察时最近 pong
约 1360 ms，无 relay last_error。LAN discovery running=true，但 UDP 有 1 次
send error（250 sends），另收到/解码 500 个 self packet；没有足够诊断将其
归因于权限，未修改任何系统设置。Relay discovery 251 sends、0 send errors。

因此基本 GUI 渲染/点击和 relay 连接有实际证据；**当前没有同组在线对端可发起
端到端会话**。远端视频首帧、输入、音频、文件互传及长时验收均未完成，不以
relay/WebSocket 成功代替。需一个既有同组设备在线后接续，不创建假 peer 或改组。

### 本次原始回执（本地，未上传敏感 UI）

| 文件（`.rust-storage/validation-logs/` 下） | SHA-256 |
| --- | --- |
| `studio-desktop-signer-migration-preapply-20261025.json` | `9b7cae680193c24cbbd569c657369f9904ab92c0e02532c80ce2ce86cc614fe4` |
| `studio-desktop-signer-migration-apply-20261025.stdout.json` | `80da0bceee47c0366905c1b450fe12bb3711ec0ce644f5fb48a97043e409b868` |
| `studio-desktop-signer-migration-independent-installed-20261025.json` | `4acf250c9a99511c0823418fba096d45a3326eacfdde6f695627207a52fa43ce` |
| `studio-desktop-signer-migration-gui-network-20261025.json` | `4a255110aa7b7cd7388a5990745e635f18ab4dc32875a69f07e7eb6cc68721d8` |
| `studio-desktop-signer-migration-delivery-outcome-20261025.json` | `85f1e8f3ce7bf325c3a4caf34226dfca17798843bed9b2c15593e404bb513d04` |

本阶段没有产品源码或打包变更，没有重建/重签候选、没有重复 NAS 交付。
固定代码回归保持此前记录的 total 856 / pass 833 / skip 23；本次另列真实安装
与 GUI/连接证据，未将 fixture 计数充当实机测试计数。
