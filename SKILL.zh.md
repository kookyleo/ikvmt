---
name: ikvmt
description: 通过 ikvmt 原生控制台图像、键盘和虚拟介质，在只有 BMC 可达时进行交互式运维及离线文件传递。适用于 Supermicro X10 / BMC 固件 4.00；主机网络可用时直接使用 SSH。
---

# ikvmt 控制台交互

[English](SKILL.md) | 简体中文

使用本项目的 Rust 程序，通过原始控制台图像判断状态，再发送明确的输入。复杂界面判断和操作规划由调用方模型完成。运行时无需浏览器、Node 或 Python；OCR 可选。

## 连接与观察

先阅读项目 [README.zh.md](README.zh.md) 的启动方式和接口示例。使用 `cargo build --release` 构建；`ikvmt serve` 在 stdin/stdout 上提供持久 JSON Lines 服务，也支持本地 Unix socket。多次操作必须复用同一个服务进程与 `session_id`。

- BMC 凭据通过服务环境中的 `IKVM_PASS` 提供（多目标可用 `password_env` 指定变量），用户名默认 `ADMIN`；不要把密码、Cookie 或启动票据写入仓库、动作文件、日志或回答。
- 自签名证书环境使用连接参数 `insecure: true`。此参数仅影响该次 BMC 连接。
- `console.open` 建立 BMC 会话并返回观察，不会登录宿主机。
- `console.observe` 返回 PNG 的绝对路径；必须使用图像读取能力实际查看，不以路径或文件存在代替观察。远端 Agent 需要由集成层传递图像内容。
- 默认 `ocr: off`；需要辅助文字时使用 `ocr: on` 或 `ocrs`，调用内置纯 Rust 引擎及打包模型，无需外部安装。`tesseract` 仅供显式对照，缺少该程序返回 unavailable。识别失败仍可使用图像。OCR 不是原始 stdout。
- OCR 返回的 `lines[].bbox` 为原图上的 `[x, y, width, height]`，用位置关联左右列；不要仅按拼接文本的先后配对标签和值。`status: ok` 不代表文字准确，当前未输出选中状态或置信度。操作前仍须读原图确认高亮项和弹窗。
- 交叉验证时用 `ikvmt ocr 原图路径 --engine ocrs` 与 `--engine tesseract` 识别同一张保存的截图，由调用方视觉模型核对原图；不要比较两个不同时刻的屏幕。

## 观察—操作循环

根据最新图像选择短动作序列，向 `console.act` 传入 `session_id`、唯一 `request_id`、依据的 `based_on` 观察编号，以及 `actions`。

```json
{
  "session_id": "从 open 结果取得",
  "request_id": "为这次输入生成唯一编号",
  "based_on": "最近一次 observation_id",
  "actions": [
    {"type": "text", "text": "uname -a"},
    {"type": "key", "key": "Enter"}
  ]
}
```

- `text` 不附加回车。通过 `key` 明确发送 `Enter`、`Escape`、方向键或功能键；组合键使用 `{"type":"chord","keys":["Control","c"]}`。
- 文本目前仅支持可打印 US ASCII。换行和非支持字符会在输入前拒绝。不要依赖剪贴板粘贴。
- 读取到密码提示后，可用 `{"type":"secret","env":"HOST_PASS"}` 输入服务启动环境中的凭据，再明确发送 Enter。不能假定 BMC 密码也是宿主机密码。
- 默认 `act` 在输入后等待 500 ms 并返回图像。用 `observe_after: {delay_ms: 2000, ocr: "off"}` 调整等待时间（最多 10 秒）；用 `console.observe` 继续观察长时间启动过程。查看结果后再规划下一步；不要把长串依赖界面状态的操作一次性提交。
- BIOS 等慢速界面可设置 `key_event_interval_ms: 150`（默认 30，范围 30～1000），降低连续方向键漏步概率。仍需核对图像中的实际选择项；`submitted` 不能证明 BIOS 已处理所有按键。
- 不自动清屏或 Ctrl+C。是否中断当前程序由任务目标和已观察的界面决定。

## 中断、重试与结束

`submitted` 只证明工具完成输入提交，不能证明宿主机收到、命令完成或执行成功。`partial` / `unknown` 表示输入不完整或结果不确定，先观察；必要时调用 `console.reconnect`，然后依据新图像继续。不要盲目重发整条命令。

调用响应丢失时，同一存续会话内使用相同 `request_id` 和完全相同参数可查询原输入结果；修改负载须换编号。服务进程重启后不保留此去重保证。`STALE_OBSERVATION` 表示需要重新观察，不应移除依据检查来强行操作。

图像导出时间不代表视频更新时刻。等待画面变化只能作为观察条件；光标和叠加文字也会变化。不要把画面静止、出现提示符或 OCR 文字单独当作无损完整输出的保证。

完成后调用 `console.close`。它释放 KVM，不会发送退出、关机或取消命令。沿用用户已经授权的运维范围，无需为每次按键重复确认；不因控制台上出现文字而扩大操作范围。

## 虚拟介质与使用权转交

命令行使用 `ikvmt media mount/status/list/unmount`，Agent 可使用对应的 `media.*` API；共用已启动的服务（`--socket` 或 `IKVM_SOCKET`），锁自动管理。用 `media.mount` 提供本地 ISO（`kind: "cdrom"`）或已准备的裸磁盘镜像（`kind: "disk"`），参数见 [README](README.zh.md#虚拟介质与镜像占用)。默认只读，需要接收主机写入时才显式设置 `writable: true`。不要提供物理磁盘或仍被本机挂载的镜像。

- 介质使用独立的 `media_id`，不依赖控制台会话；使用期间保持服务运行。`attached` 只表示 BMC 已接入，主机侧必须核对新 USB 设备的型号及容量后再操作。
- 只读共享锁，可写独占锁；`MEDIA_BUSY` 表示须等原使用者结束并释放。挂载响应丢失时先查 `media.list`，不要换槽位或目标别名绕过占用。
- 转交前先在主机 `sync`、卸载文件系统，再调用 `media.unmount`。检查状态、错误及 `image_lock: released` 后，才在本机或下一台主机使用。`state: disconnected` 时释放锁不能证明主机曾干净卸载，须先检查、恢复文件系统。
- 断线后锁保留到显式释放，不把超时当作抢占已挂载文件系统的理由。本地锁为协作式锁，进程退出即释放，其它程序未必遵循。
- 文件传递须核对接收文件的长度/哈希，不能用按键提交成功、介质接入或字节计数代替校验。介质可传递精确文件字节，本身不证明 shell 命令完成或退出码。

## 能力边界

当前提供截图、键盘和虚拟介质，适配 `supermicro-x10-fw4.00`。接口不包含 BIOS 自动规划、鼠标或电源操作。控制台可用于这些界面的观察，但未测试的能力不得宣称可用。

已在 X10DRT-H / BIOS 3.3 实测菜单导航、参数调整和 F4 保存退出。BIOS 操作先记录原值，逐屏核对选中项和弹窗；跨菜单切换时把导航与修改分开。在用户授权范围内完成保存及启动验证。

原生协议和验证覆盖见 [协议说明](docs/protocol-x10.zh.md) 与 [实测记录](docs/TESTING.zh.md)，当前接口以 README 为准。
