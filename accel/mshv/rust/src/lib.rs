/*
 * QEMU MSHV accelerator support
 *
 * Copyright Microsoft, Corp. 2017
 *
 * Authors: ziqiaozhou@microsoft.com
 *
 * This is a PoC code that need to be rewritten in C, without lots of Rust dependencies.
 *
 * This work is licensed under the terms of the GNU GPL, version 2 or later.
 * See the COPYING file in the top-level directory.
 *
 */

mod cpu;
mod memory;
mod msi;
mod vm;

pub use hypervisor::CpuVendor;
pub use mshv_bindings::StandardRegisters;

#[global_allocator]
static A: std::alloc::System = std::alloc::System;

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($arg)*);
        }
    };
}

#[no_mangle]
pub extern "C" fn mshv_new() {
    vm::init_vmdb();
    cpu::init_cpudb();
}
