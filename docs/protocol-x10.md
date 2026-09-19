# Supermicro X10 / BMC FW 4.00 native adapter

English | [简体中文](protocol-x10.zh.md)

The implementation follows the reference firmware HTML5 client's wire formats. Firmware scripts are development references; the runtime neither downloads nor executes JavaScript.

## Web session and KVM authentication

1. HTTPS login at `/cgi/login.cgi`, fields `name` and `pwd`. This firmware returns both an empty SID and a live SID. Select the last nonempty SID explicitly to avoid duplicate cookies; both failure and normal teardown attempt logout.
2. Visit `mainmenu`, `man_ikvm_html5` and `man_ikvm_html5_bootstrap` with corresponding Referer headers. Readiness/port queries carry the page's `CSRF_TOKEN` header.
3. Read the temporary `entry_value` ticket. It can exceed 24 bytes. KVM authentication uses its first 15 bytes and next 9 bytes, each padded to 24 bytes.
4. Upgrade the root path on the same HTTP(S) port to WebSocket. For a non-websockify path, this firmware requests no subprotocol and uses binary frames.
5. Negotiate RFB 3.8 / 55.8 and Insyde security type 15 or 16; read the 24-byte challenge and send the padded fields. These are not the BMC account/password.
6. ServerInit has a 12-byte extension after the standard header and desktop name, including session identity and video/keyboard capability flags.

WebSocket message boundaries do not align with RFB fields. Parsing uses a bounded continuous buffer, not the number of receive calls.

## Images and keys

Insyde framebuffer messages use a 4-byte header, 20-byte rectangle description and length-delimited payload. The rectangle contains dimensions, encoding, mode and payload size. Encoding 87 is an AST2100 block stream; the decoder handles JPEG entropy blocks, VQ palette blocks, position jumps and incremental updates into RGBA. Encoding 88 is accepted only when its payload is a complete JPEG.

The first four AST bytes describe quantization tables and color mode. The remaining bitstream uses little-endian 32-bit words, consumed most-significant bit first within each word. Unchanged regions persist across frames. Dimensions, lengths, coordinates, Huffman runs and termination are checked; failed decoding cannot publish a partially updated frame.

Keyboard packets are 18-byte Insyde messages with type 4, press state and HID usage at vendor-defined offsets. They differ from standard RFB's 8-byte keysym messages. Text follows a US layout and explicitly sends Shift; Enter is one press/release pair.

## Known scope

- References include `rfb.js`, `websock.js`, `nav_ui.js`, `ast2100.js` and actual device responses. Standard JPEG Huffman/zigzag data and firmware quantization tables are protocol constants in `tables.rs`; decoding control flow is independently implemented in Rust.
- Floating-point IDCT can differ slightly from the firmware's fixed-point rounding.
- VQ currently supports 4:4:4. Unsupported modes/messages/encodings fail explicitly.
- Receive and image-export times are separate; a new local PNG does not necessarily show a new host frame.
- Reading an image does not provide complete, byte-exact command stdout.

## Native virtual media at `/vm`

Based on `vstorage.js`, `vmlib.js`, `isohandler.js`, `imghandler.js`, `ufiCommand.js` and live responses; these scripts are not executed at runtime.

- HTTP login and launch-ticket acquisition reuse authentication code; media uses an independent `/vm` WebSocket without RFB negotiation.
- An 8-byte PDU header contains four class/device/flag bytes followed by a little-endian payload length. Opcodes: 8 query slots, 1 attach, 2 attach result, 7 endpoints, 4/3 keepalive request/reply, 5/6 detach request/reply.
- Mount authentication uses the first 15 ticket bytes and up to the following 20 bytes. USB descriptors follow the 44-byte body. Media type 1 is a disk, 3 a CD-ROM. Transport is SCSI transparent / USB Bulk-Only Transport.
- The host sends a 31-byte `USBC` CBW. Reads return data and a `USBS` CSW containing the original tag, residue and status. Writes receive the complete data PDU, write and sync locally, then send CSW. Transfers are capped at 1 MiB; buffers and WebSocket sizes are bounded too.
- The Rust SCSI target supports common READ/WRITE 6/10/12/16, capacity, INQUIRY, REQUEST SENSE, MODE SENSE, cache sync and basic optical commands. Unknown commands return ILLEGAL REQUEST; read-only writes return DATA PROTECT; out-of-range requests never extend the file.
- The browser's WRITE(10) branch does not save data back to the file. Rust performs actual writes and `sync_all` before acknowledgement, advertising no volatile write cache. Lost acknowledgements can still leave the host uncertain whether a write occurred.

Hardware validation covers slot 0, Linux ISO reads and FAT image read/write. It does not establish support for every SCSI command, bootloader or firmware. Public API details are in the [media reference](virtual-media.md).
