# Virtual media API and CLI

English | [简体中文](virtual-media.zh.md)

Available since 0.3.0 on `supermicro-x10-fw4.00`. This is a local, persistent JSON Lines service, not Redfish, JSON-RPC 2.0, MCP, or Kubernetes CRI. `ikvmt media` is a CLI convenience layer over the same `media.*` methods used by `ikvmt call` and stdin clients. It adds no second session or lock mechanism.

## Four operations

Start `ikvmt serve --socket /private/directory/control.sock` with `IKVM_PASS` in its environment. Keep it running during use. See [README](../README.md#agent-integration) for private directory setup.

```sh
export IKVM_SOCKET=/private/directory/control.sock
ikvmt media mount bmc.example /absolute/install.iso --kind cdrom
ikvmt media mount bmc.example /absolute/transfer.img --kind disk --writable
ikvmt media list
ikvmt media status m-from-mount-response
# First sync and unmount the filesystem inside the host OS.
ikvmt media unmount m-from-mount-response
```

The two mount commands are alternatives for the same default slot. To change media, unmount the previous session first. `--socket` overrides `IKVM_SOCKET`. `--insecure` permits a self-signed certificate or explicit HTTP; TLS verification is otherwise enabled. CLI paths are resolved relative to the caller and sent as absolute paths. API paths are interpreted by the service. All commands print the normal JSON response envelope and exit nonzero on request errors. Both `media unmount` and `call media.unmount` also exit nonzero if the final state is not `detached`, even when the local lock was released. API callers must inspect `state`, `error` and `image_lock`; local release does not guarantee remote detach.

| API | Required parameters | Result |
| --- | --- | --- |
| `media.mount` | `target`, `image`, `kind` | Full status with a new `media_id` |
| `media.status` | `media_id` | Full status |
| `media.list` | None | Array of all sessions in this service, including released sessions; order is unspecified |
| `media.unmount` | `media_id` | Final status after the worker stops and the lock is released |

`media.mount` options:

| Parameter | Default / meaning |
| --- | --- |
| `target` | Required BMC hostname/IP or HTTP(S) origin, without credentials, path, query or fragment |
| `image` | Required path to an existing regular file; neither devices nor directories are accepted |
| `kind` | Required: `cdrom` (ISO, 2,048-byte sectors) or `disk` (raw image, 512-byte sectors); no format autodetection |
| `writable` | `false`; `true` requires `kind: disk` and exclusive access |
| `slot` | `0`; accepted range 0–2, hardware validation currently covers only slot 0 |
| `username` | `ADMIN`; CLI also accepts `IKVM_USER` |
| `password_env` | `IKVM_PASS`; name of a variable in the **service** environment |
| `profile` | `supermicro-x10-fw4.00` |
| `insecure` | `false` |

CLI options use hyphens, e.g. `--password-env`. Unknown API parameters are rejected, including on status, list and unmount. Images must have nonzero, sector-aligned size; they are never created or resized. `disk` is raw block storage, not a directory, qcow2 decoder, or filesystem sharing protocol. Console and media sessions are independent.

## Responses and failures

```json
{"id":1,"method":"media.mount","params":{"target":"bmc.example","image":"/absolute/transfer.img","kind":"disk","writable":true}}
```

An example response, with counters dependent on actual host activity:

```json
{"id":1,"result":{"media_id":"m-example","target":"https://bmc.example","image":"/absolute/transfer.img","kind":"disk","writable":true,"size_bytes":16777216,"slot":0,"state":"attached","error":null,"image_lock":"exclusive","host_enumeration":"not_observed","reconnect":"manual","statistics":{"bytes_read":0,"bytes_written":0,"flushes":0,"commands":{},"rejected_commands":0,"last_rejection":null}}}
```

Request errors use `{"id":1,"error":{"code":"MEDIA_BUSY","message":"…"}}`. The request `id` only correlates responses. Mount is not deduplicated: after losing a response, inspect `media.list` before retrying. Status/list have no device side effects; repeated unmount on a known session is safe. Unknown IDs return `SESSION_LOST`, including after a service restart. Released records remain until the process exits.

| Code | Caller action |
| --- | --- |
| `INVALID_ARGUMENT` | Correct parameters, format or file size; do not retry unchanged |
| `UNSUPPORTED_PROFILE` | Select an implemented adapter |
| `AUTH_FAILED` | Check the service environment and BMC account |
| `MEDIA_BUSY` | Existing image owner or occupied BMC slot; finish and release that use before retrying |
| `MEDIA_IO` | File locking failed for a reason other than contention; inspect the filesystem/error |
| `MEDIA_REJECTED`, `MEDIA_TIMEOUT`, `MEDIA_PROTOCOL`, `DISCONNECTED`, `MEDIA_DETACHED` | Inspect session/BMC state; do not automatically replay or assume detach succeeded |
| `SESSION_LOST` | List sessions; do not reuse IDs from a previous service process |
| `ERROR` | Other file, HTTP, TLS or transport errors; inspect the message |

Asynchronous errors appear in the status `error` string. `MEDIA_WORKER_FAILED` means the worker panicked; its image lock is retained until explicit release. `statistics.commands` counts SCSI opcodes such as `0x28` and `0x2a`; counters are diagnostics, not file-integrity proofs.

## Ownership and handoff

| State | Meaning | Image lock |
| --- | --- | --- |
| `attached` | BMC acknowledged attachment; host enumeration is not observed | `shared` for read-only, `exclusive` for writable |
| `disconnected` before unmount | Serving stopped with an error; BMC/host state may be uncertain | Retained, including after a worker panic |
| `detached` after unmount | Local sync and BMC detach acknowledgement succeeded | `released` |
| `disconnected` after unmount | Worker stopped and local handle released; clean remote detach was not confirmed | `released`; recover the filesystem before reuse |

The worker may briefly report `detached` while unmount is still completing; use the final unmount response and `image_lock` to determine release. Never treat a connection state alone as ownership transfer.

Mount acquires the image lock automatically. Multiple read-only sessions can coexist; any writable session excludes all others using that same file, including cooperating ikvmt processes and file aliases. These are advisory OS file locks, not distributed leases; other programs can ignore them, and process termination releases them. A renamed/replaced file is a different resource: do not mutate, replace, resize or mount an image externally while serving it.

To transfer ownership: host `sync` → host filesystem unmount → `media.unmount` → inspect final status/error → next owner mounts. Read-only/writable mode is fixed for a session; changing it follows the same handoff. No force takeover, expiry, queue, lock upgrade, automatic reconnect, or write replay is provided. The service neither flushes remote filesystem caches nor changes boot order. Do not use DNS/IP aliases to bypass target occupancy checks.

Every acknowledged write is synchronized to the local image. A write whose data PDU is incomplete is not executed; a complete write may have reached the image even when its acknowledgement was lost. This is not a filesystem transaction or crash-consistency guarantee. A 60-second absence of BMC traffic disconnects the worker but retains its lock. Verify the host device's model/size and transferred file hashes, and check/repair filesystems after abnormal termination.

## Design basis and validation

The four operations follow the familiar virtual-media insert/eject/status model in [DMTF Redfish](https://redfish.dmtf.org/schemas/v1/DSP2046_2025.2.pdf). Preventing concurrent writers follows the disk-image ownership principle described by [libvirt](https://www.libvirt.org/kbase/locking.html). ikvmt uses local file locks and this firmware's `/vm` transport; it does not implement either external API or a distributed lock manager. Keeping lock acquisition inside mount avoids a second resource lifecycle.

See [protocol notes](protocol-x10.md) for the wire format and [validation record](TESTING.md) for real ISO/FAT read/write, hash checks and fault-test limits. Raw screenshots, media images, credentials and fleet configuration are excluded from Git and crates.io.
