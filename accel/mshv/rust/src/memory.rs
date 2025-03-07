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

use std::{os::fd::RawFd, sync::Arc};

use hypervisor::{HypervisorVmError, Vm};
use mshv_ioctls::MshvError;

use crate::{debug, vm::get_vminfo};

#[repr(C)]
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct MshvMemoryRegion {
    slot: u32,
    guest_phys_addr: u64,
    memory_size: u64,
    userspace_addr: u64,
    readonly: bool,
    log_dirty_pages: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MemEntry {
    pub mr: MshvMemoryRegion,
    pub mapped: bool,
}

pub struct MemManager {
    pub db: Vec<MemEntry>,
}

fn map_or_unmap_mem(
    vm: Arc<dyn Vm>,
    r: &MshvMemoryRegion,
    add: bool,
) -> Result<(), HypervisorVmError> {
    debug!(
        "map_or_unmap_mem {:x} {:x} {:x}, add = {}",
        r.guest_phys_addr, r.memory_size, r.userspace_addr, add
    );
    let mr = vm.make_user_memory_region(
        r.slot,
        r.guest_phys_addr,
        r.memory_size,
        r.userspace_addr,
        r.readonly,
        r.log_dirty_pages,
    );
    let ret = if add {
        vm.create_user_memory_region(mr)
    } else {
        vm.remove_user_memory_region(mr)
    };
    ret.map_err(|e| e.into())
}

impl MemManager {
    pub fn new() -> Self {
        MemManager { db: Vec::new() }
    }

    pub fn find_by_gpa(&self, addr: u64) -> Option<usize> {
        self.db.iter().position(|entry| {
            entry.mr.guest_phys_addr <= addr
                && addr - entry.mr.guest_phys_addr < entry.mr.memory_size
        })
    }

    pub fn map_overlapped_region(&mut self, gpa: u64, vm: Arc<dyn Vm>) -> Option<usize> {
        if let Some(index) = self.find_by_gpa(gpa) {
            let mr = &self.db[index].mr;
            if let Some(overlap_idx) = self.db.iter().enumerate().position(|(i, entry)| {
                (entry.mr.userspace_addr < mr.userspace_addr + mr.memory_size)
                    && (entry.mr.userspace_addr + entry.mr.memory_size > mr.userspace_addr)
                    && entry.mapped
                    && i != index
            }) {
                assert!(index != overlap_idx);
                debug!(
                    "{:x} overlaps with {:x}",
                    self.db[overlap_idx].mr.guest_phys_addr, mr.guest_phys_addr
                );
                let _ = self.try_map_or_unmap_overlapped_mem(vm.clone(), overlap_idx, false);
                let _ = self.try_map_or_unmap_overlapped_mem(vm, index, true);
                return Some(overlap_idx);
            }
        }
        return None;
    }

    pub fn add_del_mem(
        &mut self,
        vm: Arc<dyn Vm>,
        r: MshvMemoryRegion,
        add: bool,
    ) -> Result<(), HypervisorVmError> {
        let found = self.db.iter().position(|e| e.mr == r);
        let mut ret: Result<(), HypervisorVmError> = Ok(());
        match found {
            Some(index) => {
                if !add {
                    if self.db[index].mapped {
                        ret = map_or_unmap_mem(vm, &r, add);
                    }
                    self.db.remove(index);
                } else {
                    self.db.remove(index);
                    ret = map_or_unmap_mem(vm, &r, add);
                    self.db.push(MemEntry {
                        mr: r,
                        mapped: ret.is_ok(),
                    });
                }
            }
            None => {
                if add {
                    ret = map_or_unmap_mem(vm, &r, add);
                    self.db.push(MemEntry {
                        mr: r,
                        mapped: ret.is_ok(),
                    });
                } else {
                    panic!("invalid remove!!")
                }
            }
        }
        ret
    }

    // TODO: avoidable if MSHV driver supports double mapping.
    fn try_map_or_unmap_overlapped_mem(
        &mut self,
        vm: Arc<dyn Vm>,
        index: usize,
        add: bool,
    ) -> Result<(), HypervisorVmError> {
        let ret = map_or_unmap_mem(vm, &self.db[index].mr, add);
        if ret.is_ok() {
            if add {
                self.db[index].mapped = true;
            } else {
                self.db[index].mapped = false;
            }
        } else {
            panic!("ret is {:?}", ret);
        }

        ret
    }
}

// register memory in mshv device
fn mshv_add_del_mem(rawvm: RawFd, r: *const MshvMemoryRegion, add: bool) -> i32 {
    let vminfo = get_vminfo(rawvm).clone();
    let vm = vminfo.vm.clone();
    let mem = vminfo.mem.clone();
    let ret = mem.lock().unwrap().add_del_mem(vm, unsafe { *r }, add);
    let ret = match ret {
        Ok(_) => 0,
        Err(HypervisorVmError::CreateUserMemory(e)) => {
            e.downcast_ref::<MshvError>().unwrap().errno()
        }
        Err(HypervisorVmError::RemoveUserMemory(e)) => {
            e.downcast_ref::<MshvError>().unwrap().errno()
        }
        _ => -1,
    };
    ret
}

#[no_mangle]
pub extern "C" fn mshv_add_mem(rawvm: RawFd, r: *const MshvMemoryRegion) -> i32 {
    mshv_add_del_mem(rawvm, r, true)
}

#[no_mangle]
pub extern "C" fn mshv_remove_mem(rawvm: RawFd, r: *const MshvMemoryRegion) -> i32 {
    mshv_add_del_mem(rawvm, r, false)
}
