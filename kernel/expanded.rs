#![feature(prelude_import)]
#![no_std]
#![no_main]
#![feature(const_panic, panic_info_message)]
#![feature(core_intrinsics)]
#![feature(global_asm, llvm_asm, asm)]
#![feature(alloc_error_handler)]
#![feature(alloc_prelude)]
#[prelude_import]
use core::prelude::rust_2018::*;
#[macro_use]
extern crate core;
#[macro_use]
extern crate compiler_builtins;
extern crate alloc;

use core::intrinsics::volatile_load;
use core::sync::atomic::{AtomicBool, Ordering};

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate static_assertions;
#[macro_use]
extern crate bitflags;

use log::{info, warn, LevelFilter};
use spin::Mutex;

#[macro_use]
mod console {













    use core::fmt::{self, Write};
    use spin::Mutex;
    use crate::sbi::console_putchar;
    struct Stdout;
    impl Write for Stdout {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for c in s.chars() { console_putchar(c as usize); }
            Ok(())
        }
    }
    #[allow(missing_copy_implementations)]
    #[allow(non_camel_case_types)]
    #[allow(dead_code)]
    struct STDOUT {
        __private_field: (),
    }
    #[doc(hidden)]
    static STDOUT: STDOUT = STDOUT{__private_field: (),};
    impl ::lazy_static::__Deref for STDOUT {
        type Target = Mutex<Stdout>;
        fn deref(&self) -> &Mutex<Stdout> {
            #[inline(always)]
            fn __static_ref_initialize() -> Mutex<Stdout> {
                Mutex::new(Stdout)
            }
            #[inline(always)]
            fn __stability() -> &'static Mutex<Stdout> {
                static LAZY: ::lazy_static::lazy::Lazy<Mutex<Stdout>> =
                    ::lazy_static::lazy::Lazy::INIT;
                LAZY.get(__static_ref_initialize)
            }
            __stability()
        }
    }
    impl ::lazy_static::LazyStatic for STDOUT {
        fn initialize(lazy: &Self) { let _ = &**lazy; }
    }
    pub fn print(args: fmt::Arguments) {
        STDOUT.lock().write_fmt(args).unwrap();
    }
    #[macro_export]
    macro_rules! print {
        ($ fmt : literal $ (, $ ($ arg : tt) +) ?) =>
        {
            $ crate :: console ::
            print(format_args! ($ fmt $ (, $ ($ arg) +) ?)) ;
        }
    }
    #[macro_export]
    macro_rules! println {
        ($ fmt : literal $ (, $ ($ arg : tt) +) ?) =>
        {
            $ crate :: console ::
            print(format_args! (concat! ($ fmt, "\n") $ (, $ ($ arg) +) ?)) ;
        }
    }
}
mod batch {
    use core::cell::RefCell;
    use lazy_static::*;
    use spin::Mutex;
    use crate::trap::TrapContext;
    const USER_STACK_SIZE: usize = 4096;
    const KERNEL_STACK_SIZE: usize = 4096;
    const MAX_APP_NUM: usize = 16;
    const APP_BASE_ADDRESS: usize = 0x80400000;
    const APP_SIZE_LIMIT: usize = 0x20000;
    static KERNEL_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];
    static USER_STACK: [u8; USER_STACK_SIZE] = [0; USER_STACK_SIZE];
    pub fn is_valid_location(loc: usize) -> bool {
        let user_stack_loc = &USER_STACK as *const _ as _;
        (loc >= APP_BASE_ADDRESS && loc < APP_BASE_ADDRESS + APP_SIZE_LIMIT)
            ||
            (loc >= user_stack_loc &&
                 loc < user_stack_loc + core::mem::size_of_val(&USER_STACK))
    }
    struct AppManager {
        num_app: usize,
        current_app: usize,
        app_start: [usize; MAX_APP_NUM + 1],
    }
    impl AppManager {
        unsafe fn load_app(&self, app_id: usize) {
            crate::console::print(::core::fmt::Arguments::new_v1(&["[Kernel] Loading app_",
                                                                   "\n"],
                                                                 &match (&app_id,)
                                                                      {
                                                                      (arg0,)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Display::fmt)],
                                                                  }));
            ;
            llvm_asm!("fence.i":  :  :  : "volatile");
            (APP_BASE_ADDRESS..APP_BASE_ADDRESS +
                                   APP_SIZE_LIMIT).for_each(|addr|
                                                                {
                                                                    (addr as
                                                                         *mut u8).write_volatile(0);
                                                                });
        }
    }
    #[allow(missing_copy_implementations)]
    #[allow(non_camel_case_types)]
    #[allow(dead_code)]
    struct APP_MANAGER {
        __private_field: (),
    }
    #[doc(hidden)]
    static APP_MANAGER: APP_MANAGER = APP_MANAGER{__private_field: (),};
    impl ::lazy_static::__Deref for APP_MANAGER {
        type Target = Mutex<AppManager>;
        fn deref(&self) -> &Mutex<AppManager> {
            #[inline(always)]
            fn __static_ref_initialize() -> Mutex<AppManager> {
                Mutex::new(::core::panicking::panic("not implemented"))
            }
            #[inline(always)]
            fn __stability() -> &'static Mutex<AppManager> {
                static LAZY: ::lazy_static::lazy::Lazy<Mutex<AppManager>> =
                    ::lazy_static::lazy::Lazy::INIT;
                LAZY.get(__static_ref_initialize)
            }
            __stability()
        }
    }
    impl ::lazy_static::LazyStatic for APP_MANAGER {
        fn initialize(lazy: &Self) { let _ = &**lazy; }
    }
}
mod collections {
    pub mod memlist {
        use core::iter;
        use core::mem::size_of;
        #[repr(C)]
        pub struct Node {
            prev: Option<*mut Node>,
            next: Option<*mut Node>,
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for Node {
            fn fmt(&self, f: &mut ::core::fmt::Formatter)
             -> ::core::fmt::Result {
                match *self {
                    Node { prev: ref __self_0_0, next: ref __self_0_1 } => {
                        let debug_trait_builder =
                            &mut ::core::fmt::Formatter::debug_struct(f,
                                                                      "Node");
                        let _ =
                            ::core::fmt::DebugStruct::field(debug_trait_builder,
                                                            "prev",
                                                            &&(*__self_0_0));
                        let _ =
                            ::core::fmt::DebugStruct::field(debug_trait_builder,
                                                            "next",
                                                            &&(*__self_0_1));
                        ::core::fmt::DebugStruct::finish(debug_trait_builder)
                    }
                }
            }
        }
        #[allow(unknown_lints, eq_op)]
        const _:
         [(); 0 -
                  !{
                       const ASSERT: bool =
                           size_of::<Option<*mut Node>>() == 16;
                       ASSERT
                   } as usize] =
            [];
        #[allow(unknown_lints, eq_op)]
        const _:
         [(); 0 -
                  !{
                       const ASSERT: bool = size_of::<Option<Node>>() == 32;
                       ASSERT
                   } as usize] =
            [];
        impl Node {
            unsafe fn isolate(&mut self) {
                if let Some(prev) = self.prev { (*prev).next = self.next; }
                if let Some(next) = self.next { (*next).prev = self.prev; }
            }
        }
        #[repr(transparent)]
        pub struct MemList(Option<*mut Node>);
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for MemList {
            fn fmt(&self, f: &mut ::core::fmt::Formatter)
             -> ::core::fmt::Result {
                match *self {
                    MemList(ref __self_0_0) => {
                        let debug_trait_builder =
                            &mut ::core::fmt::Formatter::debug_tuple(f,
                                                                     "MemList");
                        let _ =
                            ::core::fmt::DebugTuple::field(debug_trait_builder,
                                                           &&(*__self_0_0));
                        ::core::fmt::DebugTuple::finish(debug_trait_builder)
                    }
                }
            }
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::marker::Copy for MemList { }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for MemList {
            #[inline]
            fn clone(&self) -> MemList {
                {
                    let _:
                            ::core::clone::AssertParamIsClone<Option<*mut Node>>;
                    *self
                }
            }
        }
        unsafe impl Send for MemList { }
        impl MemList {
            pub const fn new() -> Self { MemList(None) }
            pub fn is_empty(&self) -> bool { self.0.is_none() }
            pub fn pop(&mut self) -> Option<*mut Node> {
                let ret = self.0;
                if let Some(node) = ret {
                    unsafe { self.remove(&mut *node) };
                }
                ret
            }
            pub unsafe fn push(&mut self, ptr: *mut u8) {
                let node = ptr as *mut Node;
                (*node).prev = None;
                (*node).next = self.0;
                self.0 = Some(node);
            }
            pub unsafe fn remove(&mut self, node: &mut Node) {
                if self.0 == Some(node) { self.0 = node.next; }
                node.isolate();
            }
            pub fn iter_mut(&mut self) -> IterMut { IterMut{curr: self.0,} }
        }
        pub struct IterMut {
            curr: Option<*mut Node>,
        }
        impl iter::Iterator for IterMut {
            type Item = *mut Node;
            fn next(&mut self) -> Option<<Self as Iterator>::Item> {
                match self.curr {
                    None => None,
                    Some(curr) => {
                        self.curr = unsafe { (*curr).next };
                        Some(curr)
                    }
                }
            }
        }
    }
}
mod config {
    use crate::utils::KILOBYTE;
    pub const USER_STACK_SIZE: usize = 8 * KILOBYTE;
    pub const PAGE_SIZE: usize = 4 * KILOBYTE;
    pub const ENTRY_SIZE: usize = 8;
    pub const ENTRIES_PER_PAGE: usize = PAGE_SIZE / ENTRY_SIZE;
    pub const PAGE_SIZE_BITS: usize = 0xc;
    pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;
    pub const TRAP_CONTEXT: usize = TRAMPOLINE - PAGE_SIZE;
    pub const CLOCK_FREQ: usize = 12500000;
    pub struct EntryFlags {
        bits: u64,
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::marker::Copy for EntryFlags { }
    impl ::core::marker::StructuralPartialEq for EntryFlags { }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::cmp::PartialEq for EntryFlags {
        #[inline]
        fn eq(&self, other: &EntryFlags) -> bool {
            match *other {
                EntryFlags { bits: ref __self_1_0 } =>
                match *self {
                    EntryFlags { bits: ref __self_0_0 } =>
                    (*__self_0_0) == (*__self_1_0),
                },
            }
        }
        #[inline]
        fn ne(&self, other: &EntryFlags) -> bool {
            match *other {
                EntryFlags { bits: ref __self_1_0 } =>
                match *self {
                    EntryFlags { bits: ref __self_0_0 } =>
                    (*__self_0_0) != (*__self_1_0),
                },
            }
        }
    }
    impl ::core::marker::StructuralEq for EntryFlags { }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::cmp::Eq for EntryFlags {
        #[inline]
        #[doc(hidden)]
        #[no_coverage]
        fn assert_receiver_is_total_eq(&self) -> () {
            { let _: ::core::cmp::AssertParamIsEq<u64>; }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::clone::Clone for EntryFlags {
        #[inline]
        fn clone(&self) -> EntryFlags {
            { let _: ::core::clone::AssertParamIsClone<u64>; *self }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::cmp::PartialOrd for EntryFlags {
        #[inline]
        fn partial_cmp(&self, other: &EntryFlags)
         -> ::core::option::Option<::core::cmp::Ordering> {
            match *other {
                EntryFlags { bits: ref __self_1_0 } =>
                match *self {
                    EntryFlags { bits: ref __self_0_0 } =>
                    match ::core::cmp::PartialOrd::partial_cmp(&(*__self_0_0),
                                                               &(*__self_1_0))
                        {
                        ::core::option::Option::Some(::core::cmp::Ordering::Equal)
                        =>
                        ::core::option::Option::Some(::core::cmp::Ordering::Equal),
                        cmp => cmp,
                    },
                },
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::cmp::Ord for EntryFlags {
        #[inline]
        fn cmp(&self, other: &EntryFlags) -> ::core::cmp::Ordering {
            match *other {
                EntryFlags { bits: ref __self_1_0 } =>
                match *self {
                    EntryFlags { bits: ref __self_0_0 } =>
                    match ::core::cmp::Ord::cmp(&(*__self_0_0),
                                                &(*__self_1_0)) {
                        ::core::cmp::Ordering::Equal =>
                        ::core::cmp::Ordering::Equal,
                        cmp => cmp,
                    },
                },
            }
        }
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::hash::Hash for EntryFlags {
        fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) -> () {
            match *self {
                EntryFlags { bits: ref __self_0_0 } => {
                    ::core::hash::Hash::hash(&(*__self_0_0), state)
                }
            }
        }
    }
    impl ::bitflags::_core::fmt::Debug for EntryFlags {
        fn fmt(&self, f: &mut ::bitflags::_core::fmt::Formatter)
         -> ::bitflags::_core::fmt::Result {
            #[allow(non_snake_case)]
            trait __BitFlags {
                #[inline]
                fn Valid(&self) -> bool { false }
                #[inline]
                fn Read(&self) -> bool { false }
                #[inline]
                fn Write(&self) -> bool { false }
                #[inline]
                fn Execute(&self) -> bool { false }
                #[inline]
                fn User(&self) -> bool { false }
                #[inline]
                fn Global(&self) -> bool { false }
                #[inline]
                fn Access(&self) -> bool { false }
                #[inline]
                fn Dirty(&self) -> bool { false }
                #[inline]
                fn ReadWrite(&self) -> bool { false }
                #[inline]
                fn ReadExecute(&self) -> bool { false }
                #[inline]
                fn ReadWriteExecute(&self) -> bool { false }
                #[inline]
                fn UserReadWrite(&self) -> bool { false }
                #[inline]
                fn UserReadExecute(&self) -> bool { false }
                #[inline]
                fn UserReadWriteExecute(&self) -> bool { false }
            }
            impl __BitFlags for EntryFlags {
                #[allow(deprecated)]
                #[inline]
                fn Valid(&self) -> bool {
                    if Self::Valid.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::Valid.bits == Self::Valid.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn Read(&self) -> bool {
                    if Self::Read.bits == 0 && self.bits != 0 {
                        false
                    } else { self.bits & Self::Read.bits == Self::Read.bits }
                }
                #[allow(deprecated)]
                #[inline]
                fn Write(&self) -> bool {
                    if Self::Write.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::Write.bits == Self::Write.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn Execute(&self) -> bool {
                    if Self::Execute.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::Execute.bits == Self::Execute.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn User(&self) -> bool {
                    if Self::User.bits == 0 && self.bits != 0 {
                        false
                    } else { self.bits & Self::User.bits == Self::User.bits }
                }
                #[allow(deprecated)]
                #[inline]
                fn Global(&self) -> bool {
                    if Self::Global.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::Global.bits == Self::Global.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn Access(&self) -> bool {
                    if Self::Access.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::Access.bits == Self::Access.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn Dirty(&self) -> bool {
                    if Self::Dirty.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::Dirty.bits == Self::Dirty.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn ReadWrite(&self) -> bool {
                    if Self::ReadWrite.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::ReadWrite.bits ==
                            Self::ReadWrite.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn ReadExecute(&self) -> bool {
                    if Self::ReadExecute.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::ReadExecute.bits ==
                            Self::ReadExecute.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn ReadWriteExecute(&self) -> bool {
                    if Self::ReadWriteExecute.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::ReadWriteExecute.bits ==
                            Self::ReadWriteExecute.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn UserReadWrite(&self) -> bool {
                    if Self::UserReadWrite.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::UserReadWrite.bits ==
                            Self::UserReadWrite.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn UserReadExecute(&self) -> bool {
                    if Self::UserReadExecute.bits == 0 && self.bits != 0 {
                        false
                    } else {
                        self.bits & Self::UserReadExecute.bits ==
                            Self::UserReadExecute.bits
                    }
                }
                #[allow(deprecated)]
                #[inline]
                fn UserReadWriteExecute(&self) -> bool {
                    if Self::UserReadWriteExecute.bits == 0 && self.bits != 0
                       {
                        false
                    } else {
                        self.bits & Self::UserReadWriteExecute.bits ==
                            Self::UserReadWriteExecute.bits
                    }
                }
            }
            let mut first = true;
            if <EntryFlags as __BitFlags>::Valid(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Valid")?;
            }
            if <EntryFlags as __BitFlags>::Read(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Read")?;
            }
            if <EntryFlags as __BitFlags>::Write(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Write")?;
            }
            if <EntryFlags as __BitFlags>::Execute(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Execute")?;
            }
            if <EntryFlags as __BitFlags>::User(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("User")?;
            }
            if <EntryFlags as __BitFlags>::Global(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Global")?;
            }
            if <EntryFlags as __BitFlags>::Access(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Access")?;
            }
            if <EntryFlags as __BitFlags>::Dirty(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("Dirty")?;
            }
            if <EntryFlags as __BitFlags>::ReadWrite(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("ReadWrite")?;
            }
            if <EntryFlags as __BitFlags>::ReadExecute(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("ReadExecute")?;
            }
            if <EntryFlags as __BitFlags>::ReadWriteExecute(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("ReadWriteExecute")?;
            }
            if <EntryFlags as __BitFlags>::UserReadWrite(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("UserReadWrite")?;
            }
            if <EntryFlags as __BitFlags>::UserReadExecute(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("UserReadExecute")?;
            }
            if <EntryFlags as __BitFlags>::UserReadWriteExecute(self) {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("UserReadWriteExecute")?;
            }
            let extra_bits = self.bits & !EntryFlags::all().bits();
            if extra_bits != 0 {
                if !first { f.write_str(" | ")?; }
                first = false;
                f.write_str("0x")?;
                ::bitflags::_core::fmt::LowerHex::fmt(&extra_bits, f)?;
            }
            if first { f.write_str("(empty)")?; }
            Ok(())
        }
    }
    impl ::bitflags::_core::fmt::Binary for EntryFlags {
        fn fmt(&self, f: &mut ::bitflags::_core::fmt::Formatter)
         -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::Binary::fmt(&self.bits, f)
        }
    }
    impl ::bitflags::_core::fmt::Octal for EntryFlags {
        fn fmt(&self, f: &mut ::bitflags::_core::fmt::Formatter)
         -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::Octal::fmt(&self.bits, f)
        }
    }
    impl ::bitflags::_core::fmt::LowerHex for EntryFlags {
        fn fmt(&self, f: &mut ::bitflags::_core::fmt::Formatter)
         -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::LowerHex::fmt(&self.bits, f)
        }
    }
    impl ::bitflags::_core::fmt::UpperHex for EntryFlags {
        fn fmt(&self, f: &mut ::bitflags::_core::fmt::Formatter)
         -> ::bitflags::_core::fmt::Result {
            ::bitflags::_core::fmt::UpperHex::fmt(&self.bits, f)
        }
    }
    #[allow(dead_code)]
    impl EntryFlags {
        pub const Valid: EntryFlags = EntryFlags{bits: 1 << 0,};
        pub const Read: EntryFlags = EntryFlags{bits: 1 << 1,};
        pub const Write: EntryFlags = EntryFlags{bits: 1 << 2,};
        pub const Execute: EntryFlags = EntryFlags{bits: 1 << 3,};
        pub const User: EntryFlags = EntryFlags{bits: 1 << 4,};
        pub const Global: EntryFlags = EntryFlags{bits: 1 << 5,};
        pub const Access: EntryFlags = EntryFlags{bits: 1 << 6,};
        pub const Dirty: EntryFlags = EntryFlags{bits: 1 << 7,};
        pub const ReadWrite: EntryFlags =
            EntryFlags{bits: Self::Read.bits | Self::Write.bits,};
        pub const ReadExecute: EntryFlags =
            EntryFlags{bits: Self::Read.bits | Self::Execute.bits,};
        pub const ReadWriteExecute: EntryFlags =
            EntryFlags{bits:
                           Self::Read.bits | Self::Write.bits |
                               Self::Execute.bits,};
        pub const UserReadWrite: EntryFlags =
            EntryFlags{bits: Self::ReadWrite.bits | Self::User.bits,};
        pub const UserReadExecute: EntryFlags =
            EntryFlags{bits: Self::ReadExecute.bits | Self::User.bits,};
        pub const UserReadWriteExecute: EntryFlags =
            EntryFlags{bits:
                           Self::UserReadWriteExecute.bits |
                               Self::User.bits,};
        #[doc = r" Returns an empty set of flags"]
        #[inline]
        pub const fn empty() -> EntryFlags { EntryFlags{bits: 0,} }
        #[doc = r" Returns the set containing all flags."]
        #[inline]
        pub const fn all() -> EntryFlags {
            #[allow(non_snake_case)]
            trait __BitFlags {
                const Valid: u64 = 0;
                const Read: u64 = 0;
                const Write: u64 = 0;
                const Execute: u64 = 0;
                const User: u64 = 0;
                const Global: u64 = 0;
                const Access: u64 = 0;
                const Dirty: u64 = 0;
                const ReadWrite: u64 = 0;
                const ReadExecute: u64 = 0;
                const ReadWriteExecute: u64 = 0;
                const UserReadWrite: u64 = 0;
                const UserReadExecute: u64 = 0;
                const UserReadWriteExecute: u64 = 0;
            }
            impl __BitFlags for EntryFlags {
                #[allow(deprecated)]
                const Valid: u64 = Self::Valid.bits;
                #[allow(deprecated)]
                const Read: u64 = Self::Read.bits;
                #[allow(deprecated)]
                const Write: u64 = Self::Write.bits;
                #[allow(deprecated)]
                const Execute: u64 = Self::Execute.bits;
                #[allow(deprecated)]
                const User: u64 = Self::User.bits;
                #[allow(deprecated)]
                const Global: u64 = Self::Global.bits;
                #[allow(deprecated)]
                const Access: u64 = Self::Access.bits;
                #[allow(deprecated)]
                const Dirty: u64 = Self::Dirty.bits;
                #[allow(deprecated)]
                const ReadWrite: u64 = Self::ReadWrite.bits;
                #[allow(deprecated)]
                const ReadExecute: u64 = Self::ReadExecute.bits;
                #[allow(deprecated)]
                const ReadWriteExecute: u64 = Self::ReadWriteExecute.bits;
                #[allow(deprecated)]
                const UserReadWrite: u64 = Self::UserReadWrite.bits;
                #[allow(deprecated)]
                const UserReadExecute: u64 = Self::UserReadExecute.bits;
                #[allow(deprecated)]
                const UserReadWriteExecute: u64 =
                    Self::UserReadWriteExecute.bits;
            }
            EntryFlags{bits:
                           <EntryFlags as __BitFlags>::Valid |
                               <EntryFlags as __BitFlags>::Read |
                               <EntryFlags as __BitFlags>::Write |
                               <EntryFlags as __BitFlags>::Execute |
                               <EntryFlags as __BitFlags>::User |
                               <EntryFlags as __BitFlags>::Global |
                               <EntryFlags as __BitFlags>::Access |
                               <EntryFlags as __BitFlags>::Dirty |
                               <EntryFlags as __BitFlags>::ReadWrite |
                               <EntryFlags as __BitFlags>::ReadExecute |
                               <EntryFlags as __BitFlags>::ReadWriteExecute |
                               <EntryFlags as __BitFlags>::UserReadWrite |
                               <EntryFlags as __BitFlags>::UserReadExecute |
                               <EntryFlags as
                                   __BitFlags>::UserReadWriteExecute,}
        }
        #[doc = r" Returns the raw value of the flags currently stored."]
        #[inline]
        pub const fn bits(&self) -> u64 { self.bits }
        /// Convert from underlying bit representation, unless that
        /// representation contains bits that do not correspond to a flag.
        #[inline]
        pub fn from_bits(bits: u64)
         -> ::bitflags::_core::option::Option<EntryFlags> {
            if (bits & !EntryFlags::all().bits()) == 0 {
                ::bitflags::_core::option::Option::Some(EntryFlags{bits,})
            } else { ::bitflags::_core::option::Option::None }
        }
        #[doc =
          r" Convert from underlying bit representation, dropping any bits"]
        #[doc = r" that do not correspond to flags."]
        #[inline]
        pub const fn from_bits_truncate(bits: u64) -> EntryFlags {
            EntryFlags{bits: bits & EntryFlags::all().bits,}
        }
        #[doc =
          r" Convert from underlying bit representation, preserving all"]
        #[doc = r" bits (even those not corresponding to a defined flag)."]
        #[inline]
        pub const unsafe fn from_bits_unchecked(bits: u64) -> EntryFlags {
            EntryFlags{bits,}
        }
        #[doc = r" Returns `true` if no flags are currently stored."]
        #[inline]
        pub const fn is_empty(&self) -> bool {
            self.bits() == EntryFlags::empty().bits()
        }
        #[doc = r" Returns `true` if all flags are currently set."]
        #[inline]
        pub const fn is_all(&self) -> bool {
            self.bits == EntryFlags::all().bits
        }
        #[doc =
          r" Returns `true` if there are flags common to both `self` and `other`."]
        #[inline]
        pub const fn intersects(&self, other: EntryFlags) -> bool {
            !EntryFlags{bits: self.bits & other.bits,}.is_empty()
        }
        #[doc =
          r" Returns `true` all of the flags in `other` are contained within `self`."]
        #[inline]
        pub const fn contains(&self, other: EntryFlags) -> bool {
            (self.bits & other.bits) == other.bits
        }
        /// Inserts the specified flags in-place.
        #[inline]
        pub fn insert(&mut self, other: EntryFlags) {
            self.bits |= other.bits;
        }
        /// Removes the specified flags in-place.
        #[inline]
        pub fn remove(&mut self, other: EntryFlags) {
            self.bits &= !other.bits;
        }
        /// Toggles the specified flags in-place.
        #[inline]
        pub fn toggle(&mut self, other: EntryFlags) {
            self.bits ^= other.bits;
        }
        /// Inserts or removes the specified flags depending on the passed value.
        #[inline]
        pub fn set(&mut self, other: EntryFlags, value: bool) {
            if value { self.insert(other); } else { self.remove(other); }
        }
    }
    impl ::bitflags::_core::ops::BitOr for EntryFlags {
        type Output = EntryFlags;
        /// Returns the union of the two sets of flags.
        #[inline]
        fn bitor(self, other: EntryFlags) -> EntryFlags {
            EntryFlags{bits: self.bits | other.bits,}
        }
    }
    impl ::bitflags::_core::ops::BitOrAssign for EntryFlags {
        /// Adds the set of flags.
        #[inline]
        fn bitor_assign(&mut self, other: EntryFlags) {
            self.bits |= other.bits;
        }
    }
    impl ::bitflags::_core::ops::BitXor for EntryFlags {
        type Output = EntryFlags;
        /// Returns the left flags, but with all the right flags toggled.
        #[inline]
        fn bitxor(self, other: EntryFlags) -> EntryFlags {
            EntryFlags{bits: self.bits ^ other.bits,}
        }
    }
    impl ::bitflags::_core::ops::BitXorAssign for EntryFlags {
        /// Toggles the set of flags.
        #[inline]
        fn bitxor_assign(&mut self, other: EntryFlags) {
            self.bits ^= other.bits;
        }
    }
    impl ::bitflags::_core::ops::BitAnd for EntryFlags {
        type Output = EntryFlags;
        /// Returns the intersection between the two sets of flags.
        #[inline]
        fn bitand(self, other: EntryFlags) -> EntryFlags {
            EntryFlags{bits: self.bits & other.bits,}
        }
    }
    impl ::bitflags::_core::ops::BitAndAssign for EntryFlags {
        /// Disables all flags disabled in the set.
        #[inline]
        fn bitand_assign(&mut self, other: EntryFlags) {
            self.bits &= other.bits;
        }
    }
    impl ::bitflags::_core::ops::Sub for EntryFlags {
        type Output = EntryFlags;
        /// Returns the set difference of the two sets of flags.
        #[inline]
        fn sub(self, other: EntryFlags) -> EntryFlags {
            EntryFlags{bits: self.bits & !other.bits,}
        }
    }
    impl ::bitflags::_core::ops::SubAssign for EntryFlags {
        /// Disables all flags enabled in the set.
        #[inline]
        fn sub_assign(&mut self, other: EntryFlags) {
            self.bits &= !other.bits;
        }
    }
    impl ::bitflags::_core::ops::Not for EntryFlags {
        type Output = EntryFlags;
        /// Returns the complement of this set of flags.
        #[inline]
        fn not(self) -> EntryFlags {
            EntryFlags{bits: !self.bits,} & EntryFlags::all()
        }
    }
    impl ::bitflags::_core::iter::Extend<EntryFlags> for EntryFlags {
        fn extend<T: ::bitflags::_core::iter::IntoIterator<Item =
                                                           EntryFlags>>(&mut self,
                                                                        iterator:
                                                                            T) {
            for item in iterator { self.insert(item) }
        }
    }
    impl ::bitflags::_core::iter::FromIterator<EntryFlags> for EntryFlags {
        fn from_iter<T: ::bitflags::_core::iter::IntoIterator<Item =
                                                              EntryFlags>>(iterator:
                                                                               T)
         -> EntryFlags {
            let mut result = Self::empty();
            result.extend(iterator);
            result
        }
    }
}
mod cpu {
    pub struct CPU {
        pub hart_id: usize,
    }
}
mod lang_items {
    use crate::sbi::shutdown;
    use core::panic::PanicInfo;
    #[panic_handler]
    fn panic(info: &PanicInfo) -> ! {
        if let Some(location) = info.location() {
            crate::console::print(::core::fmt::Arguments::new_v1(&["Panicked at ",
                                                                   ":", " ",
                                                                   "\n"],
                                                                 &match (&location.file(),
                                                                         &location.line(),
                                                                         &info.message().unwrap())
                                                                      {
                                                                      (arg0,
                                                                       arg1,
                                                                       arg2)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Display::fmt),
                                                                       ::core::fmt::ArgumentV1::new(arg1,
                                                                                                    ::core::fmt::Display::fmt),
                                                                       ::core::fmt::ArgumentV1::new(arg2,
                                                                                                    ::core::fmt::Display::fmt)],
                                                                  }));
            ;
        } else {
            crate::console::print(::core::fmt::Arguments::new_v1(&["Panicked: ",
                                                                   "\n"],
                                                                 &match (&info.message().unwrap(),)
                                                                      {
                                                                      (arg0,)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Display::fmt)],
                                                                  }));
            ;
        }
        shutdown()
    }
}
mod logger {
    use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
    pub struct ColorLogger;
    impl Log for ColorLogger {
        fn enabled(&self, metadata: &Metadata) -> bool {
            metadata.level() <= Level::Info
        }
        fn log(&self, record: &Record) {
            if !self.enabled(record.metadata()) { return; }
            match record.metadata().level() {
                Level::Trace =>
                crate::console::print(::core::fmt::Arguments::new_v1(&["\u{1b}[90m",
                                                                       "\u{1b}[0m\n"],
                                                                     &match (&record.args(),)
                                                                          {
                                                                          (arg0,)
                                                                          =>
                                                                          [::core::fmt::ArgumentV1::new(arg0,
                                                                                                        ::core::fmt::Display::fmt)],
                                                                      })),
                Level::Debug =>
                crate::console::print(::core::fmt::Arguments::new_v1(&["\u{1b}[32m",
                                                                       "\u{1b}[0m\n"],
                                                                     &match (&record.args(),)
                                                                          {
                                                                          (arg0,)
                                                                          =>
                                                                          [::core::fmt::ArgumentV1::new(arg0,
                                                                                                        ::core::fmt::Display::fmt)],
                                                                      })),
                Level::Info =>
                crate::console::print(::core::fmt::Arguments::new_v1(&["\u{1b}[34m",
                                                                       "\u{1b}[0m\n"],
                                                                     &match (&record.args(),)
                                                                          {
                                                                          (arg0,)
                                                                          =>
                                                                          [::core::fmt::ArgumentV1::new(arg0,
                                                                                                        ::core::fmt::Display::fmt)],
                                                                      })),
                Level::Warn =>
                crate::console::print(::core::fmt::Arguments::new_v1(&["\u{1b}[93m",
                                                                       "\u{1b}[0m\n"],
                                                                     &match (&record.args(),)
                                                                          {
                                                                          (arg0,)
                                                                          =>
                                                                          [::core::fmt::ArgumentV1::new(arg0,
                                                                                                        ::core::fmt::Display::fmt)],
                                                                      })),
                Level::Error =>
                crate::console::print(::core::fmt::Arguments::new_v1(&["\u{1b}[31m",
                                                                       "\u{1b}[0m\n"],
                                                                     &match (&record.args(),)
                                                                          {
                                                                          (arg0,)
                                                                          =>
                                                                          [::core::fmt::ArgumentV1::new(arg0,
                                                                                                        ::core::fmt::Display::fmt)],
                                                                      })),
            };
        }
        fn flush(&self) { }
    }
    static LOGGER: ColorLogger = ColorLogger;
    pub fn init() {
        let level =
            match ::core::option::Option::None::<&'static str> {
                Some("error") => LevelFilter::Error,
                Some("warn") => LevelFilter::Warn,
                Some("info") => LevelFilter::Info,
                Some("debug") => LevelFilter::Debug,
                Some("trace") => LevelFilter::Trace,
                _ => LevelFilter::Off,
            };
        log::set_logger(&LOGGER).map(|()| log::set_max_level(level)).unwrap();
    }
}
mod memory {
    use core::alloc::{GlobalAlloc, Layout};
    use lazy_static::lazy_static;
    use log::debug;
    use spin::Mutex;
    mod allocator {
        use crate::collections::memlist::{MemList, Node};
        use crate::utils::{align_down, align_up};
        use core::alloc::Layout;
        use core::fmt;
        use core::mem::size_of;
        const MIN_ALLOCATION_SIZE_ORDER: usize =
            size_of::<Node>().next_power_of_two().trailing_zeros() as usize;
        const MAX_ALLOCATION_SIZE_ORDER: usize = 31;
        const BUCKET_COUNT: usize = MAX_ALLOCATION_SIZE_ORDER + 1;
        pub struct Allocator {
            total: usize,
            allocated: usize,
            blocks: [MemList; BUCKET_COUNT],
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::fmt::Debug for Allocator {
            fn fmt(&self, f: &mut ::core::fmt::Formatter)
             -> ::core::fmt::Result {
                match *self {
                    Allocator {
                    total: ref __self_0_0,
                    allocated: ref __self_0_1,
                    blocks: ref __self_0_2 } => {
                        let debug_trait_builder =
                            &mut ::core::fmt::Formatter::debug_struct(f,
                                                                      "Allocator");
                        let _ =
                            ::core::fmt::DebugStruct::field(debug_trait_builder,
                                                            "total",
                                                            &&(*__self_0_0));
                        let _ =
                            ::core::fmt::DebugStruct::field(debug_trait_builder,
                                                            "allocated",
                                                            &&(*__self_0_1));
                        let _ =
                            ::core::fmt::DebugStruct::field(debug_trait_builder,
                                                            "blocks",
                                                            &&(*__self_0_2));
                        ::core::fmt::DebugStruct::finish(debug_trait_builder)
                    }
                }
            }
        }
        unsafe impl Send for Allocator { }
        impl Allocator {
            pub unsafe fn new(start: usize, end: usize) -> Self {
                let mut ret =
                    Allocator{blocks: [MemList::new(); BUCKET_COUNT],
                              total: 0,
                              allocated: 0,};
                ret.add_to_heap(start, end);
                ret
            }
            pub unsafe fn add_to_heap(&mut self, mut start: usize,
                                      mut end: usize) {
                start = align_up(start, size_of::<MemList>());
                end = align_down(end, size_of::<MemList>());
                crate::console::print(::core::fmt::Arguments::new_v1_formatted(&["[add_to_heap] add [",
                                                                                 ", ",
                                                                                 ") to allocator\n"],
                                                                               &match (&start,
                                                                                       &end)
                                                                                    {
                                                                                    (arg0,
                                                                                     arg1)
                                                                                    =>
                                                                                    [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                  ::core::fmt::LowerHex::fmt),
                                                                                     ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                  ::core::fmt::LowerHex::fmt)],
                                                                                },
                                                                               &[::core::fmt::rt::v1::Argument{position:
                                                                                                                   0usize,
                                                                                                               format:
                                                                                                                   ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                       ' ',
                                                                                                                                                   align:
                                                                                                                                                       ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                   flags:
                                                                                                                                                       4u32,
                                                                                                                                                   precision:
                                                                                                                                                       ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                   width:
                                                                                                                                                       ::core::fmt::rt::v1::Count::Implied,},},
                                                                                 ::core::fmt::rt::v1::Argument{position:
                                                                                                                   1usize,
                                                                                                               format:
                                                                                                                   ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                       ' ',
                                                                                                                                                   align:
                                                                                                                                                       ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                   flags:
                                                                                                                                                       4u32,
                                                                                                                                                   precision:
                                                                                                                                                       ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                   width:
                                                                                                                                                       ::core::fmt::rt::v1::Count::Implied,},}]));
                ;
                if !(start <= end) {
                    ::core::panicking::panic("assertion failed: start <= end")
                };
                let mut remaining: usize = end - start;
                let mut curr_start = start;
                for order in
                    (MIN_ALLOCATION_SIZE_ORDER..=MAX_ALLOCATION_SIZE_ORDER).rev()
                    {
                    let size = 2_usize.pow(order as u32);
                    let count = remaining / size;
                    remaining = remaining % size;
                    for _ in 0..count {
                        self.insert_block(curr_start as *mut u8, order);
                        self.total += size;
                        curr_start += size;
                    }
                }
            }
            pub unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
                let size =
                    core::cmp::max(layout.size().next_power_of_two(),
                                   core::cmp::max(layout.align(),
                                                  size_of::<MemList>()));
                let target_order = size.trailing_zeros() as usize;
                let mut os =
                    (target_order..=MAX_ALLOCATION_SIZE_ORDER).filter(|&order|
                                                                          !self.blocks[order].is_empty());
                match os.next() {
                    None => 0 as *mut u8,
                    Some(first_enough) => {
                        let block = self.blocks[first_enough].pop().unwrap();
                        let mut curr_start = block as usize;
                        for order in target_order..first_enough {
                            let size = 2_usize.pow(order as u32);
                            self.blocks[order].push(curr_start as *mut u8);
                            curr_start += size;
                        }
                        self.allocated += 2_usize.pow(target_order as u32);
                        curr_start as *mut u8
                    }
                }
            }
            unsafe fn insert_block(&mut self, ptr: *mut u8, order: usize) {
                let neighbor =
                    self.blocks[order].iter_mut().filter(|node|
                                                             {
                                                                 *node as
                                                                     usize ==
                                                                     ptr as
                                                                         usize
                                                                         +
                                                                         2_usize.pow(order
                                                                                         as
                                                                                         u32)
                                                                     ||
                                                                     ptr as
                                                                         usize
                                                                         ==
                                                                         *node
                                                                             as
                                                                             usize
                                                                             +
                                                                             2_usize.pow(order
                                                                                             as
                                                                                             u32)
                                                             }).next();
                match neighbor {
                    None => self.blocks[order].push(ptr),
                    Some(node) => {
                        self.blocks[order].remove(&mut *node);
                        self.insert_block(core::cmp::min(ptr,
                                                         node as *mut u8),
                                          order + 1);
                    }
                }
            }
            pub unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
                let size =
                    core::cmp::max(layout.size().next_power_of_two(),
                                   core::cmp::max(layout.align(),
                                                  size_of::<MemList>()));
                let target_order = size.trailing_zeros() as usize;
                self.insert_block(ptr, target_order);
                self.allocated -= size;
            }
        }
    }
    pub mod layout {
        extern "C" {
            static _memory_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct MEMORY_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static MEMORY_START: MEMORY_START =
            MEMORY_START{__private_field: (),};
        impl ::lazy_static::__Deref for MEMORY_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_memory_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for MEMORY_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _memory_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct MEMORY_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static MEMORY_END: MEMORY_END = MEMORY_END{__private_field: (),};
        impl ::lazy_static::__Deref for MEMORY_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_memory_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for MEMORY_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _text_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct TEXT_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static TEXT_START: TEXT_START = TEXT_START{__private_field: (),};
        impl ::lazy_static::__Deref for TEXT_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_text_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for TEXT_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _text_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct TEXT_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static TEXT_END: TEXT_END = TEXT_END{__private_field: (),};
        impl ::lazy_static::__Deref for TEXT_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_text_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for TEXT_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _rodata_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct RODATA_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static RODATA_START: RODATA_START =
            RODATA_START{__private_field: (),};
        impl ::lazy_static::__Deref for RODATA_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_rodata_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for RODATA_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _rodata_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct RODATA_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static RODATA_END: RODATA_END = RODATA_END{__private_field: (),};
        impl ::lazy_static::__Deref for RODATA_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_rodata_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for RODATA_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _data_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct DATA_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static DATA_START: DATA_START = DATA_START{__private_field: (),};
        impl ::lazy_static::__Deref for DATA_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_data_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for DATA_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _data_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct DATA_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static DATA_END: DATA_END = DATA_END{__private_field: (),};
        impl ::lazy_static::__Deref for DATA_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_data_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for DATA_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _bss_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct BSS_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static BSS_START: BSS_START = BSS_START{__private_field: (),};
        impl ::lazy_static::__Deref for BSS_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_bss_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for BSS_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _bss_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct BSS_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static BSS_END: BSS_END = BSS_END{__private_field: (),};
        impl ::lazy_static::__Deref for BSS_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_bss_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for BSS_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _kernel_stack_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct KERNEL_STACK_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static KERNEL_STACK_START: KERNEL_STACK_START =
            KERNEL_STACK_START{__private_field: (),};
        impl ::lazy_static::__Deref for KERNEL_STACK_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_kernel_stack_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for KERNEL_STACK_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _kernel_stack_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct KERNEL_STACK_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static KERNEL_STACK_END: KERNEL_STACK_END =
            KERNEL_STACK_END{__private_field: (),};
        impl ::lazy_static::__Deref for KERNEL_STACK_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_kernel_stack_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for KERNEL_STACK_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _heap_start: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct HEAP_START {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static HEAP_START: HEAP_START = HEAP_START{__private_field: (),};
        impl ::lazy_static::__Deref for HEAP_START {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_heap_start as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for HEAP_START {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        extern "C" {
            static _heap_end: usize ;
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct HEAP_END {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static HEAP_END: HEAP_END = HEAP_END{__private_field: (),};
        impl ::lazy_static::__Deref for HEAP_END {
            type Target = usize;
            fn deref(&self) -> &usize {
                #[inline(always)]
                fn __static_ref_initialize() -> usize {
                    unsafe { &_heap_end as *const _ as _ }
                }
                #[inline(always)]
                fn __stability() -> &'static usize {
                    static LAZY: ::lazy_static::lazy::Lazy<usize> =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for HEAP_END {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
    }
    mod paging {
        pub mod physical_addr {
            use crate::utils::{extract_value, set_range};
            use crate::{print, println};
            use alloc::format;
            use bitflags::bitflags;
            use core::fmt;
            use core::ptr;
            use crate::config::PAGE_SIZE;
            #[repr(transparent)]
            pub struct PhysicalAddr(usize);
            impl ::core::marker::StructuralPartialEq for PhysicalAddr { }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::PartialEq for PhysicalAddr {
                #[inline]
                fn eq(&self, other: &PhysicalAddr) -> bool {
                    match *other {
                        PhysicalAddr(ref __self_1_0) =>
                        match *self {
                            PhysicalAddr(ref __self_0_0) =>
                            (*__self_0_0) == (*__self_1_0),
                        },
                    }
                }
                #[inline]
                fn ne(&self, other: &PhysicalAddr) -> bool {
                    match *other {
                        PhysicalAddr(ref __self_1_0) =>
                        match *self {
                            PhysicalAddr(ref __self_0_0) =>
                            (*__self_0_0) != (*__self_1_0),
                        },
                    }
                }
            }
            impl ::core::marker::StructuralEq for PhysicalAddr { }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::Eq for PhysicalAddr {
                #[inline]
                #[doc(hidden)]
                #[no_coverage]
                fn assert_receiver_is_total_eq(&self) -> () {
                    { let _: ::core::cmp::AssertParamIsEq<usize>; }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::PartialOrd for PhysicalAddr {
                #[inline]
                fn partial_cmp(&self, other: &PhysicalAddr)
                 -> ::core::option::Option<::core::cmp::Ordering> {
                    match *other {
                        PhysicalAddr(ref __self_1_0) =>
                        match *self {
                            PhysicalAddr(ref __self_0_0) =>
                            match ::core::cmp::PartialOrd::partial_cmp(&(*__self_0_0),
                                                                       &(*__self_1_0))
                                {
                                ::core::option::Option::Some(::core::cmp::Ordering::Equal)
                                =>
                                ::core::option::Option::Some(::core::cmp::Ordering::Equal),
                                cmp => cmp,
                            },
                        },
                    }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::Ord for PhysicalAddr {
                #[inline]
                fn cmp(&self, other: &PhysicalAddr) -> ::core::cmp::Ordering {
                    match *other {
                        PhysicalAddr(ref __self_1_0) =>
                        match *self {
                            PhysicalAddr(ref __self_0_0) =>
                            match ::core::cmp::Ord::cmp(&(*__self_0_0),
                                                        &(*__self_1_0)) {
                                ::core::cmp::Ordering::Equal =>
                                ::core::cmp::Ordering::Equal,
                                cmp => cmp,
                            },
                        },
                    }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::clone::Clone for PhysicalAddr {
                #[inline]
                fn clone(&self) -> PhysicalAddr {
                    { let _: ::core::clone::AssertParamIsClone<usize>; *self }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::marker::Copy for PhysicalAddr { }
            impl fmt::Debug for PhysicalAddr {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_fmt(::core::fmt::Arguments::new_v1_formatted(&["PhysicalAddr(",
                                                                           ", ppn: ",
                                                                           ", offset: ",
                                                                           ")"],
                                                                         &match (&self.0,
                                                                                 &self.extract_ppn_all(),
                                                                                 &self.extract_offset())
                                                                              {
                                                                              (arg0,
                                                                               arg1,
                                                                               arg2)
                                                                              =>
                                                                              [::core::fmt::ArgumentV1::new(arg0,
                                                                                                            ::core::fmt::LowerHex::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg1,
                                                                                                            ::core::fmt::LowerHex::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg2,
                                                                                                            ::core::fmt::LowerHex::fmt)],
                                                                          },
                                                                         &[::core::fmt::rt::v1::Argument{position:
                                                                                                             0usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 4u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             1usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 4u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             2usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 4u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},}]))
                }
            }
            impl PhysicalAddr {
                pub const fn new(paddr: usize) -> Self { PhysicalAddr(paddr) }
                pub fn from(ppn: usize, offset: usize) -> Self {
                    let mut bits = set_range(0, ppn, 12, 56);
                    bits = set_range(bits, offset, 0, 12);
                    PhysicalAddr(bits)
                }
                pub const fn as_ptr<T>(&self) -> *const T {
                    self.0 as *const T
                }
                pub const fn as_mut_ptr<T>(&self) -> *mut T {
                    self.0 as *mut T
                }
                pub const fn extract_bits(&self) -> usize { self.0 }
                pub const fn extract_ppn(&self, idx: usize) -> usize {
                    match idx {
                        0 => extract_value(self.0, (1 << 9) - 1, 12),
                        1 => extract_value(self.0, (1 << 9) - 1, 21),
                        2 => extract_value(self.0, (1 << 26) - 1, 30),
                        _ =>
                        ::core::panicking::panic("[paddr.extract_ppn] idx should be one of 0..=2"),
                    }
                }
                pub const fn extract_ppn_all(&self) -> usize {
                    extract_value(self.0, (1 << 44) - 1, 12)
                }
                pub const fn extract_offset(&self) -> usize {
                    extract_value(self.0, (1 << 12) - 1, 0)
                }
                pub const fn is_aligned(&self, alignment: usize) -> bool {
                    match self {
                        PhysicalAddr(addr) => (*addr) % alignment == 0,
                    }
                }
            }
        }
        mod table {
            use alloc::alloc::{alloc_zeroed, dealloc, Layout};
            use core::fmt;
            use core::mem::size_of;
            use core::ptr;
            use crate::config::{ENTRIES_PER_PAGE, ENTRY_SIZE, PAGE_SIZE,
                                PAGE_SIZE_BITS};
            use crate::utils::{extract_value, set_range};
            pub use super::physical_addr::*;
            pub use super::virtual_addr::*;
            pub const VALID: usize = 1 << 0;
            pub const READ: usize = 1 << 1;
            pub const WRITE: usize = 1 << 2;
            pub const EXECUTE: usize = 1 << 3;
            pub const USER: usize = 1 << 4;
            pub const GLOBAL: usize = 1 << 5;
            pub const ACCESS: usize = 1 << 6;
            pub const DIRTY: usize = 1 << 7;
            pub const READ_WRITE: usize = READ | WRITE;
            pub const READ_EXECUTE: usize = READ | EXECUTE;
            pub const READ_WRITE_EXECUTE: usize = READ | WRITE | EXECUTE;
            pub const USER_READ_WRITE: usize = READ_WRITE | USER;
            pub const USER_READ_EXECUTE: usize = READ_EXECUTE | USER;
            pub const USER_READ_WRITE_EXECUTE: usize =
                READ_WRITE_EXECUTE | USER;
            #[repr(transparent)]
            pub struct Entry(usize);
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::marker::Copy for Entry { }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::clone::Clone for Entry {
                #[inline]
                fn clone(&self) -> Entry {
                    { let _: ::core::clone::AssertParamIsClone<usize>; *self }
                }
            }
            #[allow(unknown_lints, eq_op)]
            const _:
             [(); 0 -
                      !{
                           const ASSERT: bool =
                               size_of::<Entry>() == ENTRY_SIZE;
                           ASSERT
                       } as usize] =
                [];
            impl fmt::Debug for Entry {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_fmt(::core::fmt::Arguments::new_v1_formatted(&["Entry(",
                                                                           ", ppn[2]: ",
                                                                           ", ppn[1]: ",
                                                                           ", ppn[0]: ",
                                                                           ", flags: ",
                                                                           ")"],
                                                                         &match (&self.0,
                                                                                 &self.extract_ppn(2),
                                                                                 &self.extract_ppn(1),
                                                                                 &self.extract_ppn(0),
                                                                                 &extract_value(self.0,
                                                                                                (1
                                                                                                     <<
                                                                                                     8)
                                                                                                    -
                                                                                                    1,
                                                                                                0))
                                                                              {
                                                                              (arg0,
                                                                               arg1,
                                                                               arg2,
                                                                               arg3,
                                                                               arg4)
                                                                              =>
                                                                              [::core::fmt::ArgumentV1::new(arg0,
                                                                                                            ::core::fmt::LowerHex::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg1,
                                                                                                            ::core::fmt::Display::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg2,
                                                                                                            ::core::fmt::Display::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg3,
                                                                                                            ::core::fmt::Display::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg4,
                                                                                                            ::core::fmt::Binary::fmt)],
                                                                          },
                                                                         &[::core::fmt::rt::v1::Argument{position:
                                                                                                             0usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 4u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             1usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 0u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             2usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 0u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             3usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 0u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             4usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 12u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Is(10usize),},}]))
                }
            }
            unsafe impl Send for Entry { }
            impl Entry {
                pub const fn new(bits: usize) -> Self { Entry(bits) }
                pub fn set_bits(&mut self, bits: usize) { self.0 = bits }
                pub fn set_flags(&mut self, flags: usize) {
                    self.0 = set_range(self.0, flags, 0, 8);
                }
                pub const fn extract_ppn(&self, idx: usize) -> usize {
                    match idx {
                        0 => extract_value(self.0, (1 << 9) - 1, 10),
                        1 => extract_value(self.0, (1 << 9) - 1, 19),
                        2 => extract_value(self.0, (1 << 26) - 1, 28),
                        _ =>
                        ::core::panicking::panic("[entry.extract_ppn] idx should be one of 0..=2"),
                    }
                }
                pub const fn extract_ppn_all(&self) -> usize {
                    extract_value(self.0, (1 << 44) - 1, 10)
                }
                pub fn set_ppn(&mut self, paddr: PhysicalAddr) {
                    self.0 =
                        set_range(self.0, paddr.extract_ppn_all(), 10, 54)
                }
                pub const fn is_leaf(&self) -> bool {
                    (self.0 & (READ | WRITE | EXECUTE)) != 0
                }
                pub const fn is_branch(&self) -> bool { !self.is_leaf() }
                pub const fn is_valid(&self) -> bool { (self.0 & VALID) != 0 }
                pub fn set_valid(&mut self) { self.0 |= VALID }
                pub fn clear_valid(&mut self) { self.0 &= !VALID }
                pub const fn is_read(&self) -> bool { (self.0 & READ) != 0 }
                pub fn set_read(&mut self) { self.0 |= READ }
                pub fn clear_read(&mut self) { self.0 &= !READ }
                pub const fn is_write(&self) -> bool { (self.0 & WRITE) != 0 }
                pub fn set_write(&mut self) { self.0 |= WRITE }
                pub fn clear_write(&mut self) { self.0 &= !WRITE }
                pub const fn is_execute(&self) -> bool {
                    (self.0 & EXECUTE) != 0
                }
                pub fn set_execute(&mut self) { self.0 |= EXECUTE }
                pub fn clear_execute(&mut self) { self.0 &= !EXECUTE }
                pub const fn is_user(&self) -> bool { (self.0 & USER) != 0 }
                pub fn set_user(&mut self) { self.0 |= USER }
                pub fn clear_user(&mut self) { self.0 &= !USER }
                pub const fn is_global(&self) -> bool {
                    (self.0 & GLOBAL) != 0
                }
                pub fn set_global(&mut self) { self.0 |= GLOBAL }
                pub fn clear_global(&mut self) { self.0 &= !GLOBAL }
                pub const fn is_access(&self) -> bool {
                    (self.0 & ACCESS) != 0
                }
                pub fn set_access(&mut self) { self.0 |= ACCESS }
                pub fn clear_access(&mut self) { self.0 &= !ACCESS }
                pub const fn is_dirty(&self) -> bool { (self.0 & DIRTY) != 0 }
                pub fn set_dirty(&mut self) { self.0 |= DIRTY }
                pub fn clear_dirty(&mut self) { self.0 &= !DIRTY }
                pub const fn is_read_write(&self) -> bool {
                    (self.0 & READ_WRITE) != 0
                }
                pub fn set_read_write(&mut self) { self.0 |= READ_WRITE }
                pub fn clear_read_write(&mut self) { self.0 &= !READ_WRITE }
                pub const fn is_read_execute(&self) -> bool {
                    (self.0 & READ_EXECUTE) != 0
                }
                pub fn set_read_execute(&mut self) { self.0 |= READ_EXECUTE }
                pub fn clear_read_execute(&mut self) {
                    self.0 &= !READ_EXECUTE
                }
                pub const fn is_read_write_execute(&self) -> bool {
                    (self.0 & READ_WRITE_EXECUTE) != 0
                }
                pub fn set_read_write_execute(&mut self) {
                    self.0 |= READ_WRITE_EXECUTE
                }
                pub fn clear_read_write_execute(&mut self) {
                    self.0 &= !READ_WRITE_EXECUTE
                }
                pub const fn is_user_read_write(&self) -> bool {
                    (self.0 & USER_READ_WRITE) != 0
                }
                pub fn set_user_read_write(&mut self) {
                    self.0 |= USER_READ_WRITE
                }
                pub fn clear_user_read_write(&mut self) {
                    self.0 &= !USER_READ_WRITE
                }
                pub const fn is_user_read_execute(&self) -> bool {
                    (self.0 & USER_READ_EXECUTE) != 0
                }
                pub fn set_user_read_execute(&mut self) {
                    self.0 |= USER_READ_EXECUTE
                }
                pub fn clear_user_read_execute(&mut self) {
                    self.0 &= !USER_READ_EXECUTE
                }
                pub const fn is_user_read_write_execute(&self) -> bool {
                    (self.0 & USER_READ_WRITE_EXECUTE) != 0
                }
                pub fn set_user_read_write_execute(&mut self) {
                    self.0 |= USER_READ_WRITE_EXECUTE
                }
                pub fn clear_user_read_write_execute(&mut self) {
                    self.0 &= !USER_READ_WRITE_EXECUTE
                }
            }
            #[repr(transparent)]
            pub struct Table {
                entries: [Entry; ENTRIES_PER_PAGE],
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::fmt::Debug for Table {
                fn fmt(&self, f: &mut ::core::fmt::Formatter)
                 -> ::core::fmt::Result {
                    match *self {
                        Table { entries: ref __self_0_0 } => {
                            let debug_trait_builder =
                                &mut ::core::fmt::Formatter::debug_struct(f,
                                                                          "Table");
                            let _ =
                                ::core::fmt::DebugStruct::field(debug_trait_builder,
                                                                "entries",
                                                                &&(*__self_0_0));
                            ::core::fmt::DebugStruct::finish(debug_trait_builder)
                        }
                    }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::clone::Clone for Table {
                #[inline]
                fn clone(&self) -> Table {
                    {
                        let _:
                                ::core::clone::AssertParamIsClone<[Entry; ENTRIES_PER_PAGE]>;
                        *self
                    }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::marker::Copy for Table { }
            #[allow(unknown_lints, eq_op)]
            const _:
             [(); 0 -
                      !{
                           const ASSERT: bool =
                               size_of::<Table>() == PAGE_SIZE;
                           ASSERT
                       } as usize] =
                [];
            unsafe impl Send for Table { }
            impl Table {
                pub const fn new() -> Self {
                    Table{entries: [Entry::new(0); ENTRIES_PER_PAGE],}
                }
            }
            unsafe fn alloc_entry_page(entry: &mut Entry) {
                let ptr = alloc_zeroed(Layout::new::<Table>());
                entry.set_ppn(PhysicalAddr::new(ptr as usize));
                entry.set_valid();
            }
            ///       The bits should contain only the following:
            ///          Read, Write, Execute, User, and/or Global
            ///       The bits MUST include one or more of the following:
            ///          Read, Write, Execute
            pub unsafe fn map(root: *mut Table, vaddr: VirtualAddr,
                              paddr: PhysicalAddr, flags: usize) {
                let mut table = root;
                for lvl in (1..=2).rev() {
                    let entry = &mut (*table).entries[vaddr.extract_vpn(lvl)];
                    if !entry.is_valid() { alloc_entry_page(entry); }
                    let ppn = entry.extract_ppn_all();
                    table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
                }
                let entry = &mut (*table).entries[vaddr.extract_vpn(0)];
                entry.set_ppn(paddr);
                entry.set_flags(flags);
                entry.set_valid();
                let mapped = virt_to_phys(root, vaddr);
                if !(mapped == Some(paddr)) {
                    ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                  " mapped to ",
                                                                                  " but get "],
                                                                                &match (&vaddr,
                                                                                        &paddr,
                                                                                        &mapped)
                                                                                     {
                                                                                     (arg0,
                                                                                      arg1,
                                                                                      arg2)
                                                                                     =>
                                                                                     [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                   ::core::fmt::Debug::fmt),
                                                                                      ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                   ::core::fmt::Debug::fmt),
                                                                                      ::core::fmt::ArgumentV1::new(arg2,
                                                                                                                   ::core::fmt::Debug::fmt)],
                                                                                 }))
                };
            }
            pub unsafe fn unmap(root: *mut Table) {
                for entry in (*root).entries.iter_mut() {
                    let ppn = entry.extract_ppn_all();
                    if entry.is_valid() {
                        if entry.is_branch() {
                            let table =
                                PhysicalAddr::from(ppn,
                                                   0).as_mut_ptr::<Table>();
                            unmap(table);
                        }
                        dealloc(PhysicalAddr::from(ppn, 0).as_mut_ptr::<u8>(),
                                Layout::new::<Table>());
                    }
                }
            }
            pub fn virt_to_phys(root: *const Table, vaddr: VirtualAddr)
             -> Option<PhysicalAddr> {
                let mut table = root;
                for lvl in (1..=2).rev() {
                    let entry =
                        unsafe { &(*table).entries[vaddr.extract_vpn(lvl)] };
                    if !entry.is_valid() { return None; }
                    let ppn = entry.extract_ppn_all();
                    table = PhysicalAddr::from(ppn, 0).as_mut_ptr::<Table>();
                }
                let entry =
                    unsafe { &(*table).entries[vaddr.extract_vpn(0)] };
                let ppn = entry.extract_ppn_all();
                Some(PhysicalAddr::from(ppn, vaddr.extract_offset()))
            }
        }
        pub mod virtual_addr {
            use crate::utils::{extract_value, set_range};
            use bitflags::bitflags;
            use core::fmt;
            use core::ptr;
            #[repr(transparent)]
            pub struct VirtualAddr(usize);
            impl ::core::marker::StructuralPartialEq for VirtualAddr { }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::PartialEq for VirtualAddr {
                #[inline]
                fn eq(&self, other: &VirtualAddr) -> bool {
                    match *other {
                        VirtualAddr(ref __self_1_0) =>
                        match *self {
                            VirtualAddr(ref __self_0_0) =>
                            (*__self_0_0) == (*__self_1_0),
                        },
                    }
                }
                #[inline]
                fn ne(&self, other: &VirtualAddr) -> bool {
                    match *other {
                        VirtualAddr(ref __self_1_0) =>
                        match *self {
                            VirtualAddr(ref __self_0_0) =>
                            (*__self_0_0) != (*__self_1_0),
                        },
                    }
                }
            }
            impl ::core::marker::StructuralEq for VirtualAddr { }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::Eq for VirtualAddr {
                #[inline]
                #[doc(hidden)]
                #[no_coverage]
                fn assert_receiver_is_total_eq(&self) -> () {
                    { let _: ::core::cmp::AssertParamIsEq<usize>; }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::PartialOrd for VirtualAddr {
                #[inline]
                fn partial_cmp(&self, other: &VirtualAddr)
                 -> ::core::option::Option<::core::cmp::Ordering> {
                    match *other {
                        VirtualAddr(ref __self_1_0) =>
                        match *self {
                            VirtualAddr(ref __self_0_0) =>
                            match ::core::cmp::PartialOrd::partial_cmp(&(*__self_0_0),
                                                                       &(*__self_1_0))
                                {
                                ::core::option::Option::Some(::core::cmp::Ordering::Equal)
                                =>
                                ::core::option::Option::Some(::core::cmp::Ordering::Equal),
                                cmp => cmp,
                            },
                        },
                    }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::cmp::Ord for VirtualAddr {
                #[inline]
                fn cmp(&self, other: &VirtualAddr) -> ::core::cmp::Ordering {
                    match *other {
                        VirtualAddr(ref __self_1_0) =>
                        match *self {
                            VirtualAddr(ref __self_0_0) =>
                            match ::core::cmp::Ord::cmp(&(*__self_0_0),
                                                        &(*__self_1_0)) {
                                ::core::cmp::Ordering::Equal =>
                                ::core::cmp::Ordering::Equal,
                                cmp => cmp,
                            },
                        },
                    }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::clone::Clone for VirtualAddr {
                #[inline]
                fn clone(&self) -> VirtualAddr {
                    { let _: ::core::clone::AssertParamIsClone<usize>; *self }
                }
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::marker::Copy for VirtualAddr { }
            impl fmt::Debug for VirtualAddr {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_fmt(::core::fmt::Arguments::new_v1_formatted(&["VirtualAddr(",
                                                                           ": vpn[2]: ",
                                                                           ", vpn[1]: ",
                                                                           ", vpn[0]: ",
                                                                           ", offset: ",
                                                                           ")"],
                                                                         &match (&self.0,
                                                                                 &self.extract_vpn(2),
                                                                                 &self.extract_vpn(1),
                                                                                 &self.extract_vpn(0),
                                                                                 &self.extract_offset())
                                                                              {
                                                                              (arg0,
                                                                               arg1,
                                                                               arg2,
                                                                               arg3,
                                                                               arg4)
                                                                              =>
                                                                              [::core::fmt::ArgumentV1::new(arg0,
                                                                                                            ::core::fmt::LowerHex::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg1,
                                                                                                            ::core::fmt::Display::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg2,
                                                                                                            ::core::fmt::Display::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg3,
                                                                                                            ::core::fmt::Display::fmt),
                                                                               ::core::fmt::ArgumentV1::new(arg4,
                                                                                                            ::core::fmt::LowerHex::fmt)],
                                                                          },
                                                                         &[::core::fmt::rt::v1::Argument{position:
                                                                                                             0usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 4u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             1usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 0u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             2usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 0u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             3usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 0u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},},
                                                                           ::core::fmt::rt::v1::Argument{position:
                                                                                                             4usize,
                                                                                                         format:
                                                                                                             ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                 ' ',
                                                                                                                                             align:
                                                                                                                                                 ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                             flags:
                                                                                                                                                 4u32,
                                                                                                                                             precision:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                             width:
                                                                                                                                                 ::core::fmt::rt::v1::Count::Implied,},}]))
                }
            }
            impl VirtualAddr {
                pub const fn new(vaddr: usize) -> Self { VirtualAddr(vaddr) }
                pub fn from(vpn: usize, offset: usize) -> Self {
                    let mut bits = set_range(0, vpn, 12, 39);
                    bits = set_range(bits, offset, 0, 12);
                    VirtualAddr(bits)
                }
                pub const fn as_ptr<T>(&self) -> *const T {
                    self.0 as *const T
                }
                pub const fn as_mut_ptr<T>(&self) -> *mut T {
                    self.0 as *mut T
                }
                pub fn extract_vpn(&self, idx: usize) -> usize {
                    let mask = (1 << 9) - 1;
                    match idx {
                        0 => extract_value(self.0, mask, 12),
                        1 => extract_value(self.0, mask, 21),
                        2 => extract_value(self.0, mask, 30),
                        _ =>
                        ::core::panicking::panic("[entry.extract_vpn] idx should be one of 0..=2"),
                    }
                }
                pub const fn extract_bits(&self) -> usize { self.0 }
                pub fn extract_offset(&self) -> usize {
                    extract_value(self.0, (1 << 12) - 1, 0)
                }
                pub fn set_offset(&mut self, offset: usize) -> Self {
                    VirtualAddr(set_range(self.0, offset, 0, 12))
                }
                pub fn is_aligned(&self, alignment: usize) -> bool {
                    match self {
                        VirtualAddr(addr) => *addr % alignment == 0,
                    }
                }
            }
        }
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct ROOT_TABLE {
            __private_field: (),
        }
        #[doc(hidden)]
        pub static ROOT_TABLE: ROOT_TABLE = ROOT_TABLE{__private_field: (),};
        impl ::lazy_static::__Deref for ROOT_TABLE {
            type Target = Mutex<Box<Table>>;
            fn deref(&self) -> &Mutex<Box<Table>> {
                #[inline(always)]
                fn __static_ref_initialize() -> Mutex<Box<Table>> {
                    unsafe {
                        let ret = Mutex::new(Box::new(Table::new()));
                        {
                            let root = ret.lock().as_mut() as *mut _;
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] root page table created at, ",
                                                                                   "\n"],
                                                                                 &match (&root,)
                                                                                      {
                                                                                      (arg0,)
                                                                                      =>
                                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                    ::core::fmt::Debug::fmt)],
                                                                                  }));
                            ;
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping UART...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, UART_BASE_ADDR,
                                         UART_BASE_ADDR + PAGE_SIZE,
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping UART completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(UART_BASE_ADDR));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(UART_BASE_ADDR));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping CLINT...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, CLINT_BASE_ADDR,
                                         CLINT_BASE_ADDR + PAGE_SIZE,
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping CLINT completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(CLINT_BASE_ADDR));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(CLINT_BASE_ADDR));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping PLIC...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, PLIC_BASE_ADDR, PLIC_END_ADDR,
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping PLIC completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(PLIC_BASE_ADDR));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(PLIC_BASE_ADDR));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping text section...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, text_start(), text_end(),
                                         paging::READ_EXECUTE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping text section completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(text_start()));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(text_start()));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping rodata section...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, rodata_start(), rodata_end(),
                                         paging::READ);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping rodata section completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(rodata_start()));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(rodata_start()));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping data section...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, data_start(), data_end(),
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping data section completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(data_start()));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(data_start()));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping bss section...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, bss_start(), bss_end(),
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping bss section completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(bss_start()));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(bss_start()));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping kernel stack...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, kernel_stack_start(),
                                         kernel_stack_end() + PAGE_SIZE,
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping kernel stack completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(kernel_stack_start()));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(kernel_stack_start()));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping heap descriptors...\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            id_map_range(root, heap_start(), memory_end(),
                                         paging::READ_WRITE);
                            crate::console::print(::core::fmt::Arguments::new_v1(&["[initialize root table] mapping heap descriptors completed\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                            let expected =
                                Some(PhysicalAddr::new(heap_start()));
                            let mapped =
                                virt_to_phys(root,
                                             VirtualAddr::new(heap_start()));
                            if !(mapped == expected) {
                                ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["expect ",
                                                                                              ", but get "],
                                                                                            &match (&expected,
                                                                                                    &mapped)
                                                                                                 {
                                                                                                 (arg0,
                                                                                                  arg1)
                                                                                                 =>
                                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                               ::core::fmt::Debug::fmt),
                                                                                                  ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                               ::core::fmt::Debug::fmt)],
                                                                                             }))
                            };
                            crate::console::print(::core::fmt::Arguments::new_v1(&["root page table mapping initialized\n"],
                                                                                 &match ()
                                                                                      {
                                                                                      ()
                                                                                      =>
                                                                                      [],
                                                                                  }));
                            ;
                        }
                        ret
                    }
                }
                #[inline(always)]
                fn __stability() -> &'static Mutex<Box<Table>> {
                    static LAZY: ::lazy_static::lazy::Lazy<Mutex<Box<Table>>>
                     =
                        ::lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for ROOT_TABLE {
            fn initialize(lazy: &Self) { let _ = &**lazy; }
        }
        pub unsafe fn init() {
            let root = ROOT_TABLE.lock();
            let addr = root.as_ref() as *const _ as usize;
            let ppn = PhysicalAddr::new(addr).extract_ppn_all();
            crate::console::print(::core::fmt::Arguments::new_v1_formatted(&["[paging::init] set satp register, mode: ",
                                                                             ", ppn: ",
                                                                             "\n"],
                                                                           &match (&satp::Mode::Sv39,
                                                                                   &ppn)
                                                                                {
                                                                                (arg0,
                                                                                 arg1)
                                                                                =>
                                                                                [::core::fmt::ArgumentV1::new(arg0,
                                                                                                              ::core::fmt::Debug::fmt),
                                                                                 ::core::fmt::ArgumentV1::new(arg1,
                                                                                                              ::core::fmt::LowerHex::fmt)],
                                                                            },
                                                                           &[::core::fmt::rt::v1::Argument{position:
                                                                                                               0usize,
                                                                                                           format:
                                                                                                               ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                   ' ',
                                                                                                                                               align:
                                                                                                                                                   ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                               flags:
                                                                                                                                                   0u32,
                                                                                                                                               precision:
                                                                                                                                                   ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                               width:
                                                                                                                                                   ::core::fmt::rt::v1::Count::Implied,},},
                                                                             ::core::fmt::rt::v1::Argument{position:
                                                                                                               1usize,
                                                                                                           format:
                                                                                                               ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                   ' ',
                                                                                                                                               align:
                                                                                                                                                   ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                               flags:
                                                                                                                                                   4u32,
                                                                                                                                               precision:
                                                                                                                                                   ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                               width:
                                                                                                                                                   ::core::fmt::rt::v1::Count::Implied,},}]));
            ;
            satp::set(satp::Mode::Sv39, 0, ppn);
            crate::console::print(::core::fmt::Arguments::new_v1(&["[paging::init] set satp register completed\n"],
                                                                 &match () {
                                                                      () =>
                                                                      [],
                                                                  }));
            ;
            crate::console::print(::core::fmt::Arguments::new_v1(&["[paging::init] sfence_vma_all\n"],
                                                                 &match () {
                                                                      () =>
                                                                      [],
                                                                  }));
            ;
            sfence_vma_all();
            crate::console::print(::core::fmt::Arguments::new_v1(&["[paging::init] sfence_vma_all completed\n"],
                                                                 &match () {
                                                                      () =>
                                                                      [],
                                                                  }));
            ;
        }
    }
    use allocator::Allocator;
    use layout::{HEAP_END, HEAP_START};
    #[allow(missing_copy_implementations)]
    #[allow(non_camel_case_types)]
    #[allow(dead_code)]
    struct ALLOCATOR {
        __private_field: (),
    }
    #[doc(hidden)]
    static ALLOCATOR: ALLOCATOR = ALLOCATOR{__private_field: (),};
    impl ::lazy_static::__Deref for ALLOCATOR {
        type Target = Mutex<Allocator>;
        fn deref(&self) -> &Mutex<Allocator> {
            #[inline(always)]
            fn __static_ref_initialize() -> Mutex<Allocator> {
                unsafe {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL &&
                               lvl <= ::log::max_level() {
                            ::log::__private_api_log(::core::fmt::Arguments::new_v1(&["[allocator] initializing global heap allocator..."],
                                                                                    &match ()
                                                                                         {
                                                                                         ()
                                                                                         =>
                                                                                         [],
                                                                                     }),
                                                     lvl,
                                                     &("kernel::memory",
                                                       "kernel::memory",
                                                       "kernel/src/memory/mod.rs",
                                                       16u32));
                        }
                    };
                    let allocator = Allocator::new(*HEAP_START, *HEAP_END);
                    crate::console::print(::core::fmt::Arguments::new_v1_formatted(&["[allocator] global heap allocator created at ",
                                                                                     "\n"],
                                                                                   &match (&(&allocator
                                                                                                 as
                                                                                                 *const _
                                                                                                 as
                                                                                                 usize),)
                                                                                        {
                                                                                        (arg0,)
                                                                                        =>
                                                                                        [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                      ::core::fmt::LowerHex::fmt)],
                                                                                    },
                                                                                   &[::core::fmt::rt::v1::Argument{position:
                                                                                                                       0usize,
                                                                                                                   format:
                                                                                                                       ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                           ' ',
                                                                                                                                                       align:
                                                                                                                                                           ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                       flags:
                                                                                                                                                           4u32,
                                                                                                                                                       precision:
                                                                                                                                                           ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                       width:
                                                                                                                                                           ::core::fmt::rt::v1::Count::Implied,},}]));
                    ;
                    Mutex::new(allocator)
                }
            }
            #[inline(always)]
            fn __stability() -> &'static Mutex<Allocator> {
                static LAZY: ::lazy_static::lazy::Lazy<Mutex<Allocator>> =
                    ::lazy_static::lazy::Lazy::INIT;
                LAZY.get(__static_ref_initialize)
            }
            __stability()
        }
    }
    impl ::lazy_static::LazyStatic for ALLOCATOR {
        fn initialize(lazy: &Self) { let _ = &**lazy; }
    }
    struct OsAllocator;
    unsafe impl GlobalAlloc for OsAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let r = ALLOCATOR.lock().alloc(layout);
            r
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            ALLOCATOR.lock().dealloc(ptr, layout);
        }
    }
    static GA: OsAllocator = OsAllocator;
    const _: () =
        {
            #[rustc_std_internal_symbol]
            unsafe fn __rg_alloc(arg0: usize, arg1: usize) -> *mut u8 {
                ::core::alloc::GlobalAlloc::alloc(&GA,
                                                  ::core::alloc::Layout::from_size_align_unchecked(arg0,
                                                                                                   arg1))
                    as *mut u8
            }
            #[rustc_std_internal_symbol]
            unsafe fn __rg_dealloc(arg0: *mut u8, arg1: usize, arg2: usize)
             -> () {
                ::core::alloc::GlobalAlloc::dealloc(&GA, arg0 as *mut u8,
                                                    ::core::alloc::Layout::from_size_align_unchecked(arg1,
                                                                                                     arg2))
            }
            #[rustc_std_internal_symbol]
            unsafe fn __rg_realloc(arg0: *mut u8, arg1: usize, arg2: usize,
                                   arg3: usize) -> *mut u8 {
                ::core::alloc::GlobalAlloc::realloc(&GA, arg0 as *mut u8,
                                                    ::core::alloc::Layout::from_size_align_unchecked(arg1,
                                                                                                     arg2),
                                                    arg3) as *mut u8
            }
            #[rustc_std_internal_symbol]
            unsafe fn __rg_alloc_zeroed(arg0: usize, arg1: usize) -> *mut u8 {
                ::core::alloc::GlobalAlloc::alloc_zeroed(&GA,
                                                         ::core::alloc::Layout::from_size_align_unchecked(arg0,
                                                                                                          arg1))
                    as *mut u8
            }
        };
    #[alloc_error_handler]
    pub fn alloc_error(l: Layout) -> ! {
        ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["Allocator failed to allocate ",
                                                                      " bytes with ",
                                                                      "-byte alignment."],
                                                                    &match (&l.size(),
                                                                            &l.align())
                                                                         {
                                                                         (arg0,
                                                                          arg1)
                                                                         =>
                                                                         [::core::fmt::ArgumentV1::new(arg0,
                                                                                                       ::core::fmt::Display::fmt),
                                                                          ::core::fmt::ArgumentV1::new(arg1,
                                                                                                       ::core::fmt::Display::fmt)],
                                                                     }));
    }
    pub unsafe fn memset(ptr: *mut u8, ch: u8, count: usize) {
        for _ in 0..count { *ptr = ch; }
    }
}
mod sbi {
    #![allow(unused)]
    const SBI_SET_TIMER: usize = 0;
    const SBI_CONSOLE_PUTCHAR: usize = 1;
    const SBI_CONSOLE_GETCHAR: usize = 2;
    const SBI_CLEAR_IPI: usize = 3;
    const SBI_SEND_IPI: usize = 4;
    const SBI_REMOTE_FENCE_I: usize = 5;
    const SBI_REMOTE_SFENCE_VMA: usize = 6;
    const SBI_REMOTE_SFENCE_VMA_ASID: usize = 7;
    const SBI_SHUTDOWN: usize = 8;
    #[inline(always)]
    fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize)
     -> usize {
        let mut ret;
        unsafe {
            llvm_asm!("ecall": "={x10}"(ret) :
                "{x10}"(arg0), "{x11}"(arg1), "{x12}"(arg2), "{x17}"(which) :
                "memory" : "volatile");
        }
        ret
    }
    #[inline(always)]
    pub fn console_putchar(c: usize) {
        sbi_call(SBI_CONSOLE_PUTCHAR, c, 0, 0);
    }
    #[inline(always)]
    pub fn console_getchar() -> usize {
        sbi_call(SBI_CONSOLE_GETCHAR, 0, 0, 0)
    }
    #[inline(always)]
    pub fn shutdown() -> ! {
        sbi_call(SBI_SHUTDOWN, 0, 0, 0);
        ::core::panicking::panic("It should shutdown!");
    }
    #[inline(always)]
    pub fn hart_id() -> usize {
        let mut hart_id: usize = 0;
        unsafe { llvm_asm!("mv $0, tp": "=r"(hart_id) :  :  : "volatile"); }
        hart_id
    }
}
mod syscall {
    const SYSCALL_WRITE: usize = 64;
    const SYSCALL_EXIT: usize = 93;
    mod fs {
        use crate::batch::is_valid_location;
        const FD_STDOUT: usize = 1;
        pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
            match fd {
                FD_STDOUT => {
                    if !is_valid_location(buf as usize) ||
                           !is_valid_location(buf as usize + len) {
                        crate::console::print(::core::fmt::Arguments::new_v1(&["[kernel] buf out of range\n"],
                                                                             &match ()
                                                                                  {
                                                                                  ()
                                                                                  =>
                                                                                  [],
                                                                              }));
                        ;
                        return -1;
                    }
                    let slice =
                        unsafe { core::slice::from_raw_parts(buf, len) };
                    let str = core::str::from_utf8(slice).unwrap();
                    crate::console::print(::core::fmt::Arguments::new_v1(&[""],
                                                                         &match (&str,)
                                                                              {
                                                                              (arg0,)
                                                                              =>
                                                                              [::core::fmt::ArgumentV1::new(arg0,
                                                                                                            ::core::fmt::Display::fmt)],
                                                                          }));
                    ;
                    len as isize
                }
                _ => {
                    crate::console::print(::core::fmt::Arguments::new_v1(&["[kernel] Unsupported fd in sys_write!\n"],
                                                                         &match ()
                                                                              {
                                                                              ()
                                                                              =>
                                                                              [],
                                                                          }));
                    ;
                    -1
                }
            }
        }
    }
    mod process {
        use crate::batch::run_next_app;
        pub fn sys_exit(exit_code: i32) -> ! {
            crate::console::print(::core::fmt::Arguments::new_v1(&["[kernel] Application exited with code ",
                                                                   "\n"],
                                                                 &match (&exit_code,)
                                                                      {
                                                                      (arg0,)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Display::fmt)],
                                                                  }));
            ;
            run_next_app()
        }
    }
    use fs::*;
    use process::*;
    pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
        match syscall_id {
            SYSCALL_WRITE =>
            sys_write(args[0], args[1] as *const u8, args[2]),
            SYSCALL_EXIT => sys_exit(args[0] as i32),
            _ =>
            ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1(&["Unsupported syscall_id: "],
                                                                        &match (&syscall_id,)
                                                                             {
                                                                             (arg0,)
                                                                             =>
                                                                             [::core::fmt::ArgumentV1::new(arg0,
                                                                                                           ::core::fmt::Display::fmt)],
                                                                         })),
        }
    }
}
mod trap {
    use log::debug;
    use riscv::register::{mtvec::TrapMode, scause::{self, Exception, Trap},
                          stval, stvec};
    mod context {
        use riscv::register::sstatus::{self, Sstatus, SPP};
        #[repr(C)]
        pub struct TrapContext {
            pub x: [usize; 32],
            pub sstatus: Sstatus,
            pub sepc: usize,
        }
        impl TrapContext {
            pub fn set_sp(&mut self, sp: usize) { self.x[2] = sp; }
            pub fn app_init_context(entry: usize, sp: usize) -> Self {
                let mut sstatus = sstatus::read();
                sstatus.set_spp(SPP::User);
                let mut cx = Self{x: [0; 32], sstatus, sepc: entry,};
                cx.set_sp(sp);
                cx
            }
        }
    }
    pub use context::TrapContext;
    global_asm! ("# disable generation of compressed instructions.\n.option norvc\n\n\n.equ XLENB, 8\n\n.macro LOAD_SP a1, a2\n    ld \\a1, \\a2*XLENB(sp)\n.endm\n\n.macro STORE_SP a1, a2\n    sd \\a1, \\a2*XLENB(sp)\n.endm\n\n.macro STORE_ALL\n        # 中断可能来自用户态（U-Mode），也可能来自内核态（S-Mode）。\n        # 如果是用户态中断，那么此时的栈指针 sp 指向的是用户栈；如果是内核态中断，那么 sp 指向的是内核栈。\n        # 我们规定：当 CPU 处于 U-Mode 时，sscratch 保存内核栈地址；处于 S-Mode 时，sscratch 为 0 。\n\n        # 交换 sp 和 sscratch 寄存器\n        csrrw sp, sscratch, sp\n        # 判断 sp（也就是交换前的 sscratch）是否为0\n        # 如果非0，说明是用户态中断，由于 sscratch 保存的是内核栈地址\n        # 此时 sp 已经指向内核栈，直接跳转到 trap_from_user 保存寄存器\n        bnez sp, save_registers\n\n    trap_from_kernel:\n        # 如果是内核态中断，则继续使用内核栈即可。所以交换回去\n        csrr sp, sscratch\n    \n    save_registers:\n        # 为 TrapFrame 预留空间\n        addi sp, sp, -34 * XLENB\n\n        # save general registers except sp(x2)\n        # 因为现在的 sp 指向中断处理时需要用到的 sp 而不是中断来源处的 sp\n        # 我们将在稍后存储真正的 sp\n        .set n, 0\n        .rept 32\n            STORE_SP x%n, %n\n            .set n, n+1\n        .endr\n        csrr t0, sstatus\n        STORE_SP t0, 32\n        csrr t1, sepc\n        STORE_SP t1, 33\n\n        # 从 sscratch 中换回 sp 到 s0; 按照规定，进入内核态后 sscratch 应为 0\n        csrrw s0, sscratch, x0\n        # 保存 sp\n        STORE_SP s0, 2\n\n.endmacro\n\n.macro RESTORE_ALL\n        # 首先根据 sstatus 寄存器中的 SPP 位，判断是回到用户态还是内核态。\n        # 如果是回到用户态，根据规定需要设置 sscratch 为内核栈\n        csrr s0, sstatus # s0 = sstatus\n        csrr s1, sepc # s1 = sepc\n        andi s2, s0, 1 << 8     # sstatus.SPP = 1?\n        bnez s2, restore_registers     # s0 = back to kernel?\n    back_to_user:\n        # 如果是回到用户态，根据规定需要还原当前 sp 到 sscratch\n        addi s0, sp, 32*XLENB\n        csrw sscratch, s0         # sscratch = kernel-sp\n    restore_registers:\n        LOAD_SP t1, 33\n        csrw sepc, t1\n        LOAD_SP t0, 32\n        csrw sstatus, t0\n\n        LOAD_SP x0, 0\n        LOAD_SP x1, 0\n        .set n, 3\n        .rept 31\n            LOAD_SP x%n, %n\n            .set n, n+1\n        .endr\n\n        # release TrapContext on kernel stack\n        addi sp, sp, 34 * XLENB\n        # 目前 sp 指向内核栈，我们将其存回 sscratch\n        csrrw sp, sscratch, sp\n\n        # restore sp last\n        LOAD_SP sp, 2\n.endmacro\n\n\n# ---\n\n.section .text\n.global trap_entry\n.balign 4\ntrap_entry:\n    STORE_ALL\n\n    mv a0, sp\n    jal trap_handler\n\ntrap_ret:\n    RESTORE_ALL\n\n    sret\n\n\n\n\n")
        extern "C" {
            fn trap_entry();
        }
        pub fn init() {
            let addr = trap_entry as usize;
            let mode = stvec::TrapMode::Direct;
            {
                let lvl = ::log::Level::Debug;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level()
                   {
                    ::log::__private_api_log(::core::fmt::Arguments::new_v1_formatted(&["set stec register: trap_entry ",
                                                                                        ", mode "],
                                                                                      &match (&addr,
                                                                                              &mode)
                                                                                           {
                                                                                           (arg0,
                                                                                            arg1)
                                                                                           =>
                                                                                           [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                         ::core::fmt::LowerHex::fmt),
                                                                                            ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                         ::core::fmt::Debug::fmt)],
                                                                                       },
                                                                                      &[::core::fmt::rt::v1::Argument{position:
                                                                                                                          0usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},},
                                                                                        ::core::fmt::rt::v1::Argument{position:
                                                                                                                          1usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              0u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},}]),
                                             lvl,
                                             &("kernel::trap", "kernel::trap",
                                               "kernel/src/trap/mod.rs",
                                               57u32));
                }
            };
            unsafe { stvec::write(addr, mode); }
        }
        #[no_mangle]
        pub fn trap_handler(cx: &mut TrapContext) -> &mut TrapContext {
            let scause = scause::read();
            let stval = stval::read();
            match scause.cause() {
                Trap::Exception(Exception::UserEnvCall) => {
                    cx.sepc += 4;
                    cx.x[10] =
                        syscall(cx.x[17], [cx.x[10], cx.x[11], cx.x[12]]) as
                            usize;
                }
                Trap::Exception(Exception::StoreFault) |
                Trap::Exception(Exception::StorePageFault) => {
                    crate::console::print(::core::fmt::Arguments::new_v1(&["[kernel] PageFault in application, core dumped.\n"],
                                                                         &match ()
                                                                              {
                                                                              ()
                                                                              =>
                                                                              [],
                                                                          }));
                    ;
                    run_next_app();
                }
                Trap::Exception(Exception::IllegalInstruction) => {
                    crate::console::print(::core::fmt::Arguments::new_v1(&["[kernel] IllegalInstruction in application, core dumped.\n"],
                                                                         &match ()
                                                                              {
                                                                              ()
                                                                              =>
                                                                              [],
                                                                          }));
                    ;
                    run_next_app();
                }
                _ => {
                    ::core::panicking::panic_fmt(::core::fmt::Arguments::new_v1_formatted(&["Unsupported trap ",
                                                                                            ", stval = ",
                                                                                            "!"],
                                                                                          &match (&scause.cause(),
                                                                                                  &stval)
                                                                                               {
                                                                                               (arg0,
                                                                                                arg1)
                                                                                               =>
                                                                                               [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                             ::core::fmt::Debug::fmt),
                                                                                                ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                             ::core::fmt::LowerHex::fmt)],
                                                                                           },
                                                                                          &[::core::fmt::rt::v1::Argument{position:
                                                                                                                              0usize,
                                                                                                                          format:
                                                                                                                              ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                                  ' ',
                                                                                                                                                              align:
                                                                                                                                                                  ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                              flags:
                                                                                                                                                                  0u32,
                                                                                                                                                              precision:
                                                                                                                                                                  ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                              width:
                                                                                                                                                                  ::core::fmt::rt::v1::Count::Implied,},},
                                                                                            ::core::fmt::rt::v1::Argument{position:
                                                                                                                              1usize,
                                                                                                                          format:
                                                                                                                              ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                                  ' ',
                                                                                                                                                              align:
                                                                                                                                                                  ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                              flags:
                                                                                                                                                                  4u32,
                                                                                                                                                              precision:
                                                                                                                                                                  ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                              width:
                                                                                                                                                                  ::core::fmt::rt::v1::Count::Implied,},}]));
                }
            }
            cx
        }
    }
    mod utils {
        use alloc::{format, string::String};
        use core::mem::size_of;
        use core::ops::Range;
        pub const fn set_nth_bit(bits: usize, n: usize, b: bool) -> usize {
            if !(n < size_of::<usize>() * 8) {
                ::core::panicking::panic("assertion failed: n < size_of::<usize>() * 8")
            };
            bits & !(1 << n) | (if b { 1 } else { 0 } << n)
        }
        pub const fn toggle_nth_bit(bits: usize, n: usize) -> usize {
            if !(n < size_of::<usize>() * 8) {
                ::core::panicking::panic("assertion failed: n < size_of::<usize>() * 8")
            };
            bits ^ (1 << n)
        }
        pub const fn extract_nth_bit(bits: usize, n: usize) -> bool {
            if !(n < size_of::<usize>() * 8) {
                ::core::panicking::panic("assertion failed: n < size_of::<usize>() * 8")
            };
            match (bits >> n) & 1 {
                0 => false,
                1 => true,
                _ => ::core::panicking::panic("unexpected result"),
            }
        }
        pub const fn extract_value(bits: usize, mask: usize, start_pos: usize)
         -> usize {
            if !(start_pos < size_of::<usize>() * 8) {
                ::core::panicking::panic("assertion failed: start_pos < size_of::<usize>() * 8")
            };
            (bits & (mask << start_pos)) >> start_pos
        }
        pub fn set_range(bits: usize, val: usize, start_pos: usize,
                         end_pos: usize) -> usize {
            if !(start_pos < size_of::<usize>() * 8 &&
                     end_pos < size_of::<usize>() * 8) {
                ::core::panicking::panic("assertion failed: start_pos < size_of::<usize>() * 8 && end_pos < size_of::<usize>() * 8")
            };
            if !(start_pos < end_pos) {
                ::core::panicking::panic("assertion failed: start_pos < end_pos")
            };
            (start_pos..end_pos).fold(bits,
                                      |bits, n|
                                          {
                                              let b =
                                                  extract_nth_bit(val,
                                                                  n -
                                                                      start_pos);
                                              set_nth_bit(bits, n, b)
                                          })
        }
        pub const KILOBYTE: usize = 1024;
        pub const MEGABYTE: usize = 1024 * KILOBYTE;
        pub const GIGABYTE: usize = 1024 * MEGABYTE;
        pub const TERABYTE: usize = 1024 * GIGABYTE;
        pub fn format_size(size: usize) -> String {
            if size >= 2 * TERABYTE {
                {
                    let res =
                        ::alloc::fmt::format(::core::fmt::Arguments::new_v1(&["",
                                                                              " TB"],
                                                                            &match (&(size
                                                                                          /
                                                                                          TERABYTE),)
                                                                                 {
                                                                                 (arg0,)
                                                                                 =>
                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                               ::core::fmt::Display::fmt)],
                                                                             }));
                    res
                }
            } else if size >= 2 * GIGABYTE {
                {
                    let res =
                        ::alloc::fmt::format(::core::fmt::Arguments::new_v1(&["",
                                                                              " GB"],
                                                                            &match (&(size
                                                                                          /
                                                                                          GIGABYTE),)
                                                                                 {
                                                                                 (arg0,)
                                                                                 =>
                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                               ::core::fmt::Display::fmt)],
                                                                             }));
                    res
                }
            } else if size >= 2 * MEGABYTE {
                {
                    let res =
                        ::alloc::fmt::format(::core::fmt::Arguments::new_v1(&["",
                                                                              " MB"],
                                                                            &match (&(size
                                                                                          /
                                                                                          MEGABYTE),)
                                                                                 {
                                                                                 (arg0,)
                                                                                 =>
                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                               ::core::fmt::Display::fmt)],
                                                                             }));
                    res
                }
            } else if size >= 2 * KILOBYTE {
                {
                    let res =
                        ::alloc::fmt::format(::core::fmt::Arguments::new_v1(&["",
                                                                              " KB"],
                                                                            &match (&(size
                                                                                          /
                                                                                          KILOBYTE),)
                                                                                 {
                                                                                 (arg0,)
                                                                                 =>
                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                               ::core::fmt::Display::fmt)],
                                                                             }));
                    res
                }
            } else {
                {
                    let res =
                        ::alloc::fmt::format(::core::fmt::Arguments::new_v1(&["",
                                                                              " B"],
                                                                            &match (&size,)
                                                                                 {
                                                                                 (arg0,)
                                                                                 =>
                                                                                 [::core::fmt::ArgumentV1::new(arg0,
                                                                                                               ::core::fmt::Display::fmt)],
                                                                             }));
                    res
                }
            }
        }
        pub unsafe fn zero_volatile<T>(range: Range<*mut T>) where
         T: From<u8> {
            let mut ptr = range.start;
            crate::console::print(::core::fmt::Arguments::new_v1(&["", "\n"],
                                                                 &match (&range,)
                                                                      {
                                                                      (arg0,)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Debug::fmt)],
                                                                  }));
            ;
            while ptr < range.end {
                core::ptr::write_volatile(ptr, T::from(0));
                ptr = ptr.offset(1);
            }
        }
        /// Align downwards. Returns the greatest x with alignment `align`
        /// so that x <= addr. The alignment must be a power of 2.
        pub fn align_down(addr: usize, align: usize) -> usize {
            if align.is_power_of_two() {
                addr & !(align - 1)
            } else if align == 0 {
                addr
            } else {
                ::core::panicking::panic("`align` must be a power of 2");
            }
        }
        /// Align upwards. Returns the smallest x with alignment `align`
        /// so that x >= addr. The alignment must be a power of 2.
        pub fn align_up(addr: usize, align: usize) -> usize {
            align_down(addr + align - 1, align)
        }
    }
    use crate::cpu::CPU;
    use crate::memory::layout::{BSS_END, BSS_START, DATA_END, DATA_START,
                                KERNEL_STACK_END, KERNEL_STACK_START,
                                RODATA_END, RODATA_START, TEXT_END,
                                TEXT_START};
    global_asm! ("# https://github.com/riscv/riscv-asm-manual/blob/master/riscv-asm.md\n\n.section .text.entry\n\n.globl _start\n_start:\n    # rustsbi-qemu set a0 = hartid and a1 = dtd\n\n    # Any hardware threads (hart) that are not bootstrapping\n    bnez    a0, 2f\n\n    # Global, uninitialized variables get the value 0 since these are allocated in the BSS section. \n    # However, since we\'re the OS, we are responsible for making sure that memory is 0.\n    la      a2, _bss_start\n    la      a3, _bss_end\n    bgeu    a2, a3, 2f\n1:\n    sd      zero, (a2)\n    addi    a2, a2, 8\n    bltu    a2, a3, 1b \n\n2:\n    # set kernel stacks for each hart, and make sure they are 0\n    # allocate 64kb stack for each hart\n    la      sp, _kernel_stack_end\n    # for a2 = 0; a2 < a0; a2 += 1\n    li      a2, 0\n    bgeu    a2, a0, 4f\n3:\n    li      a4, -65536\n    #       sp -= 65536\n    add     sp, sp, a4\n    addi    a2, a2, 1\n    # if a0 < a2 then goto 3b\n    bltu    a2, a0, 3b\n\n\n4:\n    bnez    a0, 5f\n    call    rust_main\n5:\n    call    rust_main_ap\n")
        static HAS_STARTED: AtomicBool = AtomicBool::new(false);
        #[no_mangle]
        fn rust_main(hart_id: usize) -> ! {
            crate::console::print(::core::fmt::Arguments::new_v1(&["main hart initializing\n"],
                                                                 &match () {
                                                                      () =>
                                                                      [],
                                                                  }));
            ;
            logger::init();
            {
                let lvl = ::log::Level::Info;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level()
                   {
                    ::log::__private_api_log(::core::fmt::Arguments::new_v1(&["=== memory layout ==="],
                                                                            &match ()
                                                                                 {
                                                                                 ()
                                                                                 =>
                                                                                 [],
                                                                             }),
                                             lvl,
                                             &("kernel", "kernel",
                                               "kernel/src/main.rs", 53u32));
                }
            };
            {
                let lvl = ::log::Level::Info;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level()
                   {
                    ::log::__private_api_log(::core::fmt::Arguments::new_v1_formatted(&["text_start: ",
                                                                                        ", text_end: "],
                                                                                      &match (&*TEXT_START,
                                                                                              &*TEXT_END)
                                                                                           {
                                                                                           (arg0,
                                                                                            arg1)
                                                                                           =>
                                                                                           [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                         ::core::fmt::LowerHex::fmt),
                                                                                            ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                         ::core::fmt::LowerHex::fmt)],
                                                                                       },
                                                                                      &[::core::fmt::rt::v1::Argument{position:
                                                                                                                          0usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},},
                                                                                        ::core::fmt::rt::v1::Argument{position:
                                                                                                                          1usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},}]),
                                             lvl,
                                             &("kernel", "kernel",
                                               "kernel/src/main.rs", 54u32));
                }
            };
            {
                let lvl = ::log::Level::Info;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level()
                   {
                    ::log::__private_api_log(::core::fmt::Arguments::new_v1_formatted(&["rodata_start: ",
                                                                                        ", rodata_end: "],
                                                                                      &match (&*RODATA_START,
                                                                                              &*RODATA_END)
                                                                                           {
                                                                                           (arg0,
                                                                                            arg1)
                                                                                           =>
                                                                                           [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                         ::core::fmt::LowerHex::fmt),
                                                                                            ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                         ::core::fmt::LowerHex::fmt)],
                                                                                       },
                                                                                      &[::core::fmt::rt::v1::Argument{position:
                                                                                                                          0usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},},
                                                                                        ::core::fmt::rt::v1::Argument{position:
                                                                                                                          1usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},}]),
                                             lvl,
                                             &("kernel", "kernel",
                                               "kernel/src/main.rs", 55u32));
                }
            };
            {
                let lvl = ::log::Level::Info;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level()
                   {
                    ::log::__private_api_log(::core::fmt::Arguments::new_v1_formatted(&["data_start: ",
                                                                                        ", data_end: "],
                                                                                      &match (&*DATA_START,
                                                                                              &*DATA_END)
                                                                                           {
                                                                                           (arg0,
                                                                                            arg1)
                                                                                           =>
                                                                                           [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                         ::core::fmt::LowerHex::fmt),
                                                                                            ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                         ::core::fmt::LowerHex::fmt)],
                                                                                       },
                                                                                      &[::core::fmt::rt::v1::Argument{position:
                                                                                                                          0usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},},
                                                                                        ::core::fmt::rt::v1::Argument{position:
                                                                                                                          1usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},}]),
                                             lvl,
                                             &("kernel", "kernel",
                                               "kernel/src/main.rs", 59u32));
                }
            };
            {
                let lvl = ::log::Level::Info;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level()
                   {
                    ::log::__private_api_log(::core::fmt::Arguments::new_v1_formatted(&["bss_start: ",
                                                                                        ", bss_end: "],
                                                                                      &match (&*BSS_START,
                                                                                              &*BSS_END)
                                                                                           {
                                                                                           (arg0,
                                                                                            arg1)
                                                                                           =>
                                                                                           [::core::fmt::ArgumentV1::new(arg0,
                                                                                                                         ::core::fmt::LowerHex::fmt),
                                                                                            ::core::fmt::ArgumentV1::new(arg1,
                                                                                                                         ::core::fmt::LowerHex::fmt)],
                                                                                       },
                                                                                      &[::core::fmt::rt::v1::Argument{position:
                                                                                                                          0usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},},
                                                                                        ::core::fmt::rt::v1::Argument{position:
                                                                                                                          1usize,
                                                                                                                      format:
                                                                                                                          ::core::fmt::rt::v1::FormatSpec{fill:
                                                                                                                                                              ' ',
                                                                                                                                                          align:
                                                                                                                                                              ::core::fmt::rt::v1::Alignment::Unknown,
                                                                                                                                                          flags:
                                                                                                                                                              4u32,
                                                                                                                                                          precision:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,
                                                                                                                                                          width:
                                                                                                                                                              ::core::fmt::rt::v1::Count::Implied,},}]),
                                             lvl,
                                             &("kernel", "kernel",
                                               "kernel/src/main.rs", 60u32));
                }
            };
            trap::init();
            HAS_STARTED.store(true, Ordering::SeqCst);
            let cpu = CPU{hart_id,};
            crate::console::print(::core::fmt::Arguments::new_v1(&["main hart ",
                                                                   " started\n"],
                                                                 &match (&cpu.hart_id,)
                                                                      {
                                                                      (arg0,)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Display::fmt)],
                                                                  }));
            ;
            loop  { }
        }
        #[no_mangle]
        fn rust_main_ap(hart_id: usize) -> ! {
            while !HAS_STARTED.load(Ordering::SeqCst) { }
            let cpu = CPU{hart_id,};
            crate::console::print(::core::fmt::Arguments::new_v1(&["hart ",
                                                                   " started\n"],
                                                                 &match (&cpu.hart_id,)
                                                                      {
                                                                      (arg0,)
                                                                      =>
                                                                      [::core::fmt::ArgumentV1::new(arg0,
                                                                                                    ::core::fmt::Display::fmt)],
                                                                  }));
            ;
            loop  { }
        }
