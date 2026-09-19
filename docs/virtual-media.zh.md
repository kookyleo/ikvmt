# 虚拟介质 API 与 CLI

[English](virtual-media.md) | 简体中文

从 0.3.0 起提供，适配 `supermicro-x10-fw4.00`。这是本地持久 JSON Lines 服务，不是 Redfish、JSON-RPC 2.0、MCP 或 Kubernetes CRI。`ikvmt media` 为人提供简洁命令，与 `ikvmt call`、stdin 客户端共用 `media.*` API，不另建会话或锁机制。

## 四个动作

在环境中设置 `IKVM_PASS` 后启动 `ikvmt serve --socket /private/directory/control.sock`；使用期间保持服务运行。私有目录的建立方式见 [README](../README.zh.md)。

```sh
export IKVM_SOCKET=/private/directory/control.sock
ikvmt media mount bmc.example /absolute/install.iso --kind cdrom
ikvmt media mount bmc.example /absolute/transfer.img --kind disk --writable
ikvmt media list
ikvmt media status m-from-mount-response
# 先在主机内部 sync 并卸载文件系统。
ikvmt media unmount m-from-mount-response
```

两个 mount 示例是默认槽位的两种选项；更换介质前须先卸载原会话。`--socket` 优先于 `IKVM_SOCKET`；自签名证书或显式 HTTP 使用 `--insecure`，默认校验 TLS。CLI 将相对路径按调用方目录解析成绝对路径；API 路径由服务解释。所有命令输出相同的 JSON 响应外层，请求错误返回非零退出码。`media unmount` 和 `call media.unmount` 在最终状态不是 `detached` 时也返回非零，即使本机锁已释放。API 调用方须检查 `state`、`error` 和 `image_lock`，本机释放不等于远端拔出。

| API | 必填参数 | 结果 |
| --- | --- | --- |
| `media.mount` | `target`、`image`、`kind` | 完整状态与新 `media_id` |
| `media.status` | `media_id` | 完整状态 |
| `media.list` | 无 | 本服务的所有介质会话数组，含已释放记录；顺序不保证 |
| `media.unmount` | `media_id` | 工作线程结束并释放锁后的最终状态 |

`media.mount` 参数：

| 参数 | 默认值与含义 |
| --- | --- |
| `target` | 必填，BMC 主机名/IP 或 HTTP(S) origin，不接受账号密码、路径、查询或片段 |
| `image` | 必填，已有普通文件路径；不接受物理设备或目录 |
| `kind` | 必填，`cdrom` 为 ISO、2048 字节扇区；`disk` 为 raw 镜像、512 字节扇区；不自动识别格式 |
| `writable` | `false`；`true` 要求 `kind: disk` 且独占访问 |
| `slot` | `0`；范围 0–2，目前仅槽位 0 经过硬件实测 |
| `username` | `ADMIN`；CLI 还接受 `IKVM_USER` |
| `password_env` | `IKVM_PASS`，指向**服务环境**中的密码变量名 |
| `profile` | `supermicro-x10-fw4.00` |
| `insecure` | `false` |

CLI 参数使用连字符，如 `--password-env`。所有介质 API 均拒绝未知参数，包括 status/list/unmount。镜像大小须非零且按扇区对齐，服务不创建或调整镜像大小。`disk` 提供 raw 块设备，不是目录共享或 qcow2 解码器。控制台与介质会话彼此独立。

## 响应与错误

```json
{"id":1,"method":"media.mount","params":{"target":"bmc.example","image":"/absolute/transfer.img","kind":"disk","writable":true}}
```

响应示例，计数以主机实际活动为准：

```json
{"id":1,"result":{"media_id":"m-example","target":"https://bmc.example","image":"/absolute/transfer.img","kind":"disk","writable":true,"size_bytes":16777216,"slot":0,"state":"attached","error":null,"image_lock":"exclusive","host_enumeration":"not_observed","reconnect":"manual","statistics":{"bytes_read":0,"bytes_written":0,"flushes":0,"commands":{},"rejected_commands":0,"last_rejection":null}}}
```

请求错误为 `{"id":1,"error":{"code":"MEDIA_BUSY","message":"…"}}`。`id` 仅关联请求与响应。挂载没有请求去重；丢失响应后先查 `media.list`。status/list 不改变设备；已知会话可重复 unmount。未知 ID 返回 `SESSION_LOST`，服务重启后的旧 ID 同样无效。已释放记录保留到进程退出。

| 错误码 | 处理方式 |
| --- | --- |
| `INVALID_ARGUMENT` | 修正参数、格式或文件大小，不原样重试 |
| `UNSUPPORTED_PROFILE` | 选择已实现的适配器 |
| `AUTH_FAILED` | 核对服务环境与 BMC 账号 |
| `MEDIA_BUSY` | 镜像或 BMC 槽位已占用，等待原使用者完成并释放 |
| `MEDIA_IO` | 非竞争原因的文件加锁失败，检查文件系统及错误 |
| `MEDIA_REJECTED`、`MEDIA_TIMEOUT`、`MEDIA_PROTOCOL`、`DISCONNECTED`、`MEDIA_DETACHED` | 核对会话/BMC 状态，不自动重放，也不假定已拔出 |
| `SESSION_LOST` | 查询会话列表，不沿用上一个服务进程的 ID |
| `ERROR` | 其它文件、HTTP、TLS 或传输错误，查看 message |

异步错误保存在状态的 `error` 字符串。`MEDIA_WORKER_FAILED` 表示工作线程 panic，镜像锁保留至显式释放。`statistics.commands` 按 `0x28`、`0x2a` 等 SCSI 操作码计数；计数只能诊断，不能证明文件完整。

## 使用权与转交

| 状态 | 含义 | 镜像锁 |
| --- | --- | --- |
| `attached` | BMC 确认接入，尚未观察主机枚举 | 只读 `shared`，可写 `exclusive` |
| 卸载前 `disconnected` | 服务因错误停止，BMC/主机状态可能不确定 | 保留；工作线程 panic 也保留 |
| 卸载后 `detached` | 本机同步与 BMC 拔出确认成功 | `released` |
| 卸载后 `disconnected` | 工作线程结束、本机句柄已释放，未确认远端干净拔出 | `released`；重用前检查恢复文件系统 |

卸载处理尚未结束时，工作线程可能先报告 `detached`；是否已释放应以最终卸载响应及 `image_lock` 为准，不能仅看连接状态。

mount 自动获取锁。多个只读会话可以并存；可写会话排斥同一文件的所有其它使用者，包括配合加锁的其它 ikvmt 进程和文件别名。这是操作系统协作式文件锁，不是分布式租约；其它程序可忽略，进程终止即释放。替换路径后的文件是另一资源：服务期间不要由外部修改、替换、扩容或挂载镜像。

转交流程：主机 `sync` → 主机卸载文件系统 → `media.unmount` → 核对最终状态/错误 → 下一使用者挂载。会话内读写模式不变，更改模式同样走此流程。不提供强制抢占、过期、排队、锁升级、自动重连或重放写入。服务不能刷新远端文件系统缓存，也不修改启动顺序。不要用 IP/DNS 别名绕过目标占用检查。

每次成功确认的写入都已同步到本机镜像；数据 PDU 不完整的写入不会执行，但确认丢失前完整写入可能已经落盘。这不构成文件系统事务或崩溃一致性保证。BMC 连续 60 秒无流量会断开工作线程，仍保留锁。核对主机设备型号/容量与传输文件的哈希，异常退出后先检查修复文件系统。

## 设计依据与验证

四个动作遵循 [DMTF Redfish](https://redfish.dmtf.org/schemas/v1/DSP2046_2025.2.pdf) 常见的介质接入/弹出/状态模型；限制并发写入遵循 [libvirt](https://www.libvirt.org/kbase/locking.html) 说明的磁盘镜像互斥原则。ikvmt 使用本地文件锁及该固件的 `/vm` 协议，不声称实现这些外部 API 或分布式锁服务。锁随挂载管理，避免增加第二套资源生命周期。

线协议见[协议说明](protocol-x10.zh.md)，ISO/FAT 读写、哈希及故障测试范围见[验证记录](TESTING.zh.md)。真实截图、镜像、凭据及具体机器配置不进入 Git 或 crates.io。
