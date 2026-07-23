```
--- 3-LEVEL PAGE TABLE TRANSLATION (vaddr -> paddr) ---

get root table PPN from register (RISC-V: satp holds root PPN, not raw addr)
split vaddr -> (L1 index, L2 index, L3 index) + page offset

walk:
  L2_table_addr = root.entries[L1_index].ppn << 12
  L3_table_addr = L2_table.entries[L2_index].ppn << 12
  page_addr     = L3_table.entries[L3_index].ppn << 12
  target_paddr  = page_addr + offset

NOTE: a leaf can appear at ANY level, not just L3.
  leaf at L1 -> 1GB superpage
  leaf at L2 -> 2MB superpage
  leaf at L3 -> normal 4KB page


--- PAGE TABLE ENTRY (PTE) STRUCTURE ---

layout (RISC-V Sv39):
  bits [53:10] = PPN (physical page number)
  bits [9:0]   = flags: V R W X U G A D

there is NO dedicated "is leaf" bit. leaf-ness is derived:
  R==0 && W==0 && X==0  -> pointer to next-level table (non-leaf)
  any of R/W/X set      -> leaf (points to real page/superpage)
  W=1,R=0               -> reserved/illegal

physical addr extraction:
  mask flags -> extract PPN field -> paddr = (PPN << 12) | offset
  (PPN is NOT the whole entry shifted; flags live in the low bits)


--- MEMORY COST (use powers of two, NOT base-10) ---

1 GiB = 2^30 bytes, 4 KiB page = 2^12
entries to cover 1 GiB = 2^30 / 2^12 = 2^18 = 262,144 entries
each entry 8 bytes -> 2^18 * 2^3 = 2^21 = 2 MiB

per-table size: 4 KiB / 8 bytes = 512 entries = 2^9 (9 index bits per level)



SINGLE-LEVEL:
  must cover the entire VIRTUAL address space up front.
  every entry reserved whether mapped or not.
  size scales with VA space size, NOT physical RAM
  (full 39-bit VA single-level table = absurdly huge -> reason multi-level exists)

TWO-LEVEL (1 GiB span example):
  root table: 4 KiB (one table)
  leaf tables: up to 512 * 4 KiB = 2 MiB, allocated ON DEMAND

  fully-mapped case: ~2 MiB + 4 KiB
    -> slightly MORE than single-level (extra root + rounding).
       multi-level is a net loss when everything is mapped.

  sparse/typical case (real process: code + heap + stack):
    root + ~3 leaf tables ≈ 16 KiB total instead of 2 MiB
    -> THIS is the win. sparse address spaces cost almost nothing.
```