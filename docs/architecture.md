# Design

English | [简体中文](architecture.zh.md)

ikvmt is a native Rust out-of-band interaction tool for humans and AI agents. The current adapter supports Supermicro X10DRT-H / BMC firmware 4.00. The caller interprets images and plans actions; the tool handles transport, images, keyboard input, virtual media, and inspectable results.

## Modules and flow

```text
CLI / JSON Lines
       ↓
Console sessions / media sessions
       ↓
vendor/supermicro/x10_fw_4_00
       ├── HTTP login/logout
       ├── WebSocket / Insyde RFB
       ├── AST2100 framebuffer decoding
       ├── HID keyboard messages
       └── /vm virtual USB block device

Framebuffer → PNG + metadata → optional ocrs / RTen → caller's vision model
```

- `src/main.rs`: service, media subcommands, one-shot capture, offline decode/OCR, socket calls.
- `src/interface.rs`: JSON Lines requests, common error envelopes and session registry.
- `src/console.rs`: console sessions, video workers, observation history and input deduplication.
- `src/media.rs`: independent media workers and image ownership. `src/media/scsi.rs` implements a file-backed SCSI target; the vendor's `media.rs` handles `/vm` and USB BOT.
- `src/vendor/`: explicit vendor/firmware adapters. Unknown firmware is not implicitly compatible.
- `src/ocr.rs`: bundled ONNX models; Tesseract is an explicitly selected comparison, never an automatic fallback.

Requests execute serially. Session workers continue receiving video or serving media independently. Stdin suits a persistent child process; Unix sockets let short CLI calls reuse sessions. MCP, cancellable tasks and a job queue are not implemented.

## Interaction semantics

The loop is `open → observe → act → observe → close`. Images must be accessible to the caller through a shared filesystem or an integration layer. OCR is off by default; complex BIOS interpretation remains with the caller's vision model.

Actions reference `based_on`. An input submission or connection-epoch change invalidates old observations. `request_id` deduplicates input within a live session; input results are cached before screenshot export so export failure cannot cause input replay. Deduplication does not survive process restart. Reconnect preserves the logical session, increments its epoch and never replays input.

`submitted` means the send path completed, not that the host executed the command. `partial` and `unknown` preserve uncertainty. Export time, video receive time and pixel change are separate concepts; screen changes do not prove command completion.

Credentials come from the service environment. Inputs are validated before transmission. The tool does not automatically log into the host, clear the screen, send Ctrl+C, or log out of the host. Console teardown attempts key release and BMC logout.

## Virtual media

Mount locks an existing regular file before authentication and BMC-slot checks. Read-only is the default and uses a shared lock; writable access is explicit and exclusive. Each complete write is synchronized locally before success. There is no automatic reconnect or write replay. The session retains the image handle if its worker disconnects or panics, until explicit unmount or process exit.

Host filesystem unmount, BMC detach and local lock release are separate operations. Status reports attachment, connection errors, locks and counters separately; these do not establish host enumeration or clean host unmount. See the [media reference](virtual-media.md) for the API, CLI and handoff rules.

## Future scope

1. Test actual network loss between key press/release, long-lived connections, no-signal recovery and other build platforms.
2. Exact shell results would need a separate protocol for completion markers, chunks, length/checksum and recovery. Images/OCR cannot guarantee stdout, stderr or exit status.
3. OCR fine-tuning and interaction-state models are optional enhancements; see the [evaluation](OCR-FINETUNE-EVALUATION.md).
4. Add other vendors, MCP, mouse or power control only for concrete needs; these are not current features.

See [README](../README.md) for executable interfaces and [TESTING](TESTING.md) for evidence and limits.
