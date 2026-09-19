//! Exercise the actual CLI against a local service stub, without BMC credentials.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
    thread,
};

#[test]
fn media_cli_uses_the_same_api_and_exit_status_as_call() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("service.sock");
    let image = dir.path().join("transfer.img");
    std::fs::write(&image, [0; 512]).unwrap();
    for (args, method, response, success) in [
        (
            vec![
                "mount",
                "example.invalid",
                image.to_str().unwrap(),
                "--kind",
                "disk",
                "--writable",
            ],
            "media.mount",
            json!({"result":{"media_id":"m-test"}}),
            true,
        ),
        (
            vec!["status", "m-test"],
            "media.status",
            json!({"result":{"state":"attached"}}),
            true,
        ),
        (vec!["list"], "media.list", json!({"result":[]}), true),
        (
            vec!["unmount", "missing"],
            "media.unmount",
            json!({"error":{"code":"SESSION_LOST","message":"unknown media"}}),
            false,
        ),
        (
            vec!["unmount", "m-disconnected"],
            "media.unmount",
            json!({"result":{"state":"disconnected","image_lock":"released","error":"detach unconfirmed"}}),
            false,
        ),
        (
            vec!["unmount", "m-detached"],
            "media.unmount",
            json!({"result":{"state":"detached","image_lock":"released","error":null}}),
            true,
        ),
    ] {
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            writeln!(stream, "{response}").unwrap();
            request
        });
        let output = Command::new(env!("CARGO_BIN_EXE_ikvmt"))
            .arg("media")
            .args(args)
            .env("IKVM_SOCKET", &socket)
            .output()
            .unwrap();
        assert_eq!(output.status.success(), success, "{:?}", output);
        let _: Value = serde_json::from_slice(&output.stdout).unwrap();
        let request = server.join().unwrap();
        assert_eq!(request["method"], method);
        if method == "media.mount" {
            assert_eq!(request["params"]["writable"], true);
            assert_eq!(
                request["params"]["image"],
                std::fs::canonicalize(&image).unwrap().to_str().unwrap()
            );
        }
        std::fs::remove_file(&socket).unwrap();
    }
}

#[test]
fn invalid_media_options_fail_without_contacting_a_service() {
    for args in [
        vec!["--kind", "cdrom", "--writable"],
        vec!["--kind", "disk", "--slot", "3"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_ikvmt"))
            .args([
                "media",
                "mount",
                "example.invalid",
                "/nonexistent.img",
                "--socket",
                "/nonexistent.sock",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let error = String::from_utf8(out.stderr).unwrap();
        assert!(
            error.contains("CD-ROM is read-only") || error.contains("invalid value '3'"),
            "{error}"
        );
    }
}
