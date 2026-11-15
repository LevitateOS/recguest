//! Host-side protocol check for recguest (no VM needed).
//!
//! Spawns the real binary on a PTY slave (the closest host analogue of
//! the virtio-serial port), drives the five allowlisted commands plus
//! refusal cases, and asserts the replies. Run with:
//!
//!   cargo run -p recguest --example protocol_check
//!
//! Exits nonzero on the first mismatch.

use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Agent {
    child: Child,
    master: std::fs::File,
    buf: Vec<u8>,
}

impl Agent {
    fn spawn(binary: &str) -> Agent {
        let mut master_fd: libc::c_int = -1;
        let mut slave_fd: libc::c_int = -1;
        let mut name = [0 as libc::c_char; 128];
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                name.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(rc, 0, "openpty failed");
        let slave_path = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .unwrap()
            .to_string();
        let child = Command::new(binary)
            .args(["--port", &slave_path, "--dry-run-shutdown"])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn recguest");
        // Raw mode: no echo, no line editing — like the virtio port.
        unsafe {
            let mut tio: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(slave_fd, &mut tio), 0);
            libc::cfmakeraw(&mut tio);
            assert_eq!(libc::tcsetattr(slave_fd, libc::TCSANOW, &tio), 0);
        }
        // The binary opens the slave itself; our copy can go.
        unsafe { libc::close(slave_fd) };
        // Nonblocking master so the reply deadline can bite.
        unsafe {
            let flags = libc::fcntl(master_fd, libc::F_GETFL);
            assert!(flags >= 0 && libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == 0);
        }
        let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
        Agent {
            child,
            master,
            buf: Vec::new(),
        }
    }

    /// Read one reply: raw bytes up to and including the newline.
    fn reply(&mut self) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                return self.buf.drain(..=pos).collect();
            }
            assert!(Instant::now() < deadline, "timed out waiting for a reply");
            let mut chunk = [0u8; 4096];
            match self.master.read(&mut chunk) {
                Ok(0) => panic!("recguest closed the port"),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("read failed: {e}"),
            }
        }
    }

    fn ask(&mut self, request: &str) -> String {
        self.master
            .write_all(format!("{request}\n").as_bytes())
            .unwrap();
        let raw = self.reply();
        String::from_utf8_lossy(&raw).trim_end().to_string()
    }

    fn ask_delimited(&mut self, request: &str, expected_id: i64) {
        self.master
            .write_all(format!("{request}\n").as_bytes())
            .unwrap();
        let raw = self.reply();
        assert_eq!(raw[0], 0xFF, "delimited reply must start with raw byte 0xFF: {raw:?}");
        let text = String::from_utf8_lossy(&raw[1..]).trim_end().to_string();
        assert_eq!(text, format!("{{\"return\":{expected_id}}}"));
        println!("ok: {request} -> 0xFF + {text}");
    }
}

fn expect(name: &str, got: &str, want: &str) {
    assert_eq!(got, want, "{name}: wrong reply");
    println!("ok: {name} -> {got}");
}

fn main() {
    let binary = std::env::args().nth(1).unwrap_or_else(|| {
        std::env::var("CARGO_BIN_EXE_recguest").unwrap_or_else(|_| "recguest".to_string())
    });
    let mut agent = Agent::spawn(&binary);

    let got = agent.ask(r#"{"execute":"guest-ping"}"#);
    expect("guest-ping", &got, r#"{"return":{}}"#);

    let got = agent.ask(r#"{"execute":"guest-sync","arguments":{"id":12345}}"#);
    expect("guest-sync", &got, r#"{"return":12345}"#);

    agent.ask_delimited(r#"{"execute":"guest-sync-delimited","arguments":{"id":777}}"#, 777);

    let got = agent.ask(r#"{"execute":"guest-info"}"#);
    let info: serde_json::Value = serde_json::from_str(&got).expect("guest-info is JSON");
    let names: Vec<&str> = info["return"]["supported_commands"]
        .as_array()
        .expect("supported_commands is a list")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    for want in [
        "guest-ping",
        "guest-sync",
        "guest-sync-delimited",
        "guest-info",
        "guest-shutdown",
    ] {
        assert!(names.contains(&want), "guest-info lacks {want}: {names:?}");
    }
    println!("ok: guest-info lists {names:?}");

    let got = agent.ask(r#"{"execute":"guest-exec","arguments":{"path":"/bin/true"}}"#);
    assert!(got.contains("CommandNotFound"), "unknown command must be refused: {got}");
    println!("ok: unknown command refused -> {got}");

    let got = agent.ask("this is not json");
    assert!(got.contains("\"error\""), "malformed JSON must get an error reply: {got}");
    println!("ok: malformed JSON refused -> {got}");

    // The agent is still serving after refusals.
    let got = agent.ask(r#"{"execute":"guest-ping"}"#);
    expect("guest-ping after refusals", &got, r#"{"return":{}}"#);

    let got = agent.ask(r#"{"execute":"guest-shutdown"}"#);
    expect("guest-shutdown (dry run)", &got, r#"{"return":{}}"#);

    agent.child.kill().ok();
    println!("protocol check PASSED");
}
