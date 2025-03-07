/*
 * QEMU MSHV accelerator support
 *
 * Copyright Microsoft, Corp. 2017
 *
 * Authors: ziqiaozhou@microsoft.com
 *
 * This is a PoC code that need to be rewritten in C dropping dependencies.
 *
 * This work is licensed under the terms of the GNU GPL, version 2 or later.
 * See the COPYING file in the top-level directory.
 *
 */

use core::mem;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::sync::{Arc, Mutex, RwLock};

use hypervisor::mshv::{MshvHypervisor, MshvVm};
use hypervisor::{DataMatch, HypervisorVmError, IoEventAddress, Vm};
use mshv_ioctls::InterruptRequest;
use vmm_sys_util::eventfd::EventFd;

use crate::memory::MemManager;
use crate::msi::MSIControl;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref VM_DB: RwLock<Option<HashMap<RawFd, Arc<PerVmInfo>>>> = RwLock::new(None);
}

pub fn init_vmdb() {
    *VM_DB.write().unwrap() = Some(HashMap::new());
}

pub fn get_vminfo(vmfd: RawFd) -> Arc<PerVmInfo> {
    VM_DB.read().unwrap().as_ref().unwrap()[&vmfd].clone()
}

pub struct PerVmInfo {
    pub vm: Arc<dyn Vm>,
    pub msi: Arc<Mutex<MSIControl>>,
    pub mem: Arc<Mutex<MemManager>>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MshvOps {
    guest_mem_write_fn:
        unsafe extern "C" fn(gpa: u64, *const u8, size: usize, is_secure_mode: bool) -> i32,
    guest_mem_read_fn: unsafe extern "C" fn(gpa: u64, *mut u8, size: usize, is_secure_mode: bool),
    mmio_read_fn: unsafe extern "C" fn(gpa: u64, *mut u8, size: usize, is_secure_mode: bool),
    mmio_write_fn:
        unsafe extern "C" fn(gpa: u64, *const u8, size: usize, is_secure_mode: bool) -> i32,
    pio_read_fn: unsafe extern "C" fn(port: u64, data: *mut u8, size: usize, is_secure_mode: bool),
    pio_write_fn:
        unsafe extern "C" fn(port: u64, data: *const u8, size: usize, is_secure_mode: bool) -> i32,
}

impl hypervisor::VmOps for MshvOps {
    fn guest_mem_write(&self, gpa: u64, buf: &[u8]) -> Result<usize, HypervisorVmError> {
        let size = buf.len();
        let ret = unsafe { (self.guest_mem_write_fn)(gpa, buf.as_ptr(), size, false) };
        if ret != 0 {
            Err(HypervisorVmError::GuestMemWrite(anyhow::anyhow!(
                "write {} {:?}",
                gpa,
                buf
            )))
        } else {
            Ok(size)
        }
    }

    fn guest_mem_read(&self, gpa: u64, buf: &mut [u8]) -> Result<usize, HypervisorVmError> {
        let size = buf.len();
        unsafe {
            (self.guest_mem_read_fn)(gpa, buf.as_mut_ptr(), size, false);
        }
        Ok(size)
    }

    fn mmio_read(&self, gpa: u64, data: &mut [u8]) -> Result<(), HypervisorVmError> {
        let size = data.len();
        unsafe {
            (self.mmio_read_fn)(gpa, data.as_mut_ptr(), size, false);
        }
        Ok(())
    }

    fn mmio_write(&self, gpa: u64, data: &[u8]) -> Result<(), HypervisorVmError> {
        let size = data.len();
        unsafe {
            (self.mmio_write_fn)(gpa, data.as_ptr(), size, false);
        }
        Ok(())
    }

    #[cfg(target_arch = "x86_64")]
    fn pio_read(&self, port: u64, data: &mut [u8]) -> Result<(), HypervisorVmError> {
        let size = data.len();
        unsafe {
            (self.pio_read_fn)(port, data.as_mut_ptr(), size, false);
        }
        Ok(())
    }
    #[cfg(target_arch = "x86_64")]
    fn pio_write(&self, port: u64, data: &[u8]) -> Result<(), HypervisorVmError> {
        let size = data.len();
        unsafe {
            (self.pio_write_fn)(port, data.as_ptr(), size, false);
        }
        Ok(())
    }
}

#[no_mangle]
//vm_type: 0 -> Normal, 1 -> SNP
pub extern "C" fn mshv_create_vm_with_type(vm_type: u64) -> RawFd {
    let vm: Arc<dyn Vm> = match MshvHypervisor::new().unwrap().create_vm_with_type(vm_type) {
        Ok(vm) => vm,
        Err(e) => {
            panic!("[mshv] Failed to create a VM: {:?}", e);
        }
    };
    let tmpvm = vm.clone();
    let mshv_vm: &MshvVm = tmpvm.as_any().downcast_ref::<MshvVm>().unwrap();
    let fd = mshv_vm.fd().as_raw_fd();
    VM_DB.write().unwrap().as_mut().unwrap().insert(
        fd,
        Arc::new(PerVmInfo {
            vm: vm.clone(),
            msi: Arc::new(Mutex::new(MSIControl::new(vm))),
            mem: Arc::new(Mutex::new(MemManager::new())),
        }),
    );
    fd
}

#[no_mangle]
pub extern "C" fn mshv_vm_resume(vmfd: RawFd) {
    let vm = get_vminfo(vmfd).vm.clone();
    vm.resume().expect("vm resume: failed!");
}

#[no_mangle]
pub extern "C" fn mshv_vm_pause(vmfd: RawFd) {
    let vm = get_vminfo(vmfd).vm.clone();
    vm.pause().expect("vm pause: failed!");
}

#[no_mangle]
pub extern "C" fn mshv_register_ioevent(
    vmfd: RawFd,
    rawfd: RawFd,
    is_mmio: bool,
    addr: u64,
    val: u64,
    is_64bit: bool,
    is_datamatch: bool,
) -> bool {
    let ioaddr: IoEventAddress = if is_mmio {
        IoEventAddress::Mmio(addr)
    } else {
        IoEventAddress::Pio(addr)
    };
    let fd = unsafe { EventFd::from_raw_fd(rawfd) };

    let datamatch = if is_datamatch {
        let d = if is_64bit {
            DataMatch::DataMatch64(val)
        } else {
            DataMatch::DataMatch32(val as u32)
        };
        Some(d)
    } else {
        None
    };
    get_vminfo(vmfd)
        .vm
        .register_ioevent(&fd, &ioaddr, datamatch)
        .expect("failed to register");
    mem::forget(fd);
    true
}

// is_mmio: is_mmio or is_pio
#[no_mangle]
pub extern "C" fn mshv_unregister_ioevent(
    vmfd: RawFd,
    rawfd: RawFd,
    is_mmio: bool,
    addr: u64,
) -> bool {
    let fd = unsafe { EventFd::from_raw_fd(rawfd) };
    let ioaddr: IoEventAddress = if is_mmio {
        IoEventAddress::Mmio(addr)
    } else {
        IoEventAddress::Pio(addr)
    };
    get_vminfo(vmfd)
        .vm
        .unregister_ioevent(&fd, &ioaddr)
        .expect("failed to unregister");
    mem::forget(fd);
    true
}

// interrupt_type: Fixed, low-priority, SMI, NMI, IPI, etc.
#[no_mangle]
pub extern "C" fn mshv_request_interrupt(
    vmfd: RawFd,
    interrupt_type: u32,
    vector: u32,
    vp_index: u32,
    logic_dest_mode: bool,
    level_triggered: bool,
) {
    let vm = get_vminfo(vmfd).vm.clone();
    let cfg = InterruptRequest {
        interrupt_type,
        apic_id: vp_index as u64,
        level_triggered: level_triggered,
        vector: vector,
        logical_destination_mode: logic_dest_mode,
        long_mode: false,
    };
    let vm: &MshvVm = vm.as_any().downcast_ref().unwrap();
    if vector > 0 {
        vm.fd()
            .request_virtual_interrupt(&cfg)
            .expect("failed request");
    }
}
