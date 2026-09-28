# selftest

BORUIX's on-demand self-test host: organises signal, C library, thread, audio and shell-path tests into groups and runs them when asked.

[简体中文](README.md)

## Usage

Started by a shell builtin; the group argument selects what runs:

- `selftest` — everything, including the cross-core kill storm
- `selftest quick` — signals, C library, shell path, account look-up; done in seconds
- `selftest thread` — the thread group: multithread demo, the C pthread family and its benchmark, a fork round-trip
- `selftest audio` — the audio group: blocking wake round-trip, player positive and negative cases

The effective group name is echoed at start; with no argument it prints `all`. Each test prints its
own pass or fail line, and the run ends with `[selftest] done`. The exit code is always 0; read the
output lines for the verdict.

## Known limitations

- The audio group's round-trip needs the exclusive audio consumer slot; when the hardware driver is resident that slot is taken, and the item prints SKIP rather than FAIL
- Full mode takes the longest and launches stress programs

## Building

```bash
cargo build --release
```

## Repository layout

```
selftest/
├── Cargo.toml    # package manifest
├── build.rs      # injects the linker script
├── linker.ld     # user-space segment layout
└── src/
    └── main.rs   # test groups and spawn coordination
```

## Related projects

- [`threaddemo`](https://github.com/BRX-Boruix/threaddemo) — spawned by the thread group
- [`audioe2e`](https://github.com/BRX-Boruix/audioe2e) — the audio group's round-trip test
- [`pwde2e`](https://github.com/BRX-Boruix/pwde2e) — account look-up acceptance spawned by quick
- [`shell`](https://github.com/BRX-Boruix/shell) — the builtin that starts this program

## License

MIT License, copyright Yang Borui. See [LICENSE](LICENSE).
