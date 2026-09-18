# ikvmt

[English](README.md) | 简体中文

面向人和 AI Agent 的原生带外控制台工具。通过 BMC 的 HTTP/WebSocket 获取服务器 VGA 图像、发送键盘输入；使用 Rust 实现，运行时无需浏览器、Node、Python 或模型 API。OCR 默认关闭；开启后使用内置的纯 Rust `ocrs + RTen`，模型随二进制打包。Tesseract 仅保留为显式选择的对照引擎。

当前适配 **Supermicro X10DRT-H / BMC 固件 4.00 / IPMI 2.0**，已在真实设备验证截图、键盘、持续会话和重连。固件版本决定 HTML5 iKVM 私有协议；这里的 `fw4.00` 不是 IPMI 协议版本。其他平台尚未验证，不自动探测或切换厂商。

```text
AI Agent / 人 → ikvmt → BMC HTTP + WebSocket → 宿主机 VGA / 键盘
                ↑ PNG 图像、输入提交结果
```

工具提供 `open → observe → act → observe → close`。Agent 查看图像、理解界面并决定下一步。输入提交成功不等于命令完成；截图和 OCR 也不等于逐字节完整的 stdout。首版不提供隐含这些保证的 `run(command)`。

## 构建与单张截图

```sh
cargo build --release --locked
./target/release/ikvmt --help

# 通过服务环境提供 BMC 密码，避免放入参数、JSON、源码或日志。
# 在 zsh 中可使用隐藏输入：
read -rs 'IKVM_PASS?BMC password: '
export IKVM_PASS

./target/release/ikvmt shot 192.0.2.10 --insecure --output-dir artifacts
```

`shot` 返回 PNG 绝对路径和元数据，然后关闭连接；不会向宿主机发送按键。示例 IP 需替换为实际 BMC 地址。`--insecure` 为自签名证书关闭证书校验，默认启用校验。用户名默认 `ADMIN`，可用 `--username` 或 `IKVM_USER` 修改。可选 `--ocr` 使用内置 OCR；`--ocr --ocr-engine tesseract` 才会调用本机 Tesseract。OCR 失败不影响原图返回。

要求 Rust 1.94 或更新版本，已在 macOS arm64、Rust 1.95 构建验证。程序使用系统 TLS；Linux 构建通常需要 OpenSSL 开发依赖，尚未跨平台实测。`ipmitool` 仅用于开发时独立核对设备信息，不是运行依赖。发布二进制位于 `target/release/ikvmt`。

## Agent 接入

项目内 [SKILL.md](SKILL.md) 是 Agent 的英文操作入口，中文说明见 [SKILL.zh.md](SKILL.zh.md)。以下两种方式都使用同一 JSON Lines 协议：每行一条请求，每行一条响应，诊断写 stderr。它是本地服务协议，当前未实现 MCP 协议适配。

直接让 Agent 管理持久子进程的 stdin/stdout：

```sh
./target/release/ikvmt serve --output-dir artifacts
```

需要跨 CLI 调用复用会话时，先在一个终端启动本地 Unix socket 服务：

```sh
mkdir -p artifacts/runtime
chmod 700 artifacts/runtime
./target/release/ikvmt serve \
  --socket "$PWD/artifacts/runtime/control.sock" --output-dir artifacts
```

然后在其他终端调用：

```sh
./target/release/ikvmt call console.open \
  --socket "$PWD/artifacts/runtime/control.sock" \
  --params '{"target":"192.0.2.10","username":"ADMIN","insecure":true}'
```

BMC 密码必须在 **serve 进程的环境** 中；`call` 的环境不会传入已启动的服务。多台 BMC 可在 `open` 使用 `password_env` 指定不同环境变量，默认 `IKVM_PASS`。宿主机密码与 BMC 密码分开管理。

一条完整的 stdin 请求和响应外层结构：

```json
{"id":1,"method":"console.open","params":{"target":"192.0.2.10","insecure":true}}
```

```json
{"id":1,"result":{"session":{"session_id":"…"},"observation":{"observation_id":"…","image":{"path":"/absolute/path.png"}}}}
```

上面的响应为结构简写。错误返回 `{"id":1,"error":{"code":"…","message":"…"}}`；`call` 在错误时退出码为 1。`id` 仅用于请求响应关联，输入去重使用 `params.request_id`。

## 交互接口

| 方法 | 参数与行为 |
| --- | --- |
| `console.open` | `target` 必填；`username` 默认 ADMIN，`password_env` 默认 IKVM_PASS，`profile` 默认 `supermicro-x10-fw4.00`，`insecure` 默认 false。返回会话及首张观察 |
| `console.observe` | `session_id`；可选 `ocr: "on"` 和 `wait: {since: "观察编号", timeout_ms: 5000}`。最多等待 10 秒像素变化，超时也返回观察 |
| `console.act` | `session_id`、`request_id`、`based_on`、`actions`；默认等待 500 ms 后观察，可用 `observe_after: {delay_ms: 1000, ocr: "off"}` 调整，或 `false` 关闭 |
| `console.reconnect` | `session_id`；关闭旧连接、增加 `connection_epoch`、重建连接并观察，不重放键盘输入 |
| `console.close` | `session_id`；释放连接。可重复调用；不输入退出命令、不关闭宿主机 |
| `console.status` / `console.list` | 前者需要 `session_id`；后者无参数，列出本服务进程的会话状态 |

根据 `open` 返回的 PNG 实际查看控制台，再把获得的编号填入动作文件 `act.json`：

```json
{
  "session_id": "从 open 取得",
  "request_id": "为这次输入生成唯一编号",
  "based_on": "所依据的 observation_id",
  "actions": [
    {"type": "text", "text": "uname -a"},
    {"type": "key", "key": "Enter"}
  ],
  "observe_after": {"delay_ms": 1000, "ocr": "off"}
}
```

```sh
./target/release/ikvmt call console.act \
  --socket "$PWD/artifacts/runtime/control.sock" --request-file act.json
```

输入动作：

- `text`：仅输入可打印 US ASCII，不自动补 Enter。不支持 Unicode、换行或剪贴板。前提是宿主机当前使用 US 键盘布局，CapsLock 状态由调用方观察处理。
- `key`：一次按下和释放。支持 Enter、Escape、Tab、Backspace、方向键、Home/End、PageUp/PageDown、Insert/Delete、F1–F12、修饰键等。
- `chord`：如 `{"type":"chord","keys":["Control","c"]}`，按序按下、逆序释放。
- `secret`：如 `{"type":"secret","env":"HOST_PASS"}`，从服务环境读取后按文本输入，不回显凭据，不补回车。只有看到正确输入提示后才使用。

每次最多 32 个动作、累计 4096 个文本字符。所有字符与按键在发送前完成验证；按键事件间隔至少约 30 ms，长文本会较慢。输入过程中持续处理视频更新。

BIOS 等较慢界面可在 `console.act` 顶层设置 `"key_event_interval_ms": 150`，调整每次按下/释放报文之间的间隔。默认 30 ms，允许 30～1000 ms；不合法值在发送前拒绝。实测该 BIOS 在 30 ms 下偶发漏掉连续方向键，应使用较慢节奏并观察实际选择项。它与输入结束后等待截图的 `observe_after.delay_ms` 是两个独立参数。

## 可选原生 OCR

`console.observe` 的 `ocr` 及 `console.act.observe_after.ocr` 支持 `off`（默认）、`on` / `ocrs`（内置引擎）、`tesseract`（外部对照）。选择内置引擎时无需 Tesseract、Python、模型 API 或额外下载；不会自动降级到外部引擎。

也可对已有截图离线识别，便于对同一帧交叉验证：

```sh
./target/release/ikvmt ocr /absolute/path.png
./target/release/ikvmt ocr /absolute/path.png --engine tesseract
```

内置引擎返回 `engine: "ocrs"`、`runtime: "rten"`、`models: "bundled"`、`text`、`lines: [{text, bbox: [x, y, width, height]}]` 和 `elapsed_ms`。位置框使用原始图像坐标并裁到图像边界；左右列可能分别输出，不应按文本顺序直接关联 BIOS 项目与值。没有选中状态或识别置信度字段。

`status: ok` 仅代表推理完成；可能误读字符、漏行，也不能识别所有选中项。BIOS 修改须结合原图核对标签、值、位置和高亮。模型以拉丁字符为主，不保证中文或精确终端转录。实测见 [验证记录](docs/TESTING.zh.md)。

模型首次使用时加载，后续在服务进程中复用；约 12.2 MB 权重内嵌到二进制。权重采用 CC BY-SA 4.0，随含模型的程序分发时须附带 [来源与归属](models/README.md) 和 [模型许可证](models/LICENSE-CC-BY-SA-4.0.txt)。项目代码仍为 Apache-2.0。

## 结果含义与重试

观察包含 PNG 路径、尺寸、像素 SHA-256、`observation_id`、连接代次和输入修订号。`exported_at_ms` 是本地导出时间；`video.last_update_received_at_ms` 与 `update_seq` 表示已解码的视频更新。连接中断时旧图标记 `image.source: cached`；无信号时图像为空。`wait_satisfied` 不表示命令已完成，光标闪烁也会满足像素变化条件。

`input.status: submitted` 只表示所有键盘报文已提交。`partial`、`unknown` 表示可能已输入一部分；先观察现场，再决定恢复动作。`not_submitted` 表示未提交输入。不自动清屏、Ctrl+C、登录宿主机或重发 Enter。

同一存续会话内，相同 `request_id` 和完全相同负载返回缓存结果，不再次输入；编号相同但负载不同返回 `REQUEST_ID_CONFLICT`。已发生其他输入或重连的旧观察返回 `STALE_OBSERVATION`。服务重启后无去重记录，不能盲目重试。截图失败也保留已发生输入的结果。

`open` / `reconnect` 没有请求去重；若响应丢失，先调用 `console.list` / `console.status` 查看状态。服务按请求串行执行，视频接收在独立线程中持续运行；当前不是可取消的异步任务系统。

完成后明确调用 `console.close`。stdin 服务收到 EOF 会清理会话；Unix socket 服务应先关闭会话再终止进程。异常杀进程可能留下暂时的 BMC 占位及本地 socket 文件；确认服务已退出后再删除自己创建的旧 socket。程序不会自动覆盖已有 socket。

## 范围与限制

- 当前验证的是单一机型和固件。已在四台同型机器实测 Linux 重启、POST、BIOS 菜单导航、保存设置及返回 Proxmox 登录界面；进入 BIOS 的一次性启动标记由 ipmitool 设置。未实现 BIOS 自动规划、鼠标、电源管理、虚拟介质或 SOL。
- AST2100 编码 87 已真实验证；部分未知块格式会明确报错。编码 88 只接受完整 JPEG 负载，尚未设备验证。
- 图像只反映可见屏幕，不能无损提取滚屏前内容、区分 stdout/stderr、获得退出码或证明长命令输出完整。可分页或缩短输出，由外部模型逐屏判断；可靠字节回传需要另建明确协议。
- OCR 是可选辅助，实测仍会误读符号和字符，不作为完整性保证。
- 没有自动空闲回收、磁盘配额或持久化恢复；调用方负责关闭会话和清理截图。PNG 在 Unix 上以 0600 创建，socket 仅限本机用户访问。截图可能包含控制台中的敏感内容。
- 同一服务对完全相同的目标字符串拒绝第二个活动会话；不要用 IP/主机名等不同别名并发连接同一 BMC。

## 开发

```text
src/interface.rs                    JSON Lines 服务
src/console.rs                      会话、观察、输入
src/ocr.rs                          原生 OCR 与显式 Tesseract 对照
models/                             内嵌权重及来源、许可
src/vendor/supermicro/x10_fw_4_00/   认证、私有 RFB、键盘、AST 解码
SKILL.md                            Agent 英文操作入口
SKILL.zh.md                         Agent 中文操作说明
docs/protocol-x10.zh.md              协议说明
docs/TESTING.zh.md                   验证记录
```

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

真实 BIOS 截图与标注只保存在本地，不随仓库发布。有对应本地夹具时，可按 [夹具说明](tests/fixtures/README.zh.md) 运行真实模型测试；自己的截图目录可用下列命令离线回放，检查识别状态和文字框边界：

```sh
cargo run --release --locked --example ocr_corpus -- /path/to/screenshots /tmp/ikvmt-ocr.jsonl
```

架构与后续边界见 [设计说明](docs/architecture.zh.md)。OCR 微调是可选增强，评估和数据来源见 [微调评估](docs/OCR-FINETUNE-EVALUATION.zh.md) 与 [数据筛选](docs/OCR-DATA-SOURCES.zh.md)。旧 JS 实现和浏览器移植提案已移除，可从 Git 历史查阅。

英文文档使用 `.md`，中文文档使用 `.zh.md`。提交标题与正文统一使用英文。

[Apache License 2.0](LICENSE)
