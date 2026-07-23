os mem subsystem init
1. os starting without pagetable
2. setup temporary directmapping page table
3. turn on paging
4. setup kernel page table
  - direct mapping for texts, per cpu stack, global var
  - setup mapping for mmio devices/interrupts
  - setup mapping area for heap
  - setup trampoline page for process
  - ?else
5. switch page table


# Virtual Memory (RISC-V Sv39, Linux model)

每个进程有独立的页表。**用户部分各进程不同，内核部分在所有进程间共享（同一套映射）。**

## 地址空间布局 (Sv39)

Sv39 只用低 39 位，且要求 bit[63:39] 全部等于 bit 38（符号扩展 / canonical form）。
所以整个地址空间被劈成两半，中间是一段不可用的空洞——访问即 fault。

```
地址向下递增 ↓

0x00000000_00000000 ┐
                    │  用户空间 (低半部, PTE.U=1)
0x0000003f_ffffffff ┘
        ...            非 canonical 空洞 (不可用)
0xffffffc0_00000000 ┐
                    │  内核空间 (高半部, supervisor-only, PTE.U=0)
0xffffffff_ffffffff ┘
```

- 用户空间 = **低**半部；内核空间 = **高**半部。
- x86-64 / arm64 / riscv64 方向一致：内核在高，用户在低。
- 注意：user stack 位于**用户区内部**的高地址端、向下增长。那是"用户区里的高地址"，
  不是"整个地址空间的高地址"——别把这两件事混为一谈。

## 用户空间内部布局（低 → 高）

- code / data (.text / .data / .bss)
- heap（brk，向上增长）
- mmap 区
- stack（向下增长）

## 内核空间的几个区域

- **kernel image**：内核 text/data 的映射
- **direct map（线性映射区）**：把所有物理内存连续映射进内核 VA，`va = pa + PAGE_OFFSET`
  （即 `__va()` / `__pa()`）。作用：内核可以用简单加减法访问任意物理页。这是常驻映射。
- **vmalloc**：虚拟连续、物理可不连续的内核分配区
- **kernel stacks**：每个线程一个内核栈
- **per-CPU areas**：每个 CPU 一份的数据

### per-CPU

所有 CPU 使用相同的内核地址空间布局，但每个 CPU 有自己的 per-CPU base(offset)：
同一个 per-CPU 符号 + 当前 CPU 的 offset → 落到该 CPU 自己的那份实例。
（x86-64 用 `gs` base 存放当前 CPU 的 offset；机制随架构不同，模型都是 base + offset。）

## 进程页表 = 内核基础映射 + 该进程的用户映射

```
进程 A 页表                     进程 B 页表
(地址向下递增)                  (地址向下递增)

├── A code/data                 ├── B code/data     ┐
├── A heap                      ├── B heap          │ 用户部分
├── A stack                     ├── B stack         │ 各进程不同
├── A mmap                      ├── B mmap           ┘
│                               │
└── 内核映射（所有进程共享）    └── 内核映射（所有进程共享） ┐
    ├── kernel image                ├── kernel image        │
    ├── direct map                  ├── direct map          │ 共享同一套
    ├── vmalloc                     ├── vmalloc              │ (共享顶层页表项)
    ├── kernel stacks               ├── kernel stacks        │
    └── per-CPU areas               └── per-CPU areas        ┘
```

- **base**：一套完整的内核空间映射（Linux 中对应 `swapper_pg_dir` / `init_mm.pgd`）。
- 每个进程页表 = base 的内核部分（共享顶层页表项）+ 自己的用户映射。
  "base + process mapping" 可理解为逻辑组合。

## 启动时的内存初始化

开启分页（写 `satp`）的一瞬间，PC 和 SP 立刻开始被翻译。若当前正在执行的代码 / 栈
没有被映射，就会立即 fault。因此顺序大致是：

1. 先建一个 **identity / trampoline 映射（VA == PA）**，让当前代码在开分页后还能继续跑。
   —— 注意这是**临时映射**，跟上面常驻的 direct map（线性映射）不是一回事，别都叫 "direct mapping"。
2. 建好内核最终的高地址映射，以及其它映射。
3. 写 `satp` 切到内核页表；（若走 trampoline）跳到高地址 VA，再拆掉临时的 identity 映射。

## kernel thread

内核线程没有用户地址空间（`mm == NULL`），只运行在内核映射上。
调度进来时借用上一个进程页表的内核部分即可（lazy TLB）——反正内核映射各进程都一样。
