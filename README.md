# selftest

BORUIX 的开机自检宿主：把信号、C 库、线程、音频、shell 执行路径等测试按组组织，按需运行。

[English](README.en.md)

## 用法

由 shell 的内建命令拉起，分组参数决定跑哪些组：

- `selftest`——全量，含跨核终止风暴
- `selftest quick`——信号、C 库、shell 路径、账户查询，秒级完成
- `selftest thread`——线程组：多线程演示、C pthread 系列及其基准、fork 往返
- `selftest audio`——音频组：阻塞唤醒往返、播放程序正负例

启动时回显实际生效的组名，无参数时回显 `all`。各项测试逐项打印通过或失败，全部跑完打印
`[selftest] done`。退出码恒为 0，判定看输出行。

## 已知限制

- 音频组的往返一项需要独占音频消费者槽位；硬件驱动常驻时该槽被占，这一项如实打印 SKIP 而不是 FAIL
- 全量模式耗时最长，且会拉起压测进程

## 构建

```bash
cargo build --release
```

## 文件结构

```
selftest/
├── Cargo.toml    # 包定义
├── build.rs      # 注入链接脚本
├── linker.ld     # 用户态段布局
└── src/
    └── main.rs   # 各测试组与拉起协调
```

## 相关项目

- [`threaddemo`](https://github.com/BRX-Boruix/threaddemo) —— 线程组拉起的验收程序
- [`audioe2e`](https://github.com/BRX-Boruix/audioe2e) —— 音频组拉起的往返测试
- [`pwde2e`](https://github.com/BRX-Boruix/pwde2e) —— quick 组拉起的账户查询验收
- [`shell`](https://github.com/BRX-Boruix/shell) —— 拉起本程序的内建命令

## 许可

MIT License，版权归 Yang Borui 所有。详见 [LICENSE](LICENSE)。
