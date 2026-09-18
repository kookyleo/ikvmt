# ikvmt

**Run shell commands and read the host console through a Supermicro BMC's HTML5 iKVM — fully headless, no SSH, no serial cable.**

`ikvmt` drives the BMC's built-in HTML5 KVM viewer in a headless browser. It types a shell command into the host's console using the BMC's own Insyde keymap, then reads the result back by reconnecting for a fresh video frame and OCR'ing the canvas.

```
your machine ──(IPMI/LAN)──► BMC ──(KVM video + keyboard)──► host VGA console
```

Verified on Supermicro **X10DRT-H** boards, **BMC firmware 4.00**, hosts running Proxmox VE / Debian.

## When you need it

- The host OS is running but SSH is broken, unconfigured, or the network path is gone (e.g. during NIC/IP rework).
- You need root shell access out-of-band, but the BIOS serial redirection isn't set up (or you can't reboot into BIOS).
- You're re-IPing a fleet and need to read/verify host network config from the only surviving path — the BMC.

If the host is reachable directly, use SSH. If BIOS SOL works, use `ipmitool sol activate`. `ikvmt` is the fallback that works on a stock Supermicro BMC with nothing else configured.

## Install

```bash
git clone https://github.com/kookyleo/ikvmt.git
cd ikvmt
npm install                    # installs playwright-core
brew install tesseract         # macOS (or your distro's package)
python3 -c "import PIL" || pip3 install pillow
```

Requirements:

- Node.js ≥ 18
- A Chromium-family browser (system Google Chrome by default)
- `tesseract` and `python3` + `Pillow` on PATH (screen OCR)

## Usage

```bash
export IKVM_USER=ADMIN IKVM_PASS='<bmc password>'
export HOST_USER=root HOST_PASS='<host root password>'   # only needed if the console is at a login prompt

node ikvmt.mjs <bmc-ip> shot              # OCR the current console screen
node ikvmt.mjs <bmc-ip> run "<command>"   # run a command in the host shell, print its output
node ikvmt.mjs <bmc-ip> repl              # interactive: one command per stdin line ("shot" = screenshot)
node ikvmt.mjs <bmc-ip> run "..." --json  # machine-readable output
```

Environment variables:

| Variable | Meaning |
|---|---|
| `IKVM_USER`, `IKVM_PASS` | BMC web/IPMI credentials (required) |
| `HOST_USER`, `HOST_PASS` | Host OS login, auto-used when the console is at `login:` (optional) |
| `IKVM_CHROME` | Path to a Chrome/Chromium executable (default: macOS system Chrome) |

Examples:

```bash
$ node ikvmt.mjs 10.0.0.5 run "hostname;ip -4 -o -brief a"
pve6
lo    UNKNOWN 127.0.0.1/8
vmbr0 UP 10.0.1.206/24
default via 10.0.1.1 dev vmbr0
```

## How it works (and why it's shaped this way)

Each of these design decisions exists because a simpler alternative failed on this hardware:

1. **A real headless browser, not a raw VNC/WebSocket client.** The BMC's port 5900 accepts TCP but never speaks RFB; a raw WebSocket client gets through the Insyde auth handshake and then is rejected by the server with an opaque error. The browser path reproduces exactly what the manual "Launch KVM" button does.

2. **The KVM launcher must be opened in a second tab, with a referer.** Navigating the main (frameset) tab away from the BMC frameset kills the web session; a referer-less request to the launcher page redirects back to login. Poll the `#btnikvmhtml5_bootstrap` button until enabled (the page itself polls BMC readiness), click it, wait for the popup's `UI.rfb._rfb_state === 'normal'`.

3. **Typing goes through the BMC's Insyde Keymap with explicit Shift.** Plain `keyboard.type()` in a headless browser races the noVNC layout table and garbles case and shifted characters (`@` becomes `2`, `|` becomes `\`). So every character is sent as keysym down/up pairs looked up in the BMC's own `Keymap=[[keysym,keycode],…]` table, with Shift (keysym 65505) pressed and released around shifted characters.

4. **Enter is sent twice, two ways** (`keyboard.press('Enter')` + a raw `sendKey(65293)` down/up pair) — single presses occasionally don't land.

5. **Two-phase reading defeats the video freeze.** The KVM video stream stops updating ~30–60 s into a session even though keystrokes still work, so "take a screenshot after typing" frequently captures a stale frame. Instead: *phase A* connects, types the command (`clear;…` so the screen ends with just the output), and disconnects; the BMC needs ~30 s to free the exclusive KVM slot; *phase B* reconnects and screenshots within the first 30 s, when the video is fresh. The terminal content persists on the VGA console across KVM disconnects, so phase B sees exactly what phase A produced.

6. **OCR pipeline: autocontrast → threshold → 2× upscale → tesseract.** The terminal-on-black canvas is too dark for raw tesseract. For digits that OCR confuses (3/5/6/8), pipe output through `tr 0123456789 ABCDEFGHIJ` and decode the letters — OCR misreads letters far less often.

## Limitations

- **Exclusive KVM**: one KVM session per BMC. Close other KVM tabs (yours included) first; leftover sessions can block reconnection for ~1 min.
- **~60 s per command** in `run` mode (connect + type + slot wait + fresh read). `shot` is ~30 s.
- **Output is whatever fits on one console screen** (no scrollback retrieval) — keep commands short or pipe through `head`/`tail`.
- **OCR is best-effort**: symbols, angle brackets, and long hex strings can be garbled. Prefer compact command output (`ip -o -brief`, `grep`, `tr`). The BMC also overlays sensor text (°C) onto the video at random positions.
- Reboots, long-running commands, and anything that leaves the console at a login prompt are not automatically handled beyond the initial login attempt — the tool assumes a root shell and re-logins when it detects `login:`.

## Safety

- `run` types into whatever the console shows. If the host is at a login prompt it logs in with `HOST_USER/HOST_PASS` (if set); if already in a shell it runs the command as that user.
- `repl` and `run` will execute whatever you hand them, with no confirmation. Treat `HOST_PASS` access like root shell access.
- The tool only writes keystrokes to the host console and screenshots it. It does not touch BMC configuration.

## License

[Apache License 2.0](LICENSE)
