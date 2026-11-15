//! recguest — a QEMU guest agent in Rust, replacing the upstream
//! qemu-guest-agent in LevitateOS and AcornOS guests.
//!
//! Design bounds (from the runtime-port research): speak the guest-agent
//! JSON protocol over the virtio-serial port
//! `/dev/virtio-ports/org.qemu.guest_agent.0`, start with an allowlist of
//! read-only/control commands (guest-ping, guest-sync, guest-info,
//! guest-shutdown), and do not implement host-command execution
//! (guest-exec) or filesystem freeze until they are separately decided.
//! Outside a VM (no port), exit cleanly.
//!
//! Status: scaffolding only. The protocol loop is not implemented yet.

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") | Some("-V") => {
            println!("recguest {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--help") | Some("-h") | None => {
            println!("recguest — QEMU guest agent (not implemented yet)");
            println!("usage: recguest [--version]");
        }
        Some(other) => {
            eprintln!("recguest: unknown argument {other:?} (try --help)");
            std::process::exit(2);
        }
    }
}
