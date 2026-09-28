# selftest

BORUIX's **system self-test host**: it starts the self-test items on demand and summarises the results.

[简体中文](README.md)

## Using it

```
selftest            run everything
selftest quick      quick group (signals + C library + shell paths, seconds)
selftest audio      audio group
selftest thread     thread group
```

**Why self-testing is not part of the boot flow**: the self-test is a test workload "run for a human to watch", with no real-time observation value, and the full run includes several stretches of real-time audio streaming that would delay the `shell` by minutes. It now lives entirely in this program, started on demand by a `shell` command — **boot goes straight to the `shell`**.

## Coverage

| Group | Contents |
| --- | --- |
| Signals | Signal delivery and disposition; process termination under a `SIGKILL` storm |
| C library | Allocation, strings, formatted output, numeric conversion, time, and related paths |
| Shell paths | Starting `shell` with several different arguments, covering three classes of load outcome |
| Threads | Thread creation and scheduling, per-thread `errno`, thread-local storage, shared address space |
| Audio | Audio-domain blocking round trips; WAV playback positive and negative cases |

Several items run as **real child processes**, taking the genuine load and execute paths rather than being simulated in-process.

## Arguments

The group argument is passed as a **startup argument** (`selftest <group>` in the shell). An empty or absent argument means **everything**.

The program echoes the group name **actually in effect** at startup, so you can confirm the argument was interpreted correctly.

## Exit codes

| Exit code | Meaning |
| --- | --- |
| `0` | All selected groups passed |
| Non-zero | Some items failed |

## Building

```bash
cargo build --release
```

The artifact is deployed as `/programs/selftest.elf`.

## Layout

```
selftest/
├── Cargo.toml    # package definition
├── build.rs      # injects the linker script
├── linker.ld     # user-space section layout
└── src/
    └── main.rs   # group dispatch and the individual tests
```

## Related projects

- [`shell`](https://github.com/BRX-Boruix/shell) — provides the `selftest` command
- [`pwde2e`](https://github.com/BRX-Boruix/pwde2e), [`acee2e`](https://github.com/BRX-Boruix/acee2e), [`trave2e`](https://github.com/BRX-Boruix/trave2e) — standalone acceptance programs this host starts
- [`libsys`](https://github.com/BRX-Boruix/libsys) — the user-space syscall wrapper

## License

MIT License, copyright Yang Borui. See [LICENSE](LICENSE).
