# selftest

**简体中文** | [English](#english)

BORUIX 的**系统自检程序**——把机器跑一遍，告诉你各个子系统是否正常。

在 shell 里直接运行：

```
selftest           # 全量
selftest quick     # 秒级：信号 + libc + shell 路径
selftest audio     # 只跑音频组
selftest thread    # 只跑线程组
```

---

## 为什么自检不放在开机流程里

早期版本在开机时自动跑完全部自检。这带来一个问题：**自检要跑两三分钟，而这两三分钟里用户
只能干等**。

更关键的是，自检是**"机器跑给人看"的测试负载**——它对开机没有任何作用，跑完了用户还是要进
shell 干活。把它塞在开机路径上，等于让所有用户为一次性的验证付出等待时间。

所以自检被挪出开机流程，成为**按需拉起的独立程序**：开机直达 shell，需要验证时再跑。

这个改动还有一层好处：自检从此可以在系统**已经运行起来之后**执行，验证的是真实的运行状态，
而不是刚刚启动时的状态。

## 分组

| 命令 | 内容 | 耗时 |
| --- | --- | --- |
| `selftest` | 全部 | 分钟级 |
| `selftest quick` | 信号、C 标准库、shell 执行路径 | 秒级 |
| `selftest audio` | 音频往返 | 较长 |
| `selftest thread` | 线程创建、C 线程、线程性能 | 中等 |

`quick` 组的定位是**随时可跑**：改动之后想知道有没有把基本盘弄坏，几秒钟就能得到答案，
不必等完整的一轮。

## 覆盖什么

**信号。** 注册一个信号处理函数，向自身发送信号，验证处理函数确实被投递执行、并且返回后能
正确恢复原流程。这条链路涉及内核的投递机制与用户态的恢复约定，任何一环断掉都会导致进程在
收到信号后行为异常，所以单独验证。

**线程。** 线程创建与汇合、C 线程接口、线程性能基准。

**音频。** 完整的音频往返链路。

**shell 执行路径。** 以不同参数拉起 shell，覆盖三类装载结果。**每一条参数都是一次真实执行**，
不是模拟——shell 走真实的命令解析与执行路径。

**独立的用户态验收程序。** 拉起若干专项验收进程，验证从独立程序出发调用系统接口的完整链路
（链接、符号解析、跨进程调用都成立）。

## 一个被记录下来的接口陷阱

分组参数从哪读，这里踩过一个真实的坑。

系统给程序传参数的方式是：**参数个数恒为 1，整条命令行放在第一个参数里**。也就是说，
`selftest audio` 执行时，程序拿到的是**一个**参数，内容是 `audio`。

而最初的实现按常规习惯去读**第二个**参数——那个位置根本不存在。结果是分组筛选**从来没生效
过**：要么因为参数个数不足而直接短路成全量，要么越界读到了数组的终结符。

修正后改为读第一个参数。这段经历留在代码注释里，因为它是**这套参数约定最容易踩的地方**：
从别的系统迁移过来的人会本能地按传统习惯去读第二个位置。

## 一项会如实跳过的检查

音频组里有一项检查在 shell 中运行时**必然失败**——不是回归，而是真实约束。

原因：音频通道的消费者槽位是**独占**的。系统启动序列里这项检查跑在音频服务之前，所以能拿到
槽位；而在 shell 中运行时音频服务已经常驻并占用了槽位，检查会如实报告"资源忙"。

程序的处理方式是：**尝试，遇到"资源忙"就打印"跳过"而不是"失败"**。把它标成失败是误导——
环境不具备，不是功能坏了。

## 输出

每项检查打印一行结果，明确标出通过或失败。结束时给出总体结论。

**失败信息是具体的**：说明是哪一项、期望什么、实际观察到什么，而不是笼统的"自检失败"。

## 构建

```bash
cargo build --release
```

编译产物部署为 BORUIX 系统中的用户态程序，在 shell 中执行。

## 文件结构

```
selftest/
├── Cargo.toml    # 包定义
├── build.rs      # 注入链接脚本
├── linker.ld     # 用户态段布局
└── src/
    └── main.rs   # 各组检查
```

## 相关项目

- [`init`](https://github.com/BRX-Boruix/init) —— 启动系统（自检已从其中迁出）
- [`shell`](https://github.com/BRX-Boruix/shell) —— 提供 `selftest` 命令
- [`libsys`](https://github.com/BRX-Boruix/libsys) —— 用户态系统调用封装

## 许可

MIT License，版权归 Yang Borui 所有。详见 [LICENSE](LICENSE)。

---

# English

[简体中文](#selftest) | **English**

BORUIX's **system self-test program** — it exercises the machine and reports whether each subsystem
is healthy.

Run it from the shell:

```
selftest           # everything
selftest quick     # seconds: signals + C library + shell path
selftest audio     # the audio group only
selftest thread    # the thread group only
```

---

## Why the self-test is not part of boot

An earlier version ran the whole self-test automatically at boot. That created a problem: **the
self-test takes two to three minutes, and for those minutes the user simply waits**.

More to the point, the self-test is a **test load "run by a machine for a person to read"** — it does
nothing for booting, and when it finishes the user still has to get to a shell and work. Placing it on
the boot path makes every user pay with waiting time for a one-off verification.

So the self-test was moved out of the boot flow and became a **separate program started on demand**:
boot goes straight to a shell, and verification is run when wanted.

The change carries a second benefit: the self-test can now run **after the system is already up**,
verifying the real running state rather than the state immediately after boot.

## The groups

| Command | Contents | Duration |
| --- | --- | --- |
| `selftest` | everything | minutes |
| `selftest quick` | signals, C standard library, shell execution path | seconds |
| `selftest audio` | the audio round trip | longer |
| `selftest thread` | thread creation, C threads, thread performance | moderate |

The `quick` group is meant to be **runnable at any moment**: after a change, a few seconds tell you
whether the basics are broken, without waiting for a full run.

## What it covers

**Signals.** It installs a signal handler, raises the signal against itself, and verifies that the
handler really is delivered and that the original flow resumes correctly afterwards. That chain
involves the kernel's delivery mechanism and the user-space resumption convention, and a break
anywhere leaves a process misbehaving after a signal, so it is verified on its own.

**Threads.** Thread creation and joining, the C thread interfaces, and a thread performance
benchmark.

**Audio.** The full audio round trip.

**The shell execution path.** It starts the shell with different arguments, covering three classes of
load outcome. **Every argument is a real execution**, not a simulation — the shell goes through its
real parsing and execution path.

**Separate user-space acceptance programs.** It starts several dedicated acceptance processes,
verifying the complete path from an independent program to the system interfaces (that linking,
symbol resolution, and the cross-process call all hold).

## An interface trap worth recording

Where the group argument is read from is a pit this project actually fell into.

The system passes arguments to a program such that **the argument count is always 1, with the whole
command line in the first argument**. That is, when `selftest audio` runs, the program receives
**one** argument whose content is `audio`.

The original implementation read the **second** argument out of ordinary habit — a position that does
not exist. The result was that group filtering **never worked**: it either short-circuited to a full
run for lack of arguments, or read the array terminator out of bounds.

The fix reads the first argument. The episode is kept in the code comments because it is **the easiest
place to trip over this argument convention**: anyone coming from another system will instinctively
reach for the second position.

## One check that honestly skips

One check in the audio group **necessarily fails** when run from the shell — not a regression, but a
real constraint.

The reason: the audio channel's consumer slot is **exclusive**. In the boot sequence this check ran
before the audio service, so it got the slot; when run from the shell the audio service is already
resident and holds it, so the check honestly reports "resource busy".

The program's handling is to **attempt it and, on "resource busy", print "skip" rather than "fail"**.
Marking it a failure would mislead — the environment lacks the capability, the functionality is not
broken.

## Output

Each check prints a line stating its result, clearly marked as passing or failing. A summary
conclusion is given at the end.

**Failure messages are specific**: which check, what was expected, and what was actually observed —
rather than a blanket "self-test failed".

## Building

```bash
cargo build --release
```

The artifact is deployed as a user-space program in a BORUIX system and run from the shell.

## Layout

```
selftest/
├── Cargo.toml    # package definition
├── build.rs      # injects the linker script
├── linker.ld     # user-space section layout
└── src/
    └── main.rs   # the checks for each group
```

## Related projects

- [`init`](https://github.com/BRX-Boruix/init) — starts the system (the self-test has moved out of it)
- [`shell`](https://github.com/BRX-Boruix/shell) — provides the `selftest` command
- [`libsys`](https://github.com/BRX-Boruix/libsys) — the user-space syscall wrapper

## License

MIT License, copyright Yang Borui. See [LICENSE](LICENSE).
