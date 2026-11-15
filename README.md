# recguest

A QEMU guest agent in Rust for LevitateOS and AcornOS, intended to replace the upstream `qemu-guest-agent` inside guests.

Status: protocol implemented (the five commands below); not yet shipped in the distro images.

## Check

Host-side protocol check (no VM needed; drives the binary over a PTY):

```sh
cargo build -p recguest
cargo run -p recguest --example protocol_check -- ./target/debug/recguest
```

It asserts ping/sync/sync-delimited (raw `0xFF` sentinel)/info replies, that unknown commands and malformed JSON are refused without killing the loop, and that dry-run shutdown answers before “powering off”.

Design bounds, from the runtime-port research (`docs/rec-runtime-ports.md` in the LevitateOS repo):

- Speak the guest-agent JSON protocol over the virtio-serial port `/dev/virtio-ports/org.qemu.guest_agent.0`.
- Start with an allowlist of commands: `guest-ping`, `guest-sync`, `guest-info`, `guest-shutdown`.
- No `guest-exec` or filesystem freeze in the first version; those are separate decisions.
- Outside a VM (no virtio port), exit cleanly.

Smallest validation once implemented: boot LevitateOS and AcornOS under recqemu/libvirt and drive `guest-ping`, `guest-info`, and `guest-shutdown` from the host.
