# ikvmt

English | [简体中文](README.zh.md)

A native out-of-band console tool for humans and AI agents. It retrieves server VGA images and sends keyboard input through the BMC's HTTP/WebSocket interface. Written in Rust, it runs without a browser, Node, Python, or a model API. OCR is off by default; enabling it uses the bundled pure-Rust `ocrs + RTen` engine and embedded models. Tesseract is available only as an explicitly selected comparison engine.

The current adapter targets **Supermicro X10DRT-H / BMC firmware 4.00 / IPMI 2.0**. Screenshots, keyboard input, persistent sessions, and reconnection have been verified on real hardware. The firmware version determines the proprietary HTML5 iKVM protocol; `fw4.00` is not an IPMI protocol version. Other platforms have not been verified, and the tool does not automatically detect or switch vendors.

```text
AI agent / human → ikvmt → BMC HTTP + WebSocket → host VGA / keyboard
                    ↑ PNG images, input submission results
```

The interaction loop is `open → observe → act → observe → close`. The agent views the image, interprets the interface, and chooses the next action. Successful input submission does not prove command completion; screenshots and OCR do not provide byte-exact stdout. This version does not expose a `run(command)` API that would imply those guarantees.

## Install

```sh
cargo install ikvmt --locked
ikvmt --help
```

Cargo downloads the two model crates along with the other build dependencies. The installed executable embeds both models; enabling OCR does not trigger a runtime download.

## Build and capture a screenshot

```sh
cargo build --release --locked
./target/release/ikvmt --help

# Supply the BMC password through the service environment,
# keeping it out of arguments, JSON, source files, and logs.
# Hidden input in zsh:
read -rs 'IKVM_PASS?BMC password: '
export IKVM_PASS

./target/release/ikvmt shot 192.0.2.10 --insecure --output-dir artifacts
```

`shot` returns an absolute PNG path and metadata, then closes the connection without sending host keystrokes. Replace the example IP with your BMC address. `--insecure` disables certificate verification for self-signed certificates; verification is enabled by default. The default username is `ADMIN`, configurable through `--username` or `IKVM_USER`. Add `--ocr` for native OCR; only `--ocr --ocr-engine tesseract` invokes a local Tesseract installation. OCR failure does not prevent the original image from being returned.

Requires Rust 1.94 or later; builds have been verified on macOS arm64 with Rust 1.95. The program uses system TLS. Linux builds generally require OpenSSL development dependencies and have not yet been tested. `ipmitool` was used independently during development to check device information and is not a runtime dependency. The release binary is `target/release/ikvmt`.

## Agent integration

[SKILL.md](SKILL.md) provides agent operating instructions, also available in [Chinese](SKILL.zh.md). Both service modes below use the same JSON Lines protocol: one request and one response per line, with diagnostics on stderr. This is a local service protocol; an MCP adapter is not implemented.

Let the agent manage a persistent child process through stdin/stdout:

```sh
./target/release/ikvmt serve --output-dir artifacts
```

To reuse a session across CLI calls, start a local Unix socket service in one terminal:

```sh
mkdir -p artifacts/runtime
chmod 700 artifacts/runtime
./target/release/ikvmt serve \
  --socket "$PWD/artifacts/runtime/control.sock" --output-dir artifacts
```

Then call it from another terminal:

```sh
./target/release/ikvmt call console.open \
  --socket "$PWD/artifacts/runtime/control.sock" \
  --params '{"target":"192.0.2.10","username":"ADMIN","insecure":true}'
```

The BMC password must be in the **serve process environment**. The environment of `call` is not forwarded to an already running service. For multiple BMCs, use `password_env` in `open` to select different environment variables; the default is `IKVM_PASS`. Manage host credentials separately from BMC credentials.

A complete stdin request and an abbreviated response envelope:

```json
{"id":1,"method":"console.open","params":{"target":"192.0.2.10","insecure":true}}
```

```json
{"id":1,"result":{"session":{"session_id":"…"},"observation":{"observation_id":"…","image":{"path":"/absolute/path.png"}}}}
```

Errors use `{"id":1,"error":{"code":"…","message":"…"}}`; `call` exits with status 1 on an error. `id` only correlates requests and responses. Input deduplication uses `params.request_id`.

## Interaction interface

| Method | Parameters and behavior |
| --- | --- |
| `console.open` | Requires `target`; defaults: `username: ADMIN`, `password_env: IKVM_PASS`, `profile: supermicro-x10-fw4.00`, `insecure: false`. Returns a session and its initial observation |
| `console.observe` | `session_id`; optional `ocr: "on"` and `wait: {since: "observation ID", timeout_ms: 5000}`. Waits up to 10 seconds for pixel changes and returns an observation even on timeout |
| `console.act` | `session_id`, `request_id`, `based_on`, `actions`; observes after a 500 ms delay by default. Set `observe_after: {delay_ms: 1000, ocr: "off"}` to adjust, or `false` to disable the observation |
| `console.reconnect` | `session_id`; closes the old connection, increments `connection_epoch`, reconnects, and observes. Does not replay keyboard input |
| `console.close` | `session_id`; releases the connection. Repeat calls are safe; it does not type an exit command or shut down the host |
| `console.status` / `console.list` | The former requires `session_id`; the latter takes no parameters and lists sessions in this service process |

View the PNG returned by `open`, then put the returned identifiers into an action file, `act.json`:

```json
{
  "session_id": "from the open response",
  "request_id": "a unique ID for this input",
  "based_on": "the observation_id used to choose this action",
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

Input actions:

- `text`: types printable US ASCII without appending Enter. Unicode, newlines, and clipboard operations are not supported. The host must use a US keyboard layout; the caller must observe and handle CapsLock state.
- `key`: one press and release. Supports Enter, Escape, Tab, Backspace, arrow keys, Home/End, PageUp/PageDown, Insert/Delete, F1–F12, modifier keys, and others.
- `chord`: for example, `{"type":"chord","keys":["Control","c"]}`. Presses in order and releases in reverse order.
- `secret`: for example, `{"type":"secret","env":"HOST_PASS"}`. Reads from the service environment and types the value without echoing credentials in the response or appending Enter. Use only after observing the correct input prompt.

Each request allows up to 32 actions and a total of 4,096 text characters. All characters and keys are validated before sending. Keyboard events are spaced at least approximately 30 ms apart, so long text can be slow. Video updates continue to be processed during input.

For slower interfaces such as BIOS, set `"key_event_interval_ms": 150` at the top level of `console.act` to adjust the interval between key press/release messages. The default is 30 ms, with an allowed range of 30–1,000 ms; invalid values are rejected before sending. The tested BIOS occasionally missed consecutive arrow keys at 30 ms, so use a slower pace and check the actual selection. This interval is independent of `observe_after.delay_ms`, which controls the wait between completed input and the screenshot.

## Optional native OCR

The `ocr` field in `console.observe` and `console.act.observe_after` accepts `off` (default), `on` / `ocrs` (bundled engine), or `tesseract` (external comparison). The bundled engine requires no Tesseract, Python, model API, or additional downloads, and does not automatically fall back to an external engine.

Run OCR on an existing screenshot to compare engines on the same frame:

```sh
./target/release/ikvmt ocr /absolute/path.png
./target/release/ikvmt ocr /absolute/path.png --engine tesseract
```

The native engine returns `engine: "ocrs"`, `runtime: "rten"`, `models: "bundled"`, `text`, `lines: [{text, bbox: [x, y, width, height]}]`, and `elapsed_ms`. Bounding boxes use original image coordinates and are clipped to its boundaries. Left and right columns may be returned separately; do not pair BIOS labels and values solely by text order. Selection state and recognition confidence are not provided.

`status: ok` only means inference completed. OCR may misread characters, omit lines, or miss selected items. Check labels, values, positions, and highlights against the original image before changing BIOS settings. The model primarily supports Latin characters and does not guarantee Chinese recognition or exact terminal transcription. See the [validation record (Chinese)](docs/TESTING.zh.md).

Models load lazily on first use and are reused within the service process. Approximately 12.2 MB of weights are embedded in the binary. The weights are CC BY-SA 4.0; distributions containing them must include the [source attribution](models/README.md) and [model license](models/LICENSE-CC-BY-SA-4.0.txt). The project source code remains Apache-2.0.

## Result semantics and retries

An observation contains a PNG path, dimensions, pixel SHA-256, `observation_id`, connection epoch, and input revision. `exported_at_ms` is the local export time; `video.last_update_received_at_ms` and `update_seq` describe decoded video updates. After disconnection, an old image is marked `image.source: cached`; when there is no signal, the image is null. `wait_satisfied` does not mean a command has finished: cursor blinking can also satisfy the pixel-change condition.

`input.status: submitted` only means all keyboard messages were submitted. `partial` and `unknown` mean some input may have been sent; observe the current state before choosing recovery actions. `not_submitted` means no input was submitted. The tool does not automatically clear the screen, send Ctrl+C, log into the host, or resend Enter.

Within a surviving session, the same `request_id` and identical payload return the cached result without repeating input. Reusing an ID with a different payload returns `REQUEST_ID_CONFLICT`. An observation predating other input or reconnection is rejected with `STALE_OBSERVATION`. Deduplication records do not survive a service restart, so do not retry blindly. Input results are retained even if the subsequent screenshot fails.

`open` and `reconnect` do not deduplicate requests. If a response is lost, check `console.list` or `console.status` first. The service executes requests serially while a separate thread continuously receives video. It is not a cancellable asynchronous task system.

Call `console.close` explicitly when finished. The stdin service cleans up sessions on EOF; close sessions before terminating a Unix socket service. Killing the process unexpectedly may leave temporary BMC session occupancy and a local socket file. Remove only your own stale socket after confirming that the service has exited. The program does not overwrite an existing socket automatically.

## Scope and limitations

- Validation currently covers one hardware model and firmware. Linux reboot, POST, BIOS navigation, saving settings, and returning to the Proxmox login screen have been tested on four matching machines. The one-time BIOS boot flag was set using ipmitool. BIOS planning, mouse input, power management, virtual media, and SOL are not implemented.
- AST2100 encoding 87 has been verified on hardware; some unknown block formats return explicit errors. Encoding 88 only accepts complete JPEG payloads and has not been verified on hardware.
- Images represent only the visible screen. They cannot recover text that has scrolled away, distinguish stdout from stderr, retrieve exit codes, or prove that long command output is complete. Page or shorten output for the external model to inspect each screen; reliable byte transfer requires a separate explicit protocol.
- OCR is an optional aid. It still misreads symbols and characters and does not guarantee completeness.
- There is no automatic idle cleanup, disk quota, or persistent recovery. Callers must close sessions and clean up screenshots. PNGs are created with mode 0600 on Unix, and the socket is accessible only to the local user. Screenshots may contain sensitive console content.
- The service rejects a second active session for an identical target string. Do not use different IP/hostname aliases to open concurrent sessions to the same BMC.

## Development

```text
src/interface.rs                    JSON Lines service
src/console.rs                      Sessions, observations, input
src/ocr.rs                          Native OCR and explicit Tesseract comparison
models/                             Embedded weights, attribution, and license
src/vendor/supermicro/x10_fw_4_00/   Authentication, private RFB, keyboard, AST decoding
SKILL.md                            Agent operating instructions
SKILL.zh.md                         Chinese agent operating instructions
docs/protocol-x10.zh.md              Protocol notes (Chinese)
docs/TESTING.zh.md                   Validation record (Chinese)
```

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Real BIOS screenshots and annotations stay local and are not published with the repository. With the required local fixtures, follow the [fixture notes (Chinese)](tests/fixtures/README.zh.md) to run the model test. To replay your own screenshot directory offline and check inference status and bounding-box boundaries:

```sh
cargo run --release --locked --example ocr_corpus -- /path/to/screenshots /tmp/ikvmt-ocr.jsonl
```

See the [architecture notes (Chinese)](docs/architecture.zh.md) for the design and future scope. OCR fine-tuning is an optional enhancement; see the [feasibility evaluation](docs/OCR-FINETUNE-EVALUATION.zh.md) and [data source review](docs/OCR-DATA-SOURCES.zh.md), both in Chinese. The old JavaScript implementation and browser-based port proposal have been removed and remain available in Git history.

English documentation uses `.md`; Chinese documentation uses `.zh.md`. Write commit subjects and bodies in English.

The crate packaging and release procedure is documented in the [release guide (Chinese)](docs/RELEASING.zh.md).

[Apache License 2.0](LICENSE)
