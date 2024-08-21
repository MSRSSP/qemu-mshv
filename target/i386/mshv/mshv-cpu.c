#include "qemu/osdep.h"
#include "cpu.h"
#include "sysemu/mshv.h"

static void set_seg(struct SegmentRegister *lhs, const SegmentCache *rhs)
{
    unsigned flags = rhs->flags;
    lhs->selector = rhs->selector;
    lhs->base = rhs->base;
    lhs->limit = rhs->limit;
    lhs->type_ = (flags >> DESC_TYPE_SHIFT) & 15;
    lhs->present = (flags & DESC_P_MASK) != 0;
    lhs->dpl = (flags >> DESC_DPL_SHIFT) & 3;
    lhs->db = (flags >> DESC_B_SHIFT) & 1;
    lhs->s = (flags & DESC_S_MASK) != 0;
    lhs->l = (flags >> DESC_L_SHIFT) & 1;
    lhs->g = (flags & DESC_G_MASK) != 0;
    lhs->avl = (flags & DESC_AVL_MASK) != 0;
    lhs->unusable = !lhs->present;
    lhs->padding = 0;
}

static void get_seg(SegmentCache *lhs, const struct SegmentRegister *rhs)
{
    lhs->selector = rhs->selector;
    lhs->base = rhs->base;
    lhs->limit = rhs->limit;
    lhs->flags = (rhs->type_ << DESC_TYPE_SHIFT) |
                 ((rhs->present && !rhs->unusable) * DESC_P_MASK) |
                 (rhs->dpl << DESC_DPL_SHIFT) | (rhs->db << DESC_B_SHIFT) |
                 (rhs->s * DESC_S_MASK) | (rhs->l << DESC_L_SHIFT) |
                 (rhs->g * DESC_G_MASK) | (rhs->avl * DESC_AVL_MASK);
}

static void mshv_getset_seg(struct SegmentRegister *mshv_seg,
                            SegmentCache *qemu_s, int set)
{
    if (set) {
        set_seg(mshv_seg, (const struct SegmentCache *)qemu_s);
    } else {
        get_seg(qemu_s, (const struct SegmentRegister *)mshv_seg);
    }
}

static void mshv_getput_reg(uint64_t *mshv_reg, target_ulong *qemu_reg, int set)
{
    if (set) {
        *mshv_reg = *qemu_reg;
    } else {
        *qemu_reg = *mshv_reg;
    }
}

static int mshv_getput_regs(MshvState *mshv_state, CPUState *cpu, bool set)
{
    X86CPU *x86cpu = X86_CPU(cpu);
    CPUX86State *env = &x86cpu->env;
    StandardRegisters regs;
    SpecialRegisters sregs;
    FloatingPointUnit fpu;
    int ret = 0;

    if (!set) {
        mshv_get_vcpu(mshv_vcpufd(cpu), &regs, &sregs, &fpu);
    }

    mshv_getput_reg(&regs.rax, &env->regs[R_EAX], set);
    mshv_getput_reg(&regs.rbx, &env->regs[R_EBX], set);
    mshv_getput_reg(&regs.rcx, &env->regs[R_ECX], set);
    mshv_getput_reg(&regs.rdx, &env->regs[R_EDX], set);
    mshv_getput_reg(&regs.rsi, &env->regs[R_ESI], set);
    mshv_getput_reg(&regs.rdi, &env->regs[R_EDI], set);
    mshv_getput_reg(&regs.rsp, &env->regs[R_ESP], set);
    mshv_getput_reg(&regs.rbp, &env->regs[R_EBP], set);
    mshv_getput_reg(&regs.rflags, &env->eflags, set);
    mshv_getput_reg(&regs.rip, &env->eip, set);

    mshv_getset_seg(&sregs.cs, &env->segs[R_CS], set);
    mshv_getset_seg(&sregs.ds, &env->segs[R_DS], set);
    mshv_getset_seg(&sregs.es, &env->segs[R_ES], set);
    mshv_getset_seg(&sregs.fs, &env->segs[R_FS], set);
    mshv_getset_seg(&sregs.gs, &env->segs[R_GS], set);
    mshv_getset_seg(&sregs.ss, &env->segs[R_SS], set);

    sregs.idt.limit = env->idt.limit;
    sregs.idt.base = env->idt.base;
    sregs.gdt.limit = env->gdt.limit;
    sregs.gdt.base = env->gdt.base;

    mshv_getput_reg(&sregs.cr0, &env->cr[0], set);
    mshv_getput_reg(&sregs.cr2, &env->cr[2], set);
    mshv_getput_reg(&sregs.cr3, &env->cr[3], set);
    mshv_getput_reg(&sregs.cr4, &env->cr[4], set);

    mshv_getput_reg(&sregs.efer, &env->efer, set);

    if (set) {
        sregs.cr8 = cpu_get_apic_tpr(x86cpu->apic_state);
        sregs.apic_base = cpu_get_apic_base(x86cpu->apic_state);
        memset(&sregs.interrupt_bitmap, 0, sizeof(sregs.interrupt_bitmap));
        memset(&fpu, 0, sizeof(fpu));
        mshv_configure_vcpu(
            mshv_vcpufd(cpu), cpu->cpu_index,
            IS_AMD_CPU(env) ? AMD : (IS_INTEL_CPU(env) ? Intel : Unknown),
            env->nr_dies, cpu->nr_cores / env->nr_dies, cpu->nr_threads, &regs,
            &sregs, env->xcr0, &fpu);
    } else {
        cpu_set_apic_tpr(x86cpu->apic_state, sregs.cr8);
        cpu_set_apic_base(x86cpu->apic_state, sregs.apic_base);
    }

    return ret;
}

#define MSR_ENTRIES_COUNT 64

struct MsrList {
    msr_entry entries[MSR_ENTRIES_COUNT];
    uint32_t nmsrs;
};

static struct msr_entry *mshv_msr_entry_add(struct MsrList *msrs,
                                            uint32_t index, uint64_t value)
{
    struct msr_entry *entry = &msrs->entries[msrs->nmsrs];

    assert(msrs->nmsrs < MSR_ENTRIES_COUNT);

    entry->index = index;
    entry->reserved = 0;
    entry->data = value;
    msrs->nmsrs++;

    return entry;
}

static int mshv_put_msrs(CPUState *cpu)
{
    X86CPU *x86cpu = X86_CPU(cpu);
    CPUX86State *env = &x86cpu->env;
    struct MsrList *msrs = g_malloc0(sizeof(struct MsrList));
    mshv_msr_entry_add(msrs, MSR_IA32_SYSENTER_CS, env->sysenter_cs);
    mshv_msr_entry_add(msrs, MSR_IA32_SYSENTER_ESP, env->sysenter_esp);
    mshv_msr_entry_add(msrs, MSR_IA32_SYSENTER_EIP, env->sysenter_eip);
    mshv_msr_entry_add(msrs, MSR_EFER, env->efer);
    mshv_msr_entry_add(msrs, MSR_PAT, env->pat);
    mshv_msr_entry_add(msrs, MSR_STAR, env->star);
    mshv_msr_entry_add(msrs, MSR_CSTAR, env->cstar);
    mshv_msr_entry_add(msrs, MSR_LSTAR, env->lstar);
    mshv_msr_entry_add(msrs, MSR_KERNELGSBASE, env->kernelgsbase);
    mshv_msr_entry_add(msrs, MSR_FMASK, env->fmask);
    mshv_msr_entry_add(msrs, MSR_MTRRdefType, env->mtrr_deftype);
    mshv_msr_entry_add(msrs, MSR_VM_HSAVE_PA, env->vm_hsave);
    mshv_msr_entry_add(msrs, MSR_SMI_COUNT, env->msr_smi_count);
    mshv_msr_entry_add(msrs, MSR_IA32_PKRS, env->pkrs);
    mshv_msr_entry_add(msrs, MSR_IA32_BNDCFGS, env->msr_bndcfgs);
    mshv_msr_entry_add(msrs, MSR_IA32_XSS, env->xss);
    mshv_msr_entry_add(msrs, MSR_IA32_UMWAIT_CONTROL, env->umwait);
    mshv_msr_entry_add(msrs, MSR_IA32_TSX_CTRL, env->tsx_ctrl);
    mshv_msr_entry_add(msrs, MSR_AMD64_TSC_RATIO, env->amd_tsc_scale_msr);
    mshv_msr_entry_add(msrs, MSR_TSC_AUX, env->tsc_aux);
    mshv_msr_entry_add(msrs, MSR_TSC_ADJUST, env->tsc_adjust);
    mshv_msr_entry_add(msrs, MSR_IA32_SMBASE, env->smbase);
    mshv_msr_entry_add(msrs, MSR_IA32_SPEC_CTRL, env->spec_ctrl);
    mshv_msr_entry_add(msrs, MSR_VIRT_SSBD, env->virt_ssbd);
    mshv_configure_msr(mshv_vcpufd(cpu), &msrs->entries[0], msrs->nmsrs);
    g_free(msrs);

    return 0;
}

int mshv_arch_put_registers(MshvState *mshv_state, CPUState *cpu)
{
    int ret = 0;

    ret = mshv_getput_regs(mshv_state, cpu, true);
    if (ret) {
        return ret;
    }

    ret = mshv_put_msrs(cpu);
    if (ret) {
        return ret;
    }

    return ret;
}

int mshv_arch_get_registers(MshvState *mshv_state, CPUState *cpu)
{
    return mshv_getput_regs(mshv_state, cpu, false);
}
