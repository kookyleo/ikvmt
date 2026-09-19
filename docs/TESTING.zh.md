# 验证记录

[English](TESTING.md) | 简体中文

控制台基线：2026-09-18；虚拟介质与发布复核：2026-09-19。客户端：macOS arm64、Rust 1.95。参考硬件：四台 Supermicro X10DRT-H，BMC 固件 4.00 / IPMI 2.0 / BIOS 3.3。协议与交互验证均由原生 Rust 程序完成，运行时未启动浏览器或执行固件 JavaScript。

## 可重复的本地检查

```sh
cargo fmt --check
cargo test --locked --offline
cargo clippy --all-targets --locked --offline -- -D warnings
cargo build --release --locked --offline
# 以下两项需要本地截图夹具，新 checkout 不包含图片
cargo test --release --locked --offline --test ocr_live_fixture -- --ignored
cargo run --release --locked --offline --example ocr_corpus -- tests/fixtures /tmp/ikvmt-ocr.jsonl
```

2026-09-18 基线的 fmt、Clippy、release 构建、22 项单元测试及 1 项 release 模型测试通过。release CLI 帮助、JSON Lines 错误隔离/后续请求和 Skill 格式校验通过。

单元测试覆盖 AST 解码及增量帧完整性、RFB 分片/合并和坏报文、票据/Cookie 解析、键盘映射、整批输入预检、请求去重、旧观察拒绝、截图失败不重放输入、OCR 选项及文字框边界、JSON Lines 错误隔离。

1 项显式 release 模型测试用真实 BIOS 图片检查高亮行、弹窗文字与框坐标。`examples/ocr_corpus.rs` 对目录内 PNG 逐图执行原生 OCR，校验状态和坐标范围，输出 JSONL。该检查不等于字符准确率评测。

本地完整截图集（现已移至仓库外的运维目录），有 161 张图、74 张像素唯一图，本轮全部回放通过，结果保存在 `artifacts/ocr-data-research/cleanup-corpus.jsonl`。曾发现边缘文字框 `[-2, 17, 129, 14]` 越界，现将框裁到原图范围并排除无交集的框；这不改变识别模型，也不声称修正了文字内容。

## 真实控制台交互

| 检查 | 已观察到的结果 |
| --- | --- |
| 登录、只读截图与正常关闭 | 取得完整 VGA 图像，关闭释放 BMC 会话，不发送宿主机退出 |
| 大小写、数字、符号和单 Enter | `printf` 标记与 `uname -s` 输出在原图中正确显示 |
| 持续连接 | 同一会话约 130 秒后输入仍有可观察响应 |
| 重试与预检 | 重复 request_id 返回缓存；负载冲突、旧观察、非法字符在输入前拒绝 |
| 重连 | connection_epoch 增加；保留宿主机画面，不重放命令 |
| 重启和 BIOS | 完成 POST、方向键/Enter/Escape/F4 导航、设置保存和回到 Proxmox 登录界面 |
| 分辨率切换 | 实际观察到系统控制台与 800×600 BIOS 之间切换 |
| BIOS 慢速按键 | 30 ms 下偶发漏步；使用 150 ms 事件间隔及 2 秒观察延迟完成操作，仍逐屏核对 |

一次性进入 BIOS 的启动标记由外部 ipmitool 设置；它不是 ikvmt 的内置电源能力。四台机器同型，不能据此宣称覆盖其他固件。BIOS 配置操作证明交互可用，未通过编译基准测量性能收益。

## OCR 同帧对照

在 36 张 BIOS 图片上分别运行内置 ocrs + RTen 与 Tesseract 5.5.1 / PSM 6，并由调用方视觉模型查看原图核对。内置引擎能读出对照引擎漏掉的多个灰底白字选中行和弹窗选项，但仍会误读 `HW_ALL`、`C0`、`Performance` 等；两个引擎均出现过终端 IP 或主机名误读。产品不输出选中状态或校准置信度。

| 引擎 | 最小耗时 | 中位数 | 最大耗时 |
| --- | ---: | ---: | ---: |
| ocrs + RTen | 229 ms | 322 ms | 479 ms |
| Tesseract 5.5.1 | 220 ms | 356.5 ms | 587 ms |

这是现场记录的后端耗时，非受控性能基准；Rust 复用模型，Tesseract 每图启动子进程，不含 BMC 截图等待或外部模型判断。未对整套图片逐字标注，不报告总体准确率。

使用 `PATH=/nonexistent` 运行 release 二进制的原生 OCR 仍可识别，确认不依赖外部 OCR 程序。模型及许可见 [models/README.md](../models/README.md)，本地菜单夹具的用途见 [fixtures](../tests/fixtures/README.zh.md)。截图及标注不随仓库发布。

## 未覆盖与本地证据

仍未验证：真实网络故障恰好发生在按下/释放之间、BMC 重启、独立无信号恢复、其他键盘布局、编码 88、其他固件、跨平台构建及长时间压力运行。未实现可取消异步任务、崩溃恢复、空闲回收或逐字节 shell 输出回传。

具体机器配置、凭据及逐机运维记录已移至本机仓库外的运维目录。产品协议调试、OCR 对照 JSONL 和测试镜像保留在被 Git 忽略的 `artifacts/`。`tests/fixtures/` 中的图片和标注同样只留本地。新 checkout 可以运行单元测试；准备好本地夹具后，才能运行显式真实模型测试和截图回放。


## 原生虚拟介质（2026-09-19）

在既有 Supermicro X10 / FW 4.00 测试机上使用独立小镜像，未改动系统盘或 NVMe：

- 900 KiB ISO 以虚拟 USB 光驱接入，Linux 挂载后读取预置文本成功；主机卸载后 BMC 确认拔出。
- 16 MiB 原始镜像以 Virtual Disk 接入，Linux 创建 FAT 文件系统、写入并读取 `RESULT.TXT` 成功。
- Linux 写入 1 MiB 随机 `PAYLOAD.BIN`，卸载文件系统并拔出后，本机只读挂载镜像，文本内容正确且二进制 SHA-256 与主机一致：`8a4149cdce8233edf02cd7c6f49885698315205fb44347ac7ca1d2b6825b5e78`。
- 重新以默认只读模式接入，主机显示 `RO=1`，随机文件哈希仍一致，写入报只读文件系统；测试前后整张镜像 SHA-256 一致。可写阶段记录 25 次 WRITE(10)，1,109,504 字节；没有 SCSI 拒绝。此为小镜像功能验证，不是吞吐基准。
- 原有控制台与介质使用独立连接，同时使用期间正常。
- 最终构建实测共享锁占用时，第二次可写申请返回 `MEDIA_BUSY`；显式卸载后返回 `image_lock: released`，随后同镜像可成功申请 `exclusive` 并正常释放。

自动测试覆盖 PDU 分片与合包、长度及 CBW 校验、短 CDB、越界/方向/长度错误、只读写入拒绝、落盘持久性、共享/独占锁互斥与释放后重新申请。回环 WebSocket 故障测试在 WRITE 数据只到一半时断线，确认不会产出可执行命令；会话在工作线程结束（含 panic）后继续保留独占镜像句柄，直到显式释放。

真实环境未进行写入中断电、BMC 重启、长时间负载、槽位 1/2、安装盘启动测试；不能从上述结果承诺故障下文件系统一致性。截图和测试镜像仅留本地，不加入发行包。

## 0.3.0 发布检查

30 项单元测试和 2 项真实 CLI/本地 socket 集成测试通过。新增覆盖未知参数拒绝、规范化目标地址、工作线程 panic 后继续保留锁、CLI 与 API 的一致请求及错误退出码。格式、工作区 Clippy、release 构建、本地 OCR 模型回归及 Skill 格式检查通过；发布包仅包含源代码、文档、许可及模型依赖引用。

新 release CLI 在同型 BMC 上再次完成只读 ISO 接入、状态查询、拔出和列表查询。记录读取 57,344 字节、写入 0 字节，BMC 确认 `detached` 且 `image_lock: released`。Linux 的可选探测命令（如 0x51、0x85）按不支持拒绝，不影响已验证的 ISO 读取；本轮没有挂载主机文件系统或修改主机配置。
