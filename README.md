# selftest

BORUIX 的**系统自检宿主**：按需拉起全部自检项并汇总结果。

[English](README.en.md)

## 用它做什么

```
selftest            全量跑
selftest quick      快速组（信号 + C 库 + shell 路径，秒级）
selftest audio      音频组
selftest thread     线程组
```

**为什么自检不放在开机流程里**：自检是"跑给人看"的测试负载，没有实时观察价值，而全量跑包含数段
实时音频流，会让 `shell` 迟到数分钟。现在全部收拢到本程序，由 `shell` 命令按需拉起——**开机直达
`shell`**。

## 自检覆盖

| 组 | 内容 |
| --- | --- |
| 信号 | 信号的投递与处置；`SIGKILL` 风暴下的进程终止 |
| C 库 | 内存分配、字符串、格式化输出、数值转换、时间等链路 |
| shell 路径 | 以多组不同参数拉起 `shell`，覆盖三类装载结果 |
| 线程 | 线程创建与调度、每线程 `errno`、线程局部存储、共享地址空间 |
| 音频 | 音频域阻塞往返、WAV 播放正负例 |

其中多项以**真实子进程**方式运行——走真实的装载与执行路径，而非在进程内模拟。

## 参数说明

分组参数经**启动参数**传入（`shell` 的 `selftest <组名>`）。参数为空或省略时视为**全量**。

程序启动时会回显**实际生效**的组名，便于确认参数被正确解读。

## 退出码

| 退出码 | 含义 |
| --- | --- |
| `0` | 所选组全部通过 |
| 非零 | 有失败项 |

## 构建

```bash
cargo build --release
```

编译产物部署为 `/programs/selftest.elf`。

## 文件结构

```
selftest/
├── Cargo.toml    # 包定义
├── build.rs      # 注入链接脚本
├── linker.ld     # 用户态段布局
└── src/
    └── main.rs   # 分组调度与各项自检
```

## 相关项目

- [`shell`](https://github.com/BRX-Boruix/shell) —— 提供 `selftest` 命令
- [`pwde2e`](https://github.com/BRX-Boruix/pwde2e)、[`acee2e`](https://github.com/BRX-Boruix/acee2e)、
  [`trave2e`](https://github.com/BRX-Boruix/trave2e) —— 本宿主拉起的独立验收程序
- [`libsys`](https://github.com/BRX-Boruix/libsys) —— 用户态系统调用封装

## 许可

MIT License，版权归 Yang Borui 所有。详见 [LICENSE](LICENSE)。
