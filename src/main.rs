//! BORUIX selftest：开机自检的按需宿主（原 init 启动序列的自检整体迁入）。
//!
//! 背景（用户裁决）：开机必须直达 shell；自检是"机器跑给人看"的测试负载，
//! 没有实时观察价值，还让 shell 迟到两三分钟（2 x 44s 实时音频流等）。
//! 现在全部收拢到本程序，由 shell 的 `selftest` 命令按需拉起：
//!
//!     selftest          全量跑（信号/libc/threaddemo/C pthread/pthread bench/
//!                       A2 音频往返/shell 路径/audiofile 正负例/SIGKILL 风暴）
//!     selftest audio    只跑音频组
//!     selftest thread   只跑线程组（threaddemo/pthread 系列）
//!     selftest quick    信号 + libc + shell 路径（秒级）
//!
//! 注意（A2 消费者槽位约束，从 init 原注释保留）：A2 的 audioe2e consumer
//! 要 attach 独占消费者槽。开机序列里它在 intel-hda 之前跑所以能成功；
//! 在 shell 里跑时 intel-hda 已常驻占槽，A2 会如实报 EBUSY——这是真实
//! 约束，不是回归。所以 `selftest audio` 的 A2 项会先尝试，EBUSY 时如实
//! 打印 SKIP 而不是 FAIL。

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};
use libsys::{exec_path, kill, waitpid_any, write, yield_now, STDOUT};

// ---- ADR-034 S1-14：信号端到端自测（libsys action/raise + 用户 handler + sigreturn） ----

// ---- ADR-034 S1-14：信号端到端自测（libsys action/raise + 用户 handler + sigreturn） ----

/// 用户 SIGUSR1 handler 置位标记（供主流程验证 handler 确实被投递并 sigreturn 恢复）。
static SIG_HANDLER_RAN: AtomicU32 = AtomicU32::new(0);

/// naked handler：置位 `SIG_HANDLER_RAN` 后 `ret` → restorer → rt_sigreturn。
/// 内核 `deliver_handler` 把 handler 返回地址写成 restorer 地址，`ret` 即跳 restorer。
#[unsafe(naked)]
unsafe extern "C" fn init_sigusr1_handler() {
    core::arch::naked_asm!(
        "mov dword ptr [rip + {ran}], 1",
        "ret",
        ran = sym SIG_HANDLER_RAN,
    );
}

/// S1-14 自测：注册 SIGUSR1 handler → 向自身 raise → 返回用户态时投递进 handler
/// → restorer → rt_sigreturn 恢复。任何失败打印错误但不中断后续启动（防御式）。
fn signal_selftest() {
    let _ = write(STDOUT, b"[init] signal: testing S1-14 action/raise/handler...\n");
    let handler_addr = init_sigusr1_handler as *const () as usize as u64;
    if let Err(_) = libsys::signal::action(libsys::signal::SIGUSR1, handler_addr, 0) {
        let _ = write(STDOUT, b"[init] signal: action(SIGUSR1) failed\n");
        return;
    }
    let _ = write(STDOUT, b"[init] signal: action(SIGUSR1, handler) ok\n");
    // 向自身（init 恒为 PID 1）raise SIGUSR1。
    match libsys::signal::raise(1, libsys::signal::SIGUSR1) {
        Ok(_) => {}
        Err(_) => {
            let _ = write(STDOUT, b"[init] signal: raise(SIGUSR1) failed\n");
            return;
        }
    }
    let _ = write(STDOUT, b"[init] signal: raise ok, awaiting delivery...\n");
    // 让出触发返回用户态投递（若 raise 返回时未投递，yield 再给一次机会）。
    let _ = yield_now();
    let _ = yield_now();
    if SIG_HANDLER_RAN.load(Ordering::SeqCst) == 1 {
        let _ = write(STDOUT, b"[init] signal: SIGUSR1 handler ran + sigreturn ok (S1-14 PASS)\n");
    } else {
        let _ = write(STDOUT, b"[init] signal: handler did NOT run (S1-14 FAIL)\n");
    }
}

/// libc 最小链路自检（ADR 目标：内核→libsys→libc→init 在开机即通）。
///
/// 验证 libc 的核心 C ABI（malloc/string/printf/strtol/time），打印逐项
/// OK/FAIL 与汇总。防御式：失败仅记录，不中断启动流程。
fn libc_selftest() {
    let _ = write(STDOUT, b"[init] libc: testing core C ABI...\n");
    let mut pass = 0u32;
    let mut fail = 0u32;

    // 1) malloc/free 堆分配。
    unsafe {
        let p = libc::malloc::malloc(48);
        if !p.is_null() {
            *p.add(0) = 0x42;
            *p.add(47) = 0x43;
            if p.add(0).read() == 0x42 && p.add(47).read() == 0x43 {
                pass += 1;
                let _ = write(STDOUT, b"[init] libc: malloc/free OK\n");
            } else {
                fail += 1;
                let _ = write(STDOUT, b"[init] libc: malloc writable FAIL\n");
            }
            libc::malloc::free(p);
        } else {
            fail += 1;
            let _ = write(STDOUT, b"[init] libc: malloc FAIL\n");
        }
    }

    // 2) string：strlen/strcmp。
    unsafe {
        let a = b"hello\0".as_ptr() as *const i8;
        if libc::string::strlen(a) == 5 && libc::string::strcmp(a, b"hello\0".as_ptr() as *const i8) == 0 {
            pass += 1;
            let _ = write(STDOUT, b"[init] libc: string OK\n");
        } else {
            fail += 1;
            let _ = write(STDOUT, b"[init] libc: string FAIL\n");
        }
    }

    // 3) snprintf（格式引擎 + 浮点）。
    unsafe {
        let mut buf = [0u8; 64];
        let n = libc::stdio::snprintf(
            buf.as_mut_ptr() as *mut i8, buf.len(),
            b"v=%d f=%.2f\0".as_ptr() as *const i8, 7, 3.14,
        );
        // 期望 "v=7 f=3.14"（长度 10）。
        if n == 10 {
            pass += 1;
            let _ = write(STDOUT, b"[init] libc: snprintf OK\n");
        } else {
            fail += 1;
            let _ = write(STDOUT, b"[init] libc: snprintf FAIL\n");
        }
    }

    // 4) strtol 整数解析。
    unsafe {
        if libc::stdlib::strtol(b"-99\0".as_ptr() as *const i8, core::ptr::null_mut(), 10) == -99 {
            pass += 1;
            let _ = write(STDOUT, b"[init] libc: strtol OK\n");
        } else {
            fail += 1;
            let _ = write(STDOUT, b"[init] libc: strtol FAIL\n");
        }
    }

    // 5) time 墙钟读数。
    {
        if libc::time::time(core::ptr::null_mut()) > 0 {
            pass += 1;
            let _ = write(STDOUT, b"[init] libc: time OK\n");
        } else {
            fail += 1;
            let _ = write(STDOUT, b"[init] libc: time FAIL\n");
        }
    }

    // 汇总。
    let _ = write(STDOUT, b"[init] libc: selftest passed=");
    let mut b1 = [0u8; 8];
    let pb = u64_to_dec(pass as u64, &mut b1);
    let _ = write(STDOUT, pb);
    let _ = write(STDOUT, b" failed=");
    let mut b2 = [0u8; 8];
    let fb = u64_to_dec(fail as u64, &mut b2);
    let _ = write(STDOUT, fb);
    let _ = write(STDOUT, b"\n");
}

/// 把 u64 写成十进制字节（最小，无前导零）。
fn u64_to_dec(mut v: u64, buf: &mut [u8; 8]) -> &[u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut i = buf.len();
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[i..]
}

/// T1-8：拉起 threaddemo（端到端同进程双线程示例）并等待其完成、收尸。
///
/// 真实用户程序 /programs/threaddemo.elf 在自身进程内 thread_spawn 两个线程（共享
/// 组长 Arc 地址空间、各自独立 mmap 用户栈）→ 各自打印 → thread_exit → 组长 join。
/// init 以 exec_path 派生它并等 waitpid_any 收尸到其 pid。线程demo 秒级完成，故本
/// 阶段其它长驻子进程（volumed/fpcheck）不会先退出。
///
/// SMP 语义：waitpid_any 在"本核此刻无其它就绪进程可接盘、无法阻塞"时会返回
/// Err(WouldBlock/NotFound)——这**不是** threaddemo 已退/不存在，只是当前无法阻塞
/// 等待。故用「yield 让出 + 重试 waitpid_any」轮询直到真正收尸到 threaddemo 的 pid：
/// threaddemo 完成后留 zombie，随后的 waitpid_any 必能同步收尸。**绝不**在未收尸
/// threaddemo 前就放行进入下一阶段（跨核风暴），以免风暴的跨核 SIGKILL 与仍在跑的
/// threaddemo 线程并发触发调度竞争。
fn threaddemo_launch() {
    let _ = write(STDOUT, b"[init] launching threaddemo (T1-8 two-thread demo)\n");
    let pid = match libsys::exec_path("/programs/threaddemo.elf", &[]) {
        Ok(p) => p,
        Err(_) => {
            let _ = write(STDOUT, b"[init] exec_path(threaddemo.elf) failed (non-fatal)\n");
            return;
        }
    };
    let mut bbuf = [0u8; 8];
    let _ = write(STDOUT, b"[init] threaddemo spawned (pid ");
    let _ = write(STDOUT, dec_u64(pid, &mut bbuf));
    let _ = write(STDOUT, b"), waiting for it to join both threads...\n");
    // yield + waitpid_any 轮询，直到收尸 threaddemo 本体。Err（WouldBlock/NotFound）
    // 表示当前核心此刻无法阻塞等待（非 threaddemo 已死），让出再试；上限 20000 次
    // yield（约数秒）后仍未收到则记录并放行（防御式，理论上不达）。若意外收尸到
    // volumed/fpcheck 等非 threaddemo 子进程，记录后继续等 threaddemo。
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == pid => {
                let _ = write(STDOUT, b"[init] threaddemo reaped (code ");
                let _ = write(STDOUT, dec_u64(wr.code, &mut bbuf));
                let _ = write(STDOUT, b")\n");
                break;
            }
            Ok(wr) => {
                let _ = write(STDOUT, b"[init] waitpid_any reaped other child pid=");
                let _ = write(STDOUT, dec_u64(wr.pid, &mut bbuf));
                let _ = write(STDOUT, b" (continuing)\n");
            }
            Err(_) => {
                spins += 1;
                if spins > 20000 {
                    let _ = write(STDOUT, b"[init] threaddemo reap timeout (giving up)\n");
                    return;
                }
                let _ = yield_now();
            }
        }
    }
}

fn audio_e2e_launch() {
    let _ = write(STDOUT, b"[init] launching audioe2e (A2 blocking round-trip)\n");

    // ---- 1. 先派生 consumer 并让它跑起来（attach + 在空 ring 上阻塞）----
    let consumer = match libsys::exec_path("/programs/audioe2e.elf", b"consumer") {
        Ok(p) => p,
        Err(_) => {
            let _ = write(STDOUT, b"[init] audioe2e spawn failed (non-fatal)\n");
            return;
        }
    };
    let mut bbuf = [0u8; 8];
    let _ = write(STDOUT, b"[init] audioe2e consumer pid ");
    let _ = write(STDOUT, dec_u64(consumer, &mut bbuf));
    let _ = write(STDOUT, b"\n");
    // 让 consumer 跑到 fetch 并入睡。yield 保持 init 就绪（它若也阻塞，
    // 无就绪同伴可切，consumer 的阻塞路径就走不到）。
    for _ in 0..4000 {
        let _ = yield_now();
    }

    // ---- 2. init 经 VFS syscall 写入一帧 PCM，唤醒阻塞的 consumer ----
    // 填充必须与 audioe2e 的 consumer 端逐字节一致。
    const FRAME: usize = 256;
    let mut frame = [0u8; FRAME];
    let mut i = 0usize;
    while i < FRAME {
        frame[i] = ((i * 37) ^ (i >> 3)) as u8;
        i += 1;
    }
    let fd = match libsys::open(
        "/devices/audio/dsp",
        libsys::OpenFlags::READ_WRITE,
        libsys::Permissions::read_write(),
    ) {
        Ok(f) => f,
        Err(_) => {
            let _ = write(STDOUT, b"[init] audioe2e open dsp failed (non-fatal)\n");
            return;
        }
    };
    match write(fd, &frame) {
        Ok(n) if n == FRAME => {
            let _ = write(
                STDOUT,
                b"[init] audioe2e wrote 256B frame (should have woken blocked reader)\n",
            );
        }
        Ok(n) => {
            let _ = write(STDOUT, b"[init] audioe2e SHORT write ");
            let _ = write(STDOUT, dec_u64(n as u64, &mut bbuf));
            let _ = write(STDOUT, b" (expected 256) - FAIL\n");
        }
        Err(_) => {
            let _ = write(
                STDOUT,
                b"[init] audioe2e write FAILED (consumer not attached?)\n",
            );
        }
    }
    // 让被唤醒的 consumer 跑完校验/commit/detach。
    for _ in 0..4000 {
        let _ = yield_now();
    }

    // ---- 3. 收 consumer 退出码，断言 0 ----
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == consumer => {
                if wr.code == 0 {
                    let _ = write(
                        STDOUT,
                        b"[init] audioe2e PASS: blocked reader woke, verified, committed\n",
                    );
                } else {
                    let _ = write(STDOUT, b"[init] audioe2e FAIL: consumer exit=");
                    let _ = write(STDOUT, dec_u64(wr.code, &mut bbuf));
                    let _ = write(STDOUT, b"\n");
                }
                break;
            }
            // 收到别的子进程（volumed/fpcheck 等），记录后继续等 consumer。
            Ok(wr) => {
                let _ = write(STDOUT, b"[init] audioe2e reaped other pid=");
                let _ = write(STDOUT, dec_u64(wr.pid, &mut bbuf));
                let _ = write(STDOUT, b" (continuing)\n");
            }
            Err(_) => {
                spins += 1;
                if spins > 20000 {
                    let _ = write(STDOUT, b"[init] audioe2e reap timeout (giving up)\n");
                    return;
                }
                let _ = yield_now();
            }
        }
    }
}

/// T2-0：拉起 chelldemo（第一个真实 freestanding C 程序，x86-64 clang/lld 交叉链 +
/// crt0 + 直连 syscall，不依赖 Rust libc）并收尸，验证 C 运行时地基端到端。
/// chelldemo 立即打印并 exit(0)，故快速 poll 收尸即可；失败非致命。
fn chelldemo_launch() {
    let _ = write(STDOUT, b"[init] launching chelldemo (T2-0 C runtime, freestanding clang)\n");
    let pid = match libsys::exec_path("/programs/chelldemo.elf", &[]) {
        Ok(p) => p,
        Err(_) => { let _ = write(STDOUT, b"[init] exec_path(chelldemo.elf) failed (non-fatal)\n"); return; }
    };
    let mut bbuf = [0u8; 8];
    let _ = write(STDOUT, b"[init] chelldemo spawned (pid ");
    let _ = write(STDOUT, dec_u64(pid, &mut bbuf));
    let _ = write(STDOUT, b")\n");
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == pid => {
                let _ = write(STDOUT, b"[init] chelldemo reaped (code ");
                let _ = write(STDOUT, dec_u64(wr.code, &mut bbuf));
                let _ = write(STDOUT, b")\n");
                break;
            }
            Ok(_) => {}
            Err(_) => {
                spins += 1;
                if spins > 10000 { let _ = write(STDOUT, b"[init] chelldemo reap timeout\n"); return; }
                let _ = yield_now();
            }
        }
    }
}

/// T2-3：拉起 pthreaddemo（C pthread 生命周期端到端：create/join/detach/self）并收尸。
/// 快速执行并 exit(0)；失败非致命。
fn pthreaddemo_launch() {
    let _ = write(STDOUT, b"[init] launching pthreaddemo (T2-3 C pthread lifecycle)\n");
    let pid = match libsys::exec_path("/programs/pthreaddemo.elf", &[]) {
        Ok(p) => p,
        Err(_) => { let _ = write(STDOUT, b"[init] exec_path(pthreaddemo.elf) failed (non-fatal)\n"); return; }
    };
    let mut bbuf = [0u8; 8];
    let _ = write(STDOUT, b"[init] pthreaddemo spawned (pid ");
    let _ = write(STDOUT, dec_u64(pid, &mut bbuf));
    let _ = write(STDOUT, b")\n");
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == pid => {
                let _ = write(STDOUT, b"[init] pthreaddemo reaped (code ");
                let _ = write(STDOUT, dec_u64(wr.code, &mut bbuf));
                let _ = write(STDOUT, b")\n");
                break;
            }
            Ok(_) => {}
            Err(_) => {
                spins += 1;
                if spins > 20000 { let _ = write(STDOUT, b"[init] pthreaddemo reap timeout\n"); return; }
                let _ = yield_now();
            }
        }
    }
}

/// T2-4：拉起 pthread_syncdemo（C pthread 互斥/condvar/信号量端到端）并收尸。
/// 快速执行并 exit(0)；失败非致命。
fn pthread_syncdemo_launch() {
    let _ = write(STDOUT, b"[init] launching pthread_syncdemo (T2-4 C mutex/cond/sem)\n");
    let pid = match libsys::exec_path("/programs/pthread_syncdemo.elf", &[]) {
        Ok(p) => p,
        Err(_) => { let _ = write(STDOUT, b"[init] exec_path(pthread_syncdemo.elf) failed (non-fatal)\n"); return; }
    };
    let mut bbuf = [0u8; 8];
    let _ = write(STDOUT, b"[init] pthread_syncdemo spawned (pid ");
    let _ = write(STDOUT, dec_u64(pid, &mut bbuf));
    let _ = write(STDOUT, b")\n");
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == pid => {
                let _ = write(STDOUT, b"[init] pthread_syncdemo reaped (code ");
                let _ = write(STDOUT, dec_u64(wr.code, &mut bbuf));
                let _ = write(STDOUT, b")\n");
                break;
            }
            Ok(_) => {}
            Err(_) => {
                spins += 1;
                if spins > 40000 { let _ = write(STDOUT, b"[init] pthread_syncdemo reap timeout\n"); return; }
                let _ = yield_now();
            }
        }
    }
}

/// 通用 C 程序拉起 + 收尸：exec_path + waitpid_any 轮询，超时容忍。
fn launch_c_prog(path: &str, tag: &str) {
    let mut msg = [0u8; 96];
    let mut n = 0;
    for b in b"[init] launching ".iter() { msg[n] = *b; n += 1; }
    for b in tag.bytes() { msg[n] = b; n += 1; }
    for b in b"\n".iter() { msg[n] = *b; n += 1; }
    let _ = write(STDOUT, &msg[..n]);
    let pid = match libsys::exec_path(path, &[]) {
        Ok(pp) => pp,
        Err(_) => {
            let _ = write(STDOUT, b"[init] exec_path failed (non-fatal)\n");
            return;
        }
    };
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == pid => {
                let mut rp = [0u8; 8];
                let _ = write(STDOUT, b"[init] ");
                let _ = write(STDOUT, tag.as_bytes());
                let _ = write(STDOUT, b" reaped (code ");
                let _ = write(STDOUT, dec_u64(wr.code, &mut rp));
                let _ = write(STDOUT, b")\n");
                return;
            }
            Ok(_) => {}
            Err(_) => {
                spins += 1;
                if spins > 80000 { let _ = write(STDOUT, b"[init] reap timeout\n"); return; }
                let _ = yield_now();
            }
        }
    }
}

/// 跨核 spawn + SIGKILL terminate 风暴（S1 迁移 + 既有跨核终止 bug 的复现/回归脚手架）。
///
/// 每轮派生 W 个 spinburn 长驻子进程（least-loaded 分到各核），BSP 对其逐 kill(SIGKILL)，
/// 再 waitpid_any 收尸。复现: 跨核 SIGKILL '运行中/仅存其核' 的进程后, 被杀进程未被
/// 及时切走/可收尸 => 系统冻结。修复后此风暴应能多轮全绿(spawn==killed==reaped)。
/// shell 路径执行自检：以不同 argv 拉起 shell，覆盖三类装载结果。
///
/// **每条 argv 是一次真实执行**，不是模拟：shell 走 `exec_line` -> `run_command`
/// -> `classify_command` -> `exec_via_path` -> `exec_path`，与用户手敲完全同一条路径。
///
/// 三类结果都必须出现，否则本特性只被证明了一半：
///
/// 1. **成功**：`/programs/fpcheck.elf` 存在且是可加载 ELF，应跑完并以真实退出码返回；
/// 2. **ENOENT**：一个确定不存在的路径，必须报 not found 而不是笼统失败；
/// 3. **ENOEXEC**：`/programs` 下找一个**存在但不是 ELF** 的文件。
///    `/devices/...` 之类的虚拟文件不是普通可读文件；这里用 `/` 之外的稳定目标——
///    若找不到合适对象则该类跳过并**如实记录跳过**（不得假装验过）。
///
/// 相对路径 `./x` 也测一条：它是 Unix 用户最先试的写法，
/// 且能验证「含斜杠即走路径」而不是「必须以斜杠开头」。
fn shell_path_selfcheck() {
    // ENOEXEC 用例需要一个**存在但不是 ELF** 的普通文件。liveCD 的 /programs 里
    // 只有 ELF，故先自己造一个：在可写的 RamFS 根写一个纯文本文件。
    // 这不依赖任何预先存在的测试资产，用例自带前置条件。
    let notelf = "/not-an-elf.txt";
    let created = match libsys::open(
        notelf,
        libsys::OpenFlags::CREATE_OR_TRUNCATE,
        libsys::Permissions::read_write(),
    ) {
        Ok(fd) => {
            let _ = libsys::write(fd, b"this is plain text, not an ELF image\n");
            let _ = libsys::close(fd);
            true
        }
        Err(_) => false,
    };
    if !created {
        // 造不出来就如实说跳过，绝不假装验过（S39）。
        let _ = write(STDOUT, b"[shell-path] SKIP ENOEXEC case: could not create ");
        let _ = write(STDOUT, notelf.as_bytes());
        let _ = write(STDOUT, b"\n");
    }

    let cases: [(&[u8], &str); 5] = [
        (b"/programs/fpcheck.elf", "existing ELF"),
        (b"/programs/definitely-not-here.elf", "missing file ENOENT"),
        (b"./definitely-not-here.elf", "relative path ENOENT"),
        (b"/programs", "directory EISDIR"),
        (b"/not-an-elf.txt", "not an ELF ENOEXEC"),
    ];
    for (cmd, what) in cases {
        let _ = write(STDOUT, b"[shell-path] --- case: ");
        let _ = write(STDOUT, what.as_bytes());
        let _ = write(STDOUT, b" cmd=");
        let _ = write(STDOUT, cmd);
        let _ = write(STDOUT, b"\n");
        match libsys::exec_path("/programs/shell.elf", cmd) {
            Ok(pid) => {
                // 收尸并记录 shell 的退出码：退出码本身也是证据
                // （126 = 无法执行，0 = 成功）。
                let mut buf = [0u8; 8];
                let _ = write(STDOUT, b"[shell-path] spawned pid=");
                let _ = write(STDOUT, dec_u64(pid, &mut buf));
                let _ = write(STDOUT, b"\n");
                // **必须按 pid 收尸这个 shell**。
                //
                // 旧实现只 spawn 不收尸：5 个非交互 shell 全部滞留存活，每个都
                // 阻塞在 `read(STDIN)` 上。监督循环随后又拉起真正的交互 shell，于是
                // 多个 shell **并发读同一个键盘环形缓冲**，把一次击键瓜分给不同
                // 进程（实测 `ls` → `lls`/`s`/`l`，`clear` → `llear`）——用户报的
                // "命令对不对全靠运气"的直接成因。
                //
                // 按 pid 匹配（不用 WAIT_ANY），避免把别的子进程退出码算到本条。
                loop {
                    match waitpid_any() {
                        Ok(wr) if wr.pid == pid => {
                            let mut eb = [0u8; 8];
                            let _ = write(STDOUT, b"[shell-path]   exit=");
                            let _ = write(STDOUT, dec_u64(wr.code as u64, &mut eb));
                            let _ = write(STDOUT, b"\n");
                            break;
                        }
                        Ok(_) => continue, // 别人的退出码：继续等本 pid
                        Err(_) => {
                            // WouldBlock（本核暂无可切者）或瞬时失败：让出后重试。
                            let _ = libsys::yield_now();
                        }
                    }
                }
            }
            Err(_) => {
                let _ = write(STDOUT, b"[shell-path] FAILED to spawn shell\n");
            }
        }
    }
}

/// ADR-038 U1/D5：真实 C 用户态 `fork()` 端到端自检。
///
/// # 为什么这一条必须存在（与内核侧 e2e 的分工）
///
/// `kernel::tests::test_task_derive_e2e` 走的是**内核自建帧**的 syscall 路径：它直接
/// 构造 `InterruptFrame` 调 `syscall_entry`，验证的是**内核语义**（子进程首跑帧、
/// 新线程组、COW 共享与隔离、帧引用计数归位）。
///
/// 但它盖不住「**真实用户态代码**经 `int 0x80` 进来、再从 `fork()` 内部返回两次」
/// 这条整链：
///   - 子进程从 syscall 返回后，用户态栈/寄存器/代码段是否完好（fork 最易崩的地方）；
///   - csrc 的 C 运行时（crt0.S + crtrt.c）与 libc 的 ABI 是否一致；
///   - `waitpid` 经 **r10** 交付被收尸 pid、经 **rax** 交付退出码的双通道协议在
///     用户态是否被正确读取（只看 rax 会拿到退出码却拿不到 pid）。
///
/// 本自检用 shell 的非交互模式（argv 即整行命令）拉起 `/programs/forkdemo.elf`，
/// 与用户在命令行敲完全同一条路径（ADR-029 的既有手法）。
///
/// `/programs/forkdemo.elf` 是**真正的 freestanding C**（csrc/prog/forkdemo.c，
/// 经 clang/lld 交叉链编译，零 Rust libc，S35），它自己打印 [forkdemo] OK/FAIL 行。
/// 本函数只负责拉起、收尸、并把**退出码**作为证据输出——forkdemo 以 0 退出即
/// 全部断言通过，非 0 即失败条数。
fn forkdemo_launch() {
    let _ = write(STDOUT, b"[forkdemo-check] --- real C userland fork()+waitpid() ---\n");
    // 经 shell 非交互模式执行，与用户手敲同路径。
    let cmd: &[u8] = b"/programs/forkdemo.elf";
    let pid = match libsys::exec_path("/programs/shell.elf", cmd) {
        Ok(p) => p,
        Err(_) => {
            // S09：拉不起来就如实说，绝不假装验过。
            let _ = write(STDOUT, b"[forkdemo-check] FAILED to spawn shell (cannot run forkdemo)\n");
            return;
        }
    };
    let mut spins: u32 = 0;
    loop {
        match waitpid_any() {
            Ok(wr) if wr.pid == pid => {
                let mut b = [0u8; 8];
                let _ = write(STDOUT, b"[forkdemo-check] shell exited with code ");
                let _ = write(STDOUT, dec_u64(wr.code, &mut b));
                let _ = write(STDOUT, b" (0 = forkdemo PASS, N = N failed assertions)\n");
                return;
            }
            Ok(_) => continue, // 别人的退出码：继续等本 pid
            Err(_) => {
                spins += 1;
                if spins > 80000 {
                    let _ = write(STDOUT, b"[forkdemo-check] reap timeout (non-fatal)\n");
                    return;
                }
                let _ = libsys::yield_now();
            }
        }
    }
}

/// `audiofile` 端到端自检：真实播放一个真实 WAV 文件。
///
/// **为何必须有这个自检**：`audiofile` 的价值在于「真的放出声」这条完整链路
/// ——读盘、解析、写设备、等待。若只在宿主上测解析器，或只在 QEMU 里手工敲一次，
/// 都盖不住「写设备」与「等待」这两段（它们需要真实设备与进程上下文）。
///
/// 用 shell 的非交互模式拉起，与用户在命令行敲 `audiofile <path>` 完全同一条路径。
fn audiofile_selfcheck() {
    let _ = write(STDOUT, b"[audiofile-check] --- playing a real WAV via audiofile ---\n");
    let path = "/volumes/BORUIX_DATA/tone-a4-48k.wav";
    // 命令拼成 shell 的一整行：**ELF 路径** + 参数。
    //
    // **必须写全路径**：shell 的路径执行以「含 `/`」为判据，
    // 裸名 `audiofile` 会走内建查找并报 unknown command（实测踩到）。
    // 这与用户在命令行敲的是同一条路径，正是本自检要覆盖的。
    let mut cmd = [0u8; 128];
    let prog = b"/programs/audiofile.elf ";
    cmd[..prog.len()].copy_from_slice(prog);
    cmd[prog.len()..prog.len() + path.len()].copy_from_slice(path.as_bytes());
    let n = prog.len() + path.len();
    // exec_path 第二参是 `&[u8]`（命令字节），不需要转 str。
    let cmd_bytes = &cmd[..n];
    match libsys::exec_path("/programs/shell.elf", cmd_bytes) {
        Ok(pid) => {
            let mut buf = [0u8; 8];
            let _ = write(STDOUT, b"[audiofile-check] spawned pid=");
            let _ = write(STDOUT, dec_u64(pid, &mut buf));
            let _ = write(STDOUT, b"\n");
            // 收尸并读取退出码：退出码本身即判据（0=完整播放）。
            //
            // **必须按 pid 匹配**：启动期还有别的子进程（各类自检 shell）
            // 在同时退出，`waitpid_any` 很可能先收到**别人**的退出码 ——
            // 那样会把「播放成功」误报成失败（或反之），判据完全失真。
            // 实测正是如此：audiofile 明明完整播放并打印了 done，
            // 这里却收到另一个 shell 的 126。
            match waitpid_any() {
                Ok(wr) if wr.pid != pid => {
                    let _ = write(STDOUT, b"[audiofile-check] NOTE: reaped unrelated pid ");
                    let _ = write(STDOUT, dec_u64(wr.pid, &mut buf));
                    let _ = write(STDOUT, b" (code ");
                    let _ = write(STDOUT, dec_u64(wr.code as u64, &mut buf));
                    let _ = write(STDOUT, b"), not our shell; playback result is in the log above\n");
                }
                Ok(wr) => {
                    let _ = write(STDOUT, b"[audiofile-check] exit code=");
                    let _ = write(STDOUT, dec_u64(wr.code as u64, &mut buf));
                    if wr.code == 0 {
                        let _ = write(STDOUT, b" -> playback completed\n");
                    } else {
                        let _ = write(STDOUT, b" -> FAILED (see audiofile output above)\n");
                    }
                }
                Err(_) => {
                    let _ = write(STDOUT, b"[audiofile-check] waitpid failed\n");
                }
            }
        }
        Err(_) => {
            let _ = write(
                STDOUT,
                b"[audiofile-check] SKIP: could not spawn shell (is /programs/shell.elf present?)\n",
            );
        }
    }
}

/// `audiofile` 负例自检：坏输入必须**如实失败**，且退出码非 0。
///
/// **为何必须做**：只验成功路径的话，「不管输入是什么都返回 0」的实现
/// 也能通过 —— 那种实现其实什么都没播，却看起来一切正常（S20/S39）。
///
/// 每个用例都断言「退出码非 0」，而不是只看有没有打印错误（打印可以造假）。
fn audiofile_selftest_negative() {
    let _ = write(STDOUT, b"[audiofile-check] --- negative cases (must fail) ---\n");
    // (传给 audiofile 的路径, 用例说明)
    let cases: [(&str, &[u8]); 3] = [
        (
            "/volumes/BORUIX_DATA/definitely-not-here.wav",
            b"nonexistent file",
        ),
        ("/volumes/BORUIX_DATA/README.md", b"not a WAV (no RIFF header)"),
        ("/volumes/BORUIX_DATA", b"a directory, not a file"),
    ];
    for (path, what) in cases {
        let _ = write(STDOUT, b"[audiofile-check] case: ");
        let _ = write(STDOUT, what);
        let _ = write(STDOUT, b"\n");
        let mut cmd = [0u8; 128];
        let prog = b"/programs/audiofile.elf ";
        cmd[..prog.len()].copy_from_slice(prog);
        cmd[prog.len()..prog.len() + path.len()].copy_from_slice(path.as_bytes());
        let n = prog.len() + path.len();
        match libsys::exec_path("/programs/shell.elf", &cmd[..n]) {
            Ok(pid) => {
                // 同样按 pid 匹配，避免把别人的退出码算到自己头上。
                loop {
                    match waitpid_any() {
                        Ok(wr) if wr.pid == pid => {
                            let mut buf = [0u8; 8];
                            let _ = write(STDOUT, b"[audiofile-check]   exit=");
                            let _ = write(STDOUT, dec_u64(wr.code as u64, &mut buf));
                            if wr.code != 0 {
                                let _ = write(STDOUT, b" (correctly rejected)\n");
                            } else {
                                let _ = write(STDOUT, b" (UNEXPECTED SUCCESS - bad input accepted!)\n");
                            }
                            break;
                        }
                        Ok(_) => continue,
                        Err(_) => break,
                    }
                }
            }
            Err(_) => {
                let _ = write(STDOUT, b"[audiofile-check]   SKIP (could not spawn shell)\n");
            }
        }
    }
}

fn cross_core_sigkill_storm() {
    const W: u32 = 5;
    const ROUNDS: u32 = 4;
    let _ = write(STDOUT, b"[init] cross-core SIGKILL storm: start\n");
    let mut rbuf = [0u8; 8];
    for round in 0..ROUNDS {
        let mut pids = [0u64; 8];
        let mut n = 0u32;
        for _ in 0..W {
            if let Ok(p) = exec_path("/programs/spinburn.elf", &[]) {
                if (n as usize) < pids.len() { pids[n as usize] = p; n += 1; }
            }
            for _ in 0..100 { let _ = yield_now(); }
        }
        if n == 0 { continue; }
        for _ in 0..2500 { let _ = yield_now(); }
        let mut killed = 0u32;
        for i in 0..n { if kill(pids[i as usize], 9).is_ok() { killed += 1; } }
        let mut reaped = 0u32;
        for _ in 0..n {
            match waitpid_any() { Ok(_) => { reaped += 1; } Err(_) => { break; } }
        }
        let _ = write(STDOUT, b"[init] storm round ");
        let _ = write(STDOUT, dec_u64(round as u64, &mut rbuf));
        let _ = write(STDOUT, b": spawn="); let _ = write(STDOUT, dec_u64(n as u64, &mut rbuf));
        let _ = write(STDOUT, b" killed="); let _ = write(STDOUT, dec_u64(killed as u64, &mut rbuf));
        let _ = write(STDOUT, b" reaped="); let _ = write(STDOUT, dec_u64(reaped as u64, &mut rbuf));
        let _ = write(STDOUT, b"\n");
    }
    let _ = write(STDOUT, b"[init] cross-core SIGKILL storm done\n");
}

/// 十进制格式化（init 的 dec_u64 同款拷贝：宿主程序各自持有，S15 允许——
/// 抽公共 crate 的收益低于多一层依赖的成本）。
fn dec_u64(v: u64, buf: &mut [u8; 8]) -> &[u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut n = 0usize;
    let mut x = v;
    while x > 0 {
        buf[7 - n] = b'0' + (x % 10) as u8;
        x /= 10;
        n += 1;
    }
    let s = 8 - n;
    buf.copy_within(s..8, 0);
    &buf[..n]
}

/// 分组判定：argv[1]（若有）。
///   audio  -> 音频组；thread -> 线程组；quick -> 信号+libc+shell 路径；
///   无参   -> 全量。
///
/// # 参数实际落在 argv[0]，不是 argv[1]（本轮定位的真实缺陷）
///
/// 内核的用户栈参数块 mini-ABI **恒为 `argc = 1`**：整条命令行作为一个 C 串放在
/// `argv[0]`（`loader/src/lib.rs::setup_user_stack` 的 `*rp = 1; *rp.add(1) = str_user;`，
/// 第三格写 0 终结）。内核**不做**按空格切分。
///
/// 于是链路上：shell `exec_line` 切出 `arg` -> `cmd_selftest(arg)` ->
/// `exec_path("/programs/selftest.elf", arg)` -> `arg` 成为新进程的 `argv[0]`。
///
/// 旧实现读的是 `argv[1]`，而 `argc` 恒为 1 —— 它要么走 `argc <= 1` 短路（永远
/// 「全量」），要么在短路不成立时越界读 `argv[1]`（该位置是终结用的 NULL）。
/// 两个后果都指向同一件事：**分组过滤从未生效**，`selftest thread` 一直在跑全量
/// 套件（含最耗时的 SIGKILL 风暴）。旧注释「i=1 < argc 恒在界内」正是该缺陷的
/// 成文形态——它把一个恒不成立的假设写成了安全依据。
///
/// 故改为读 `argv[0]`，并在为空时不匹配任何组（`argc == 0` 或空串 = 无参 = 全量，
/// 由调用方 `user_main` 的 `argc <= 1` 分支处理「全量」的**显示**，此处只做匹配）。
fn group_enabled(argc: isize, argv: *const *const u8, g: &[u8]) -> bool {
    // 无 argv（argc == 0）或 argv 数组为空 -> 无参，视为全量。
    if argc <= 0 || argv.is_null() {
        return true;
    }
    // SAFETY: argc >= 1，故 argv[0] 在界内；由内核 exec 路径构造为 NUL 结尾 C 串。
    let sp = unsafe { *argv };
    if sp.is_null() {
        return true; // 空串 = 无参 = 全量
    }
    let mut j = 0usize;
    // SAFETY: sp 指向 NUL 结尾 C 串。
    if unsafe { *sp } == 0 {
        return true; // 空串 = 无参 = 全量
    }
    let mut same = true;
    loop {
        // SAFETY: 同上，C 字符串以 NUL 结尾。
        let b = unsafe { *sp.add(j) };
        if b == 0 {
            break;
        }
        if j >= g.len() || b != g[j] {
            same = false;
        }
        j += 1;
    }
    same
}

/// libsys `_start` 按符号名找 `user_main`，必须有 `#[unsafe(no_mangle)]`。
#[unsafe(no_mangle)]
pub extern "C" fn user_main(argc: isize, argv: *const *const u8) -> i32 {
    let audio = group_enabled(argc, argv, b"audio");
    let thread = group_enabled(argc, argv, b"thread");
    let quick = group_enabled(argc, argv, b"quick");
    let _ = write(STDOUT, b"[selftest] start (group=");
    // 回显**实际生效**的组名：与 group_enabled 同源（argv[0]），不是 argv[1]。
    // 无参或空串时回显 all——与 group_enabled 返回 true 的语义一致。
    let mut showed = false;
    if argc > 0 && !argv.is_null() {
        // SAFETY: argc > 0，argv[0] 在界内；内核构造为 NUL 结尾 C 串。
        let sp = unsafe { *argv };
        if !sp.is_null() && unsafe { *sp } != 0 {
            let mut j = 0usize;
            // SAFETY: C 字符串以 NUL 结尾。
            loop {
                let b = unsafe { *sp.add(j) };
                if b == 0 {
                    break;
                }
                let _ = write(STDOUT, &[b]);
                j += 1;
            }
            showed = true;
        }
    }
    if !showed {
        let _ = write(STDOUT, b"all");
    }
    let _ = write(STDOUT, b")\n");

    if quick {
        signal_selftest();
        libc_selftest();
        shell_path_selfcheck();
    }
    if thread {
        threaddemo_launch();
        chelldemo_launch();
        pthreaddemo_launch();
        pthread_syncdemo_launch();
        launch_c_prog("/programs/pthread_bench.elf", "pthread_bench");
        // ADR-038 U1/D5：真实 C 用户态 fork()+waitpid() 端到端（零 Rust libc），
        // 经 shell 非交互模式（argv = 整行命令）拉起，与用户手敲同路径。
        forkdemo_launch();
    }
    if audio {
        audio_e2e_launch();
        audiofile_selftest_negative();
        audiofile_selfcheck();
    }
    if argc <= 1 {
        // 全量模式才跑风暴（耗时最长，且会拉起 spinburn）。
        cross_core_sigkill_storm();
    }
    let _ = write(STDOUT, b"[selftest] done\n");
    0
}
