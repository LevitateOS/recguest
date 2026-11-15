# recguest

A QEMU guest agent in Rust for LevitateOS and AcornOS, in place of the upstream
`qemu-guest-agent`.

## What it does

It speaks the guest-agent JSON protocol over the virtio-serial port
`/dev/virtio-ports/org.qemu.guest_agent.0`, with an allowlist of five commands:
`guest-ping`, `guest-sync`, `guest-sync-delimited`, `guest-info` and
`guest-shutdown`. Anything else, and malformed JSON, is refused without ending
the loop. There is no `guest-exec` and no filesystem freeze; those would be
separate decisions. Outside a VM (no virtio port) it exits cleanly.

## Where it runs

The builder puts `recguest` in the system image (`distro/builder`,
`LIVE_TOOLS`), and the desktop system starts it only when the machine has the
virtio port: a systemd unit with that condition, or the `vm-guest-agents`
OpenRC service. A base system carries the binary and starts nothing
(`distro/builder/os-recipes/installed-system.rhai`, `desktop_services`).

## Check

Host-side protocol check (no VM needed; drives the binary over a PTY):

```sh
cargo build -p recguest
cargo run -p recguest --example protocol_check -- ./target/debug/recguest
```

It asserts the ping, sync, sync-delimited (raw `0xFF` sentinel) and info
replies, that unknown commands and malformed JSON are refused, and that a dry-run
shutdown answers before "powering off".

Not yet checked: booting LevitateOS and AcornOS under recqemu and driving
`guest-ping`, `guest-info` and `guest-shutdown` from the host. The design notes
are in `docs/rec-runtime-ports.md` in the LevitateOS repository.
