# easy-fs 结构笔记

参考实现 easy-fs 的结构笔记，数字均为 easy-fs 的参数。本 crate 自己的设计见
`DESIGN.md`，实际常量见 `src/layout.rs`；两者在下文标注处有意不同。

硬盘被抽象为一段连续地址空间（字节数组），文件系统在其上划分为 5 个区。

分两层理解：
- **逻辑层**：假设存储扁平、可按字节寻址，描述结构与索引逻辑。
- **物理层**：加入"块设备只能按 block 读写"这一硬约束，得出额外限制。

---

## 逻辑层：五区结构

### 1. superblock
文件系统元数据，包含：
- magic：校验文件系统合法性
- total_blocks：设备总块数
- 其余 4 个区各自的**大小**（块数）：inode bitmap / inode area / data bitmap / data area

各区 offset 不存储，由各区大小顺序累加得出。root 不在 superblock 中，约定为 **inode 0**。

### 2. inode bitmap
连续 bit 数组，第 i 位表示 inode number = i 是否已分配。

### 3. data bitmap
连续 bit 数组，第 i 位表示第 i 个 data block 是否已分配。
第 i 块物理位置 = data area offset + block_size * i。

### 4. inode area
连续的 inode（`DiskInode`）数组，每个 inode 记录：
- 类型：File / Directory
- 持有的 data blocks，用多级索引表表示，级数决定单文件最大大小：
  - `direct`：`[u32; 28]`，28 个 block id 直接指向 data block
  - `indirect1`：一个 block id，指向"装满 block id"的块（一级间接），覆盖 512/4 = 128 块
  - `indirect2`：一个 block id，指向一个块，其每个 entry 又是 indirect1 式的块（二级间接），覆盖 128 × 128 = 16384 块

索引项为 **block id（u32）**，非机器地址。
单文件上限 = (28 + 128 + 16384) × 512 ≈ 8 MB。

### 5. data blocks
一个 data block 内容有三种形态：
- 纯数据
- 索引表（indirect1 / indirect2 指向的块）
- 目录项（direntry）：定长 32 字节 = `inode_number(u32, 4) + name_len(u8, 1) + name([u8; 27], 27)`。用 `name_len` 记名字字节数，**不用 null 结尾**——名字可含任意字节、O(1) 取长度、无需扫描终止符。名字上限 27 字节。每块容 512/32 = 16 项

---

## 物理层：block size 约束

块设备只能按整块读写（`BlockDevice::read_block/write_block`），无法读单字节。block size 是介质硬约束，非性能优化。由此产生的限制：

### 各区块对齐
所有区按 block 对齐，superblock 独占 block 0。

### inode 不跨块
inode 大小须整除 block size。`DiskInode` 精确凑成 128 字节 = size(u32,4) + type(4) + direct(28×4=112) + indirect1(4) + indirect2(4)，`BLOCK_SZ = 512`，一块正好 4 个 inode。定位 inode i：
```
block_id     = inode_area_start + i / (BLOCK_SZ / inode_size)
in_block_off = (i % (BLOCK_SZ / inode_size)) * inode_size
```
128 | 512 保证任一 inode 完整落在单块内，一次读写即可取回。

### 最小占用以块为粒度
- 空文件 / 空目录（size == 0）：占 0 个 data block，仅占 inode 区一个槽位。
- 目录含 ≥1 个 direntry：至少占 1 个 data block（即使只有 32 字节一项，也占满整块）。
- easy-fs 目录不含 `.` 和 `..`，故新建空目录为 0 块。
- data block 按需分配（写入时 `increase_size`），不预分配。

---

## 数据索引的三个正交驱动力

inode 数据索引的设计由三个独立因素决定，不可混为一谈。

### 索引为何存在：非连续分配
根源是**文件大小动态变化**。连续存放会导致：文件增长被后续文件堵塞需整体搬迁；反复增删产生外部碎片。解法同物理内存——空间切成定长块、允许一个文件的块散落任意位置，用"逻辑块号 → 物理块号"映射串联。

本质是 **frame allocator 思路**，与 block size 无关：data 区 = frame 池，data bitmap = 分配位图。

### frame 大小为何等于 block size：纯 block 约束
设备只能按块 I/O，frame 粒度下界卡在 block size，故锁为 block size（真实系统常取整数倍，如 4KB cluster over 512B sector）。此项是唯一真正源于 block 约束的部分。

### 索引为何多级：文件大小分布
针对"多数文件小、少数文件大"的分布做空间优化。direct 内联使小文件零额外开销；indirect1/2 按需分配使大文件可扩展，且不拖累每个 inode。与 block 约束无关。

### 本质等价：文件系统 = 磁盘上的分页

| 物理内存 | easy-fs |
|---|---|
| 物理 frame | data block |
| frame allocator + 位图 | data 区 + data bitmap |
| 多级页表 | inode 的 direct/indirect 索引 |
| 虚拟地址 → 物理地址 | 文件 offset（逻辑块号）→ 物理 block id |
| 缺页按需分配 | 写入时 `increase_size` 按需分配 |

差异：页表定长辐射、覆盖整个地址空间；inode 索引不对称（direct 内联 + 两级 indirect），此不对称即小文件优化。

---

## 目录的增删与空洞

目录是定长 32 字节槽位（direntry）的数组，`create` 靠 `increase_size` 在**末尾追加**一项。删除非末尾项时是否留空洞，取决于删除策略。

### 策略一：朴素置空（tombstone）——留空洞
把目标槽标记为空。代价三处：
- 哨兵**不能用 `inode_number == 0`**（inode 0 是 root），用 `name_len == 0` 表示空槽（合法项名字非空）。
- 每个 reader（`dir_lookup` / `dir_list`）必须跳过空槽。
- 目录 `size` 不缩、块不释放；`create` 须先扫空槽复用，否则反复增删下目录无限膨胀。

### 策略二：末位交换 + 缩容——不留空洞
删第 k 项时，把最后一项搬进第 k 槽，再缩掉 32 字节（空出整块则归还 data bitmap）。
- 永不留空洞，无需哨兵，reader 可无脑遍历 `size/32` 项且全部有效。
- 唯一代价：不保序。但 POSIX `readdir` 不保证顺序，无语义损失。
- 需要 `increase_size` 的逆操作（算新块数、尾块还 bitmap、改 size）；文件截断亦需此函数。

rfs 取策略二，逆操作即 `Fs::set_len`。

### 对照分页
留空洞 = 页表项保留 valid 位（sparse）；末位交换 = 目录可压实，因为**条目顺序无语义**（页表项顺序有语义，不能压实）。此为目录与页表的关键差异。

`rmdir` 前置检查 `dir_is_empty`；策略二下"空"直接等价 `size == 0`。


---

## 目录操作 API

分两层：**inode 层**（参数是已知 inode id 的目录）与**路径层**（参数是 path 字符串）。

### 依赖关系
```
dir_lookup / dir_list / dir_is_empty   ← inode 层原语，读该 dir inode 的 direct/indirect 块，逐块读 direntry
resolve      ← 逐层重复 dir_lookup，每个 '/' 一层，每层只挑出命名的那一个子项
path_ops     ← resolve 拿到目标 inode 后，再调 inode 层原语
```
读目录内容都要遍历该 inode 的直接/间接 datablock 指针。

### inode 层原语
```
dir_lookup(dir, name) -> Option<u32>   // 单层查名
```
- 与 `dir_list` 同一趟遍历；区别仅在终止动作：lookup **命中即短路返回**，list 扫完。
```
dir_list(dir) -> Vec<DirEntry>         // ls
```
- 遍历 dir inode 的所有数据块，收集全部有效 direntry。
```
dir_is_empty(dir) -> bool
```
- 策略二（无空洞）下**直接 `size == 0`，O(1)**，无需遍历。仅 tombstone 策略才需扫描。

### 路径层
```
resolve(path) -> Option<u32>           // 路径 → inode，一层一个 '/'
```
- 构件是 `dir_lookup`（**非** `dir_list`）：从起始 inode 起，按组件逐层 lookup，任一层缺失即 `None`。
- 反向关系：路径版 `dir_list(path)` = `resolve(path)` + inode 层 `dir_list`。

### 变更操作
```
create(dir, name, kind) -> Option<u32> // 建文件/目录
```
前置：**父目录存在 且 name 不存在**（重名失败）。有序步骤：
1. 确认父目录存在且是 dir
2. `dir_lookup` 确认 name 不存在
3. 从 inode bitmap 分配新 inode id
4. 初始化新 DiskInode（写类型；若为目录则空目录 `size=0`，无 `.`/`..`）
5. 向父目录 append direntry `{name, new_id}`（`increase_size` + 写）
6. 返回 new_id

顺序要点：**先分配并初始化 inode，再挂 direntry**，否则中途崩溃留悬空项。
```
remove(dir, name) -> bool              // unlink / rmdir
```
1. `dir_lookup` 找到目标 inode；不存在返回 false
2. 目录需先 `dir_is_empty` 检查，非空 rmdir 拒绝；unlink 文件 vs rmdir 目录分流
3. 从父目录删 direntry：**末位交换 + `decrease_size`**（策略二，不留空洞）
4. 释放目标 inode（见下），成功返回 true

**释放 inode 的两步，缺一不可**：
- **归还所有 data block**：`clear_size` 把该 inode 的 direct/indirect 数据块全部还给 data bitmap。**这是空间大头，漏了就永久泄漏。**
- 从 inode bitmap 释放 inode id。

### 硬链接
- **不支持**：无 `nlink` 概念，remove 无条件执行上面两步释放。easy-fs 如此。
- **支持**：DiskInode 加 `nlink: u32`，remove 改为递减、归零才释放。代价：128 字节已排满，加字段须从 direct 里让出位置。
- 另有 in-memory **open-count**（打开句柄数）与 `nlink` 无关，仅内存、不落盘，用于"unlink 后延迟到最后 close 才真删"的 POSIX 语义。

rfs 支持硬链接（`Fs::link`），`nlink` 与 `_reserved` 各占 4 字节，direct 因此为 26 项。