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

use hypervisor::mshv::MshvVm;
use hypervisor::{InterruptSourceConfig, IrqRoutingEntry, MsiIrqSourceConfig, Vm};
use vmm_sys_util::eventfd::EventFd;

use std::collections::HashMap;
use std::mem;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Arc;

use crate::debug;
use crate::vm::get_vminfo;

pub const MSHV_MAX_MSI_ROUTES: u32 = 4096;

pub struct MSIControl {
    gsi_routes: HashMap<u32, IrqRoutingEntry>,
    vm: Arc<dyn Vm>,
    max_gsi: u32,
    updated: bool,
}

impl MSIControl {
    pub fn new(vm: Arc<dyn Vm>) -> Self {
        MSIControl {
            gsi_routes: HashMap::new(), // gsi-> fd
            vm,
            max_gsi: MSHV_MAX_MSI_ROUTES,
            updated: false,
        }
    }

    pub fn set_msi_routing(&mut self, gsi: u32, addr: u64, data: u32, devid: u32) {
        assert!(gsi < self.max_gsi);
        let config = MsiIrqSourceConfig {
            high_addr: (addr >> 32) as u32,
            low_addr: addr as u32,
            data,
            devid,
        };
        if self.gsi_routes.contains_key(&gsi) {
            let IrqRoutingEntry::Mshv(r) = self.gsi_routes[&gsi];
            if r.data == data && r.address_hi == config.high_addr && r.address_lo == config.low_addr
            {
                return;
            }
        }
        debug!("register addr {:x}, gsi: {}, data = {:x}", addr, gsi, data);
        let route = self
            .vm
            .make_routing_entry(gsi, &InterruptSourceConfig::MsiIrq(config));
        self.updated = true;
        self.gsi_routes.insert(gsi, route);
    }

    pub fn add_msi_routing(&mut self, addr: u64, data: u32, devid: u32) -> Option<u32> {
        debug!("add_msi_routing addr {:x}, data = {:x}", addr, data);
        for gsi in 0..self.max_gsi {
            if !self.gsi_routes.contains_key(&gsi) {
                self.set_msi_routing(gsi, addr, data, devid);
                return Some(gsi);
            }
        }

        return None;
    }

    pub fn remove_gsi(&mut self, gsi: u32) {
        debug!("remove gsi: {}", gsi);
        self.gsi_routes.remove(&gsi);
        self.updated = true;
    }

    pub fn enable_msi_routing(&mut self) {
        if self.updated {
            let mut routes: Vec<IrqRoutingEntry> = Vec::new();
            for (_, v) in &self.gsi_routes {
                routes.push(*v);
            }
            debug!("mshv_enable_msi_routing\n");

            self.vm
                .set_gsi_routing(&routes)
                .expect("failed to set gsi route");
        }
        self.updated = false;
    }

    pub fn register_irqfd(&mut self, fd: &EventFd, gsi: u32) {
        debug!("register gsi: {}, fd{:#?}", gsi, fd);
        self.vm
            .register_irqfd(fd, gsi)
            .expect("failed to register irqfd");
        assert!(self.gsi_routes.contains_key(&gsi));
        //self.gsi_map.get_mut(&gsi).unwrap().fd  = Some(fd);
    }

    pub fn register_irqfd_with_resample(&mut self, fd: &EventFd, rfd: &EventFd, gsi: u32) {
        debug!("register gsi: {}, fd{:#?}, rfd = {:#?}", gsi, fd, rfd);
        self.vm
            .as_any()
            .downcast_ref::<MshvVm>()
            .unwrap()
            .fd()
            .register_irqfd_with_resample(fd, rfd, gsi)
            .expect("failed to register irqfd");
        assert!(self.gsi_routes.contains_key(&gsi));
    }

    pub fn unregister_irqfd(&mut self, gsi: u32, fd: &EventFd) {
        debug!("unregister gsi: {}, fd{:#?}", gsi, fd);
        self.vm
            .unregister_irqfd(fd, gsi)
            .expect("failed to unregister");
    }
}

#[no_mangle]
pub extern "C" fn mshv_set_msi_gsi_routing(vmfd: RawFd, gsi: u32, addr: u64, data: u32) {
    let msi = get_vminfo(vmfd).msi.clone();
    msi.lock().unwrap().set_msi_routing(gsi, addr, data, 0);
}

#[no_mangle]
pub extern "C" fn mshv_remove_gsi_routing(vmfd: RawFd, gsi: u32) {
    let msi = get_vminfo(vmfd).msi.clone();
    msi.lock().unwrap().remove_gsi(gsi);
}

#[no_mangle]
pub extern "C" fn mshv_add_msi_gsi_routing(vmfd: RawFd, addr: u64, data: u32) -> u32 {
    let msi = get_vminfo(vmfd).msi.clone();
    let gsi = msi.lock().unwrap().add_msi_routing(addr, data, 0).unwrap();
    gsi
}

#[no_mangle]
pub extern "C" fn mshv_enable_msi_routing(vmfd: RawFd) {
    let msi = get_vminfo(vmfd).msi.clone();
    msi.lock().unwrap().enable_msi_routing();
}

#[no_mangle]
pub extern "C" fn mshv_register_irqfd(vmfd: RawFd, fd: RawFd, gsi: u32) {
    let efd = unsafe { EventFd::from_raw_fd(fd) };
    let msi = get_vminfo(vmfd).msi.clone();
    let mut x = msi.lock().unwrap();
    x.register_irqfd(&efd, gsi);
    mem::forget(efd);
}

#[no_mangle]
pub extern "C" fn mshv_register_irqfd_with_resample(vmfd: RawFd, fd: RawFd, rfd: RawFd, gsi: u32) {
    let efd = unsafe { EventFd::from_raw_fd(fd) };
    let refd = unsafe { EventFd::from_raw_fd(rfd) };
    let msi = get_vminfo(vmfd).msi.clone();
    let mut x = msi.lock().unwrap();
    x.register_irqfd_with_resample(&efd, &refd, gsi);
    mem::forget(efd);
    mem::forget(refd);
}

#[no_mangle]
pub extern "C" fn mshv_unregister_irqfd(vmfd: RawFd, fd: RawFd, gsi: u32) {
    let efd = unsafe { EventFd::from_raw_fd(fd) };
    let msi = get_vminfo(vmfd).msi.clone();
    let mut x = msi.lock().unwrap();
    x.unregister_irqfd(gsi, &efd);
    mem::forget(efd);
}
