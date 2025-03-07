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

use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};
use std::panic;
use std::sync::{Arc, RwLock};

use anyhow::anyhow;
use hypervisor::arch::x86::emulator::{CpuStateManager, Emulator};
use hypervisor::arch::x86::regs::DF;
use hypervisor::arch::{emulator::PlatformEmulator, x86::emulator::EmulatorCpuState};
use hypervisor::mshv::MshvHypervisor;
use hypervisor::mshv::{emulator::MshvEmulatorContext, ExtendedControlRegisters, MshvVcpu};
use hypervisor::{CpuVendor, HypervisorCpuError, Vcpu, VmOps};
use mshv_bindings::{
    hv_message, hv_register_assoc, hv_register_name_HV_X64_REGISTER_RAX,
    hv_register_name_HV_X64_REGISTER_RDI, hv_register_name_HV_X64_REGISTER_RIP,
    hv_register_name_HV_X64_REGISTER_RSI, hv_register_value, hv_x64_io_port_intercept_message,
    msr_entry, FloatingPointUnit, Msrs, SpecialRegisters, StandardRegisters,
};

use mshv_ioctls::{set_registers_64, Mshv, MshvError};

use crate::debug;
use crate::vm::{get_vminfo, MshvOps, PerVmInfo};

use lazy_static::lazy_static;

lazy_static! {
    static ref CPU_DB: RwLock<Option<HashMap<RawFd, Arc<PerCpuInfo>>>> = RwLock::new(None);
}

pub fn get_vcpuinfo(cpufd: RawFd) -> Arc<PerCpuInfo> {
    CPU_DB.read().unwrap().as_ref().unwrap()[&cpufd].clone()
}

pub fn init_cpudb() {
    *CPU_DB.write().unwrap() = Some(HashMap::new());
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub enum MshvCpuVendor {
    #[default]
    Unknown,
    Intel,
    AMD,
}

impl From<MshvCpuVendor> for CpuVendor {
    fn from(c: MshvCpuVendor) -> Self {
        match c {
            MshvCpuVendor::Unknown => CpuVendor::Unknown,
            MshvCpuVendor::Intel => CpuVendor::Intel,
            MshvCpuVendor::AMD => CpuVendor::AMD,
        }
    }
}

pub struct PerCpuInfo {
    pub vcpu: Arc<dyn Vcpu>,
    pub vp_index: u8,
    pub vm_ops: Option<Arc<dyn VmOps>>,
    pub vminfo: Arc<PerVmInfo>,
}

#[repr(C)]
#[derive(Debug)]
pub enum MshvVmExit {
    Ignore = 0,
    Shutdown = 1,
    Special = 2,
}

#[no_mangle]
pub extern "C" fn mshv_new_vcpu(vm: RawFd, vp_index: u8, mshv_ops: *const MshvOps) -> RawFd {
    let mshv_ops = unsafe { *mshv_ops };
    let mshv_ops = Arc::new(mshv_ops);
    let vmops: Arc<dyn hypervisor::VmOps> = mshv_ops.clone();
    let vminfo = get_vminfo(vm);
    let vm = vminfo.vm.clone();
    let vcpu = vm.create_vcpu(vp_index, Some(vmops.clone())).unwrap();
    let tmp = vcpu.clone();
    let mshv_vcpu: &MshvVcpu = tmp.as_any().downcast_ref().unwrap();
    let vcpufd = mshv_vcpu.fd();
    let fd = vcpufd.as_raw_fd();
    CPU_DB.write().unwrap().as_mut().unwrap().insert(
        fd,
        Arc::new(PerCpuInfo {
            vcpu,
            vp_index,
            vm_ops: Some(vmops),
            vminfo,
        }),
    );
    fd
}

fn configure_and_check_msr(vcpu: Arc<dyn Vcpu>, msrs: &[msr_entry]) {
    let supported_msrs = get_supported_msr();
    let mut msrs = msrs.to_owned();
    msrs.retain(|e| supported_msrs.binary_search(&e.index).is_ok());

    let mshv_vcpu: &MshvVcpu = vcpu.as_any().downcast_ref().unwrap();
    mshv_vcpu
        .fd()
        .set_msrs(&Msrs::from_entries(&msrs).unwrap())
        .expect(format!("failed to set msr {:?}", msrs).as_str());
}

#[no_mangle]
pub extern "C" fn mshv_configure_msr(cpufd: RawFd, raw_msrs: *const msr_entry, len: u32) {
    let raw_msrs: &[msr_entry] = unsafe { std::slice::from_raw_parts(raw_msrs, len as usize) };
    let vcpu = get_vcpuinfo(cpufd).vcpu.clone();
    configure_and_check_msr(vcpu, raw_msrs);
}

fn get_supported_msr() -> Vec<u32> {
    let msrs = Mshv::new().unwrap().get_msr_index_list().unwrap();
    let mut ret = msrs.as_slice().to_owned();
    ret.sort();
    ret
}

#[no_mangle]
pub extern "C" fn mshv_get_vcpu(
    cpufd: RawFd,
    raw_regs: *mut StandardRegisters,
    raw_sregs: *mut SpecialRegisters,
    raw_fpu: *mut FloatingPointUnit,
) {
    let vcpu = get_vcpuinfo(cpufd).vcpu.clone();
    let mshv_vcpu: &MshvVcpu = vcpu.as_any().downcast_ref().unwrap();
    unsafe {
        *raw_regs = mshv_vcpu.fd().get_regs().expect("failed to get regs");
        *raw_sregs = mshv_vcpu.fd().get_sregs().expect("failed to get sregs");
        *raw_fpu = mshv_vcpu.fd().get_fpu().expect("failed to get fpu");
    }
}

#[no_mangle]
pub extern "C" fn mshv_set_vcpu(
    cpufd: RawFd,
    raw_regs: *const StandardRegisters,
    raw_sregs: *const SpecialRegisters,
    raw_fpu: *const FloatingPointUnit,
    xcr0: u64,
) {
    let vcpu = get_vcpuinfo(cpufd).vcpu.clone();
    let mshv_vcpu: &MshvVcpu = vcpu.as_any().downcast_ref().unwrap();
    mshv_vcpu
        .fd()
        .set_regs(unsafe { &*raw_regs })
        .expect("failed to set regs");
    mshv_vcpu
        .fd()
        .set_fpu(unsafe { &*raw_fpu })
        .expect("failed to set fpu");
    mshv_vcpu
        .fd()
        .set_sregs(unsafe { &*raw_sregs })
        .expect("failed to set sregs");
    mshv_vcpu
        .fd()
        .set_xcrs(&ExtendedControlRegisters { xcr0 })
        .expect("failed to set xcr0");
}

// config CPU in mshvcpu device
// topolohy: threads_per_core, cores_per_die, packages
#[no_mangle]
pub extern "C" fn mshv_configure_vcpu(
    cpufd: RawFd,
    id: u8,
    cpu_vendor: MshvCpuVendor,
    die: u8,
    ncore_per_die: u8,
    thread_per_core: u8,
    raw_regs: *const StandardRegisters,
    raw_sregs: *const SpecialRegisters,
    xcr0: u64,
    raw_fpu: *const FloatingPointUnit,
) {
    let vcpu = get_vcpuinfo(cpufd).vcpu.clone();
    let mshv = MshvHypervisor::new().unwrap();
    let phys_bits = arch::get_host_cpu_phys_bits(&mshv);
    let config = arch::CpuidConfig {
        sgx_epc_sections: None,
        phys_bits,
        kvm_hyperv: false,
        amx: false,
    };
    let cpuid = arch::generate_common_cpuid(&mshv, &config).unwrap();
    // Let mshv to fill cpuid;
    arch::configure_vcpu(
        &vcpu,
        id,
        None,
        cpuid,
        false,
        cpu_vendor.into(),
        Some((thread_per_core, ncore_per_die, die)),
    )
    .expect("Failed to configure cpuid\n");

    arch::x86_64::interrupts::set_lint(&(vcpu.clone() as Arc<dyn Vcpu>))
        .expect("failed to set interrupt");

    mshv_set_vcpu(cpufd, raw_regs, raw_sregs, raw_fpu, xcr0);
}

#[no_mangle]
pub extern "C" fn mshv_run_vcpu(cpufd: RawFd, raw_hv_message: *mut hv_message) -> MshvVmExit {
    let cpuinfo = get_vcpuinfo(cpufd);
    let vcpu = cpuinfo.vcpu.clone();
    let mshv_vcpu: &MshvVcpu = vcpu.as_any().downcast_ref().unwrap();
    let ret = mshv_vcpu.fd().run();
    let vp_index = cpuinfo.vp_index as usize;
    let vminfo = cpuinfo.vminfo.clone();
    let ret = match ret {
        Ok(x) => match x.header.message_type {
            mshv_bindings::hv_message_type_HVMSG_UNRECOVERABLE_EXCEPTION => {
                unsafe {
                    *(raw_hv_message as *mut hv_message) = x;
                }
                MshvVmExit::Shutdown
            }
            mshv_bindings::hv_message_type_HVMSG_UNMAPPED_GPA => {
                mshv_handle_unmapped_mem(&x, mshv_vcpu, &vminfo, vp_index)
            }
            mshv_bindings::hv_message_type_HVMSG_GPA_INTERCEPT => {
                mshv_handle_intercept_gpa(&x, mshv_vcpu, vp_index)
            }
            mshv_bindings::hv_message_type_HVMSG_X64_IO_PORT_INTERCEPT => {
                let ret = mshv_handle_pio(&x, mshv_vcpu, cpuinfo.vm_ops.clone());
                match ret {
                    Ok(_) => MshvVmExit::Ignore,
                    _ => MshvVmExit::Special,
                }
            }
            _ => {
                unsafe {
                    *(raw_hv_message as *mut hv_message) = x;
                }
                MshvVmExit::Special
            }
        },
        Err(_) => MshvVmExit::Shutdown,
    };
    ret
}

#[no_mangle]
pub extern "C" fn mshv_nmi(cpufd: RawFd) -> i32 {
    let cpu = get_vcpuinfo(cpufd).vcpu.clone();
    match cpu.nmi() {
        Ok(_) => 0,
        Err(HypervisorCpuError::Nmi(e)) => e.downcast_ref::<MshvError>().unwrap().errno(),
        _ => {
            unimplemented!()
        }
    }
}

#[no_mangle]
pub extern "C" fn mshv_remove_vcpu(cpufd: RawFd) {
    let _ = CPU_DB
        .write()
        .unwrap()
        .as_mut()
        .unwrap()
        .remove(&cpufd)
        .unwrap();
}

fn emulate(
    vp_index: usize,
    vcpu: &MshvVcpu,
    gva: u64,
    gpa: u64,
    instruction_bytes: &[u8],
) -> Result<(), anyhow::Error> {
    // Create a new emulator.
    let mut context = MshvEmulatorContext {
        vcpu,
        map: (gva, gpa),
    };
    let mut emul = Emulator::new(&mut context);

    // Emulate the trapped instruction, and only the first one.
    let new_state = match emul.emulate_first_insn(vp_index, instruction_bytes) {
        Ok(s) => s,
        Err(e) => {
            panic!("{:#?}", e);
        }
    };

    // Set CPU state back.
    context
        .set_cpu_state(vp_index, new_state)
        .map_err(|e| anyhow!(e))?;
    Ok(())
}

#[cfg(debug_assertions)]
fn show_decode(insn_bytes: &[u8]) -> iced_x86::Instruction {
    let mut decoder = iced_x86::Decoder::new(64, insn_bytes, iced_x86::DecoderOptions::NONE);
    let mut insn = iced_x86::Instruction::default();
    if decoder.can_decode() {
        decoder.decode_out(&mut insn);
        debug!(
            "insn = {}, code = {:?}, last_error ={:#?}",
            insn,
            insn.code(),
            decoder.last_error()
        );
    }
    return insn;
}

fn mshv_handle_mmio(msg: &hv_message, vcpu: &MshvVcpu, vp_index: usize) -> MshvVmExit {
    let info = msg.to_memory_info().unwrap();
    let insn_len = info.instruction_byte_count as usize;
    let acces_type = info.header.intercept_access_type;
    let gva = info.guest_virtual_address;
    let gpa = info.guest_physical_address;

    if acces_type == 2 {
        // This should not be reacheable.
        unimplemented!();
    }

    assert!(0 < insn_len && insn_len <= 16);

    #[cfg(debug_assertions)]
    if insn_len != 16 {
        debug!(
            "cross page boundary: access = {} insn_len = {} {:#?}",
            acces_type,
            insn_len,
            show_decode(&info.instruction_bytes[..insn_len])
        );
    }

    emulate(
        vp_index,
        vcpu,
        gva,
        gpa,
        &info.instruction_bytes[..insn_len],
    )
    .expect("MMIO emulation failed");
    MshvVmExit::Ignore
}

fn mshv_handle_intercept_gpa(msg: &hv_message, vcpu: &MshvVcpu, vp_index: usize) -> MshvVmExit {
    mshv_handle_mmio(msg, vcpu, vp_index)
}

fn mshv_handle_unmapped_mem(
    msg: &hv_message,
    vcpu: &MshvVcpu,
    vminfo: &PerVmInfo,
    vp_index: usize,
) -> MshvVmExit {
    let info = msg.to_memory_info().unwrap();
    let gpa = info.guest_physical_address;

    let mem = vminfo.mem.clone();
    let vm = vminfo.vm.clone();
    if mem.lock().unwrap().find_by_gpa(gpa).is_none() {
        return mshv_handle_mmio(msg, vcpu, vp_index);
    }
    if mem.lock().unwrap().map_overlapped_region(gpa, vm).is_none() {
        MshvVmExit::Special
    } else {
        MshvVmExit::Ignore
    }
}

// Resturn rax or Error
fn mshv_handle_pio_non_str(
    info: &mut hv_x64_io_port_intercept_message,
    vcpu: &MshvVcpu,
    vmops: Option<Arc<dyn VmOps>>,
) -> Result<(), anyhow::Error> {
    // SAFETY: access_info is valid, otherwise we won't be here
    let len = unsafe { info.access_info.__bindgen_anon_1.access_size() } as usize;
    let is_write = info.header.intercept_access_type == 1;
    let port = info.port_number;

    if is_write {
        let data = (info.rax as u32).to_le_bytes();
        if let Some(vm_ops) = vmops {
            vm_ops
                .pio_write(port.into(), &data[0..len])
                .map_err(|e| anyhow!(e))?;
        }
    } else {
        let mut data: [u8; 4] = [0; 4];
        if let Some(vm_ops) = vmops {
            vm_ops
                .pio_read(port.into(), &mut data[0..len])
                .map_err(|e| anyhow!(e))?;
        }

        let v = u32::from_le_bytes(data);
        /* Preserve high bits in EAX but clear out high bits in RAX */
        let mask = 0xffffffff >> (32 - len * 8);
        let eax = (info.rax as u32 & !mask) | (v & mask);
        info.rax = eax as u64;
    }

    let insn_len = info.header.instruction_length() as u64;

    /* Advance RIP and update RAX */
    let arr_reg_name_value = [
        (
            hv_register_name_HV_X64_REGISTER_RIP,
            info.header.rip + insn_len,
        ),
        (hv_register_name_HV_X64_REGISTER_RAX, info.rax),
    ];

    set_registers_64!(vcpu.fd(), arr_reg_name_value)
        .map_err(|e| HypervisorCpuError::SetRegister(e.into()))
        .unwrap();
    Ok(())
}

// Resturn rax or Error
fn mshv_handle_pio_str(
    info: &mut hv_x64_io_port_intercept_message,
    vcpu: &MshvVcpu,
    vmops: Option<Arc<dyn VmOps>>,
) -> Result<(), anyhow::Error> {
    let access_info = info.access_info;
    // SAFETY: access_info is valid, otherwise we won't be here
    let len = unsafe { access_info.__bindgen_anon_1.access_size() } as usize;
    let is_write = info.header.intercept_access_type == 1;
    let port = info.port_number;
    let repop = unsafe { access_info.__bindgen_anon_1.rep_prefix() == 1 };
    let repeat = if repop { info.rcx } else { 1 } as usize;
    let insn_len = info.header.instruction_length() as u64;

    let mut data: [u8; 4] = [0; 4];
    let mut context = MshvEmulatorContext {
        vcpu: &vcpu,
        map: (0, 0),
    };

    let emu_cpu = EmulatorCpuState {
        regs: vcpu.get_regs()?,
        sregs: vcpu.get_sregs()?,
    };

    debug!(
        "str port {} access {} len {} bytelen {} insnlen {} repeat {}",
        info.port_number as u64,
        info.header.intercept_access_type,
        len,
        info.instruction_byte_count,
        insn_len,
        repeat
    );

    let df = (emu_cpu.flags() & DF) != 0;

    let arr_reg_name_value = if is_write {
        let mut src = emu_cpu
            .linearize(iced_x86::Register::DS, info.rsi, true)
            .map_err(|e| anyhow!(e))?;

        for _ in 0..repeat {
            // Confirm it is not cross-page.
            // TODO: remove this if the emulator support cross-page access.
            assert!((src + len as u64) % 0x1000 == src);
            let _ = context.read_memory(src, &mut data[0..len]);
            if let Some(vm_ops) = &vmops {
                vm_ops
                    .pio_write(port.into(), &data[0..len])
                    .map_err(|e| anyhow!(e))?;
            }
            if df {
                src = src.wrapping_sub(len as u64);
                info.rsi = info.rsi.wrapping_sub(len as u64);
            } else {
                src = src.wrapping_add(len as u64);
                info.rsi = info.rsi.wrapping_add(len as u64);
            }
        }
        [
            (
                hv_register_name_HV_X64_REGISTER_RIP,
                info.header.rip + insn_len,
            ),
            (hv_register_name_HV_X64_REGISTER_RAX, info.rax),
            (hv_register_name_HV_X64_REGISTER_RSI, info.rsi),
        ]
    } else {
        let mut dst = emu_cpu
            .linearize(iced_x86::Register::ES, info.rdi, true)
            .map_err(|e| anyhow!(e))?;
        for _ in 0..repeat {
            if let Some(vm_ops) = &vmops {
                vm_ops
                    .pio_read(port.into(), &mut data[0..len])
                    .map_err(|e| anyhow!(e))?;
            }
            context
                .write_memory(dst, &mut data[0..len])
                .expect("failed to write mem\n");
            if df {
                dst = dst.wrapping_sub(len as u64);
                info.rdi = info.rdi.wrapping_sub(len as u64);
            } else {
                dst = dst.wrapping_add(len as u64);
                info.rdi = info.rdi.wrapping_add(len as u64);
            }
        }
        [
            (
                hv_register_name_HV_X64_REGISTER_RIP,
                info.header.rip + insn_len,
            ),
            (hv_register_name_HV_X64_REGISTER_RAX, info.rax),
            (hv_register_name_HV_X64_REGISTER_RDI, info.rdi),
        ]
    };

    set_registers_64!(vcpu.fd(), arr_reg_name_value).map_err(|e| anyhow::anyhow!(e))?;
    /* Advance RIP and update RAX */

    Ok(())
}

fn mshv_handle_pio(
    msg: &hv_message,
    vcpu: &MshvVcpu,
    vmops: Option<Arc<dyn VmOps>>,
) -> Result<(), anyhow::Error> {
    let mut info = msg.to_ioport_info().unwrap();
    let strop = unsafe { info.access_info.__bindgen_anon_1.string_op() == 1 };

    if !strop {
        mshv_handle_pio_non_str(&mut info, vcpu, vmops)?
    } else {
        mshv_handle_pio_str(&mut info, vcpu, vmops)?
    }
    Ok(())
}
