//! recguest — a QEMU guest agent in Rust, replacing the upstream
//! qemu-guest-agent in LevitateOS and AcornOS guests.
//!
//! Protocol: one JSON object per line over the virtio-serial port, QMP
//! style (`{"execute": ..., "arguments": {...}}` in, `{"return": ...}` /
//! `{"error": {...}}` out), per the QEMU Guest Agent Protocol Reference.
//! A `0xFF` byte from the client flushes any partially read input;
//! `guest-sync-delimited` answers with a leading `0xFF` sentinel.
//!
//! Allowlist (anything else is refused): guest-ping, guest-sync,
//! guest-sync-delimited, guest-info, guest-shutdown. There is no
//! guest-exec, no filesystem freeze, no host-command execution.
//! Outside a VM (no port), exit cleanly.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

const DEFAULT_PORT: &str = "/dev/virtio-ports/org.qemu.guest_agent.0";
const COMMANDS: &[&str] = &[
    "guest-ping",
    "guest-sync",
    "guest-sync-delimited",
    "guest-info",
    "guest-shutdown",
];

enum Port {
    File(std::fs::File),
    Unix(UnixStream),
}

impl Read for Port {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Port::File(f) => f.read(buf),
            Port::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Port {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Port::File(f) => f.write(buf),
            Port::Unix(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Port::File(f) => f.flush(),
            Port::Unix(s) => s.flush(),
        }
    }
}

/// Open the port: a socket path connects, anything else opens read/write.
fn open_port(path: &str) -> std::io::Result<Port> {
    if let Ok(stream) = UnixStream::connect(path) {
        return Ok(Port::Unix(stream));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    Ok(Port::File(file))
}

fn reply_ok(value: serde_json::Value) -> String {
    serde_json::json!({ "return": value }).to_string()
}

fn reply_err(class: &str, desc: &str) -> String {
    serde_json::json!({ "error": { "class": class, "desc": desc } }).to_string()
}

/// One response: JSON text, whether it needs the raw 0xFF sentinel
/// prefix (guest-sync-delimited), and whether to power off after sending.
struct Response {
    body: String,
    delimited: bool,
    shutdown: bool,
}

/// Handle one request line.
fn handle(line: &str, dry_run_shutdown: bool) -> Response {
    let plain = |body: String| Response {
        body,
        delimited: false,
        shutdown: false,
    };
    let req: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return plain(reply_err("InvalidParameter", &format!("invalid JSON: {e}"))),
    };
    let Some(execute) = req.get("execute").and_then(|v| v.as_str()) else {
        return plain(reply_err("InvalidParameter", "missing \"execute\""));
    };
    let args = req.get("arguments").cloned().unwrap_or(serde_json::Value::Null);
    match execute {
        "guest-ping" => plain(reply_ok(serde_json::json!({}))),
        "guest-sync" | "guest-sync-delimited" => {
            let Some(id) = args.get("id").and_then(|v| v.as_i64()) else {
                return plain(reply_err(
                    "InvalidParameter",
                    "guest-sync needs an integer \"id\"",
                ));
            };
            Response {
                body: reply_ok(serde_json::Value::from(id)),
                delimited: execute == "guest-sync-delimited",
                shutdown: false,
            }
        }
        "guest-info" => plain(reply_ok(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "supported_commands": COMMANDS.iter().map(|name| serde_json::json!({
                "name": name, "enabled": true, "success-response": true
            })).collect::<Vec<_>>(),
        }))),
        "guest-shutdown" => {
            let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("powerdown");
            match mode {
                // v1 powers off; a guest "reboot" mode is not offered.
                "powerdown" | "halt" => Response {
                    body: reply_ok(serde_json::json!({})),
                    delimited: false,
                    shutdown: !dry_run_shutdown,
                },
                other => plain(reply_err(
                    "InvalidParameter",
                    &format!("unknown shutdown mode {other:?}"),
                )),
            }
        }
        other => plain(reply_err("CommandNotFound", &format!("unknown command {other:?}"))),
    }
}

fn poweroff() -> ! {
    // Direct reboot-syscall poweroff: identical under systemd, OpenRC and
    // busybox init, no init-system cooperation needed for a VM shutdown.
    unsafe {
        libc::sync();
        libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
    }
    eprintln!("recguest: poweroff syscall failed");
    std::process::exit(1);
}

fn serve(port: &mut Port, dry_run_shutdown: bool) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = port.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        for &byte in &chunk[..n] {
            if byte == 0xFF {
                // Client asks us to flush partially read input.
                buffer.clear();
                continue;
            }
            if byte == b'\n' {
                let line = String::from_utf8_lossy(&buffer).trim().to_string();
                buffer.clear();
                if line.is_empty() {
                    continue;
                }
                let response = handle(&line, dry_run_shutdown);
                if response.delimited {
                    port.write_all(&[0xFF])?;
                }
                port.write_all(response.body.as_bytes())?;
                port.write_all(b"\n")?;
                port.flush()?;
                if response.shutdown {
                    poweroff();
                }
            } else {
                buffer.push(byte);
            }
        }
    }
}

struct Options {
    port: String,
    explicit_port: bool,
    dry_run_shutdown: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options {
        port: DEFAULT_PORT.to_string(),
        explicit_port: false,
        dry_run_shutdown: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                opts.port = args.next().ok_or("--port needs a path")?;
                opts.explicit_port = true;
            }
            "--dry-run-shutdown" => opts.dry_run_shutdown = true,
            "--version" | "-V" => {
                println!("recguest {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("recguest — QEMU guest agent\nusage: recguest [--port <path>] [--dry-run-shutdown] [--version]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other:?} (try --help)")),
        }
    }
    Ok(opts)
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("recguest: {e}");
            std::process::exit(2);
        }
    };
    let mut port = match open_port(&opts.port) {
        Ok(p) => p,
        Err(e) => {
            if opts.explicit_port {
                eprintln!("recguest: cannot open {}: {e}", opts.port);
                std::process::exit(1);
            }
            // No virtio port: not a VM (or no agent channel). Clean exit.
            eprintln!("recguest: no guest agent port ({}): not a VM, exiting", opts.port);
            return;
        }
    };
    if opts.dry_run_shutdown {
        eprintln!("recguest: dry-run shutdown enabled (test only)");
    }
    if let Err(e) = serve(&mut port, opts.dry_run_shutdown) {
        eprintln!("recguest: port loop ended: {e}");
        std::process::exit(1);
    }
}
