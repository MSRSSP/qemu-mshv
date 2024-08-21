#include "qemu/osdep.h"
#include "qapi/error.h"
#include "qemu/error-report.h"
#include "qemu/module.h"

#include "exec/address-spaces.h"
#include "hw/i386/x86.h"
#include "qemu/accel.h"
#include "qemu/guest-random.h"
#include "sysemu/cpus.h"
#include "sysemu/runstate.h" //vm_stop
#include "sysemu/accel-blocker.h"
#include "sysemu/mshv.h"
#include "sysemu/reset.h" //register reset
#include "trace.h"

#define TYPE_MSHV_ACCEL ACCEL_CLASS_NAME("mshv")

DECLARE_INSTANCE_CHECKER(MshvState, MSHV_STATE, TYPE_MSHV_ACCEL)

bool mshv_allowed;

MshvState *mshv_state;

static int do_mshv_set_memory(const MshvMemoryRegion *mshv_mr, bool add)
{
    int ret = 0;

    if (!mshv_mr) {
        return -1;
    }

    trace_mshv_set_memory(add, mshv_mr->guest_phys_addr, mshv_mr->memory_size,
                          mshv_mr->userspace_addr, mshv_mr->readonly, ret);
    if (add) {
        ret = mshv_add_mem(mshv_state->vm, mshv_mr);
    }

    if (!add) {
        ret = mshv_remove_mem(mshv_state->vm, mshv_mr);
    }

    return ret;
}

/*
 * Calculate and align the start address and the size of the section.
 * Return the size. If the size is 0, the aligned section is empty.
 */
static hwaddr align_section(MemoryRegionSection *section, hwaddr *start)
{
    hwaddr size = int128_get64(section->size);
    hwaddr delta, aligned;

    /* works in page size chunks, but the function may be called
   with sub-page size and unaligned start address. Pad the start
   address to next and truncate size to previous page boundary. */
    aligned = ROUND_UP(section->offset_within_address_space,
                       qemu_real_host_page_size());
    delta = aligned - section->offset_within_address_space;
    *start = aligned;
    if (delta > size) {
        return 0;
    }

    return (size - delta) & qemu_real_host_page_mask();
}

static int mshv_set_phys_mem(MshvMemoryListener *mml,
                             MemoryRegionSection *section, bool add,
                             const char *name)
{
    int ret = 0;
    MemoryRegion *area = section->mr;
    bool writable = !area->readonly && !area->rom_device;
    hwaddr start_addr, mr_offset, size;
    void *ram;
    MshvMemoryRegion tmp, *mshv_mr = &tmp;

    if (!memory_region_is_ram(area)) {
        if (writable) {
            return ret;
        } else if (!memory_region_is_romd(area)) {
            /*
             * If the memory device is not in romd_mode, then we actually want
             * to remove the memory slot so all accesses will trap.
             */
            add = false;
        }
    }

    size = align_section(section, &start_addr);
    if (!size) {
        return ret;
    }

    mr_offset = section->offset_within_region + start_addr -
                section->offset_within_address_space;

    ram = memory_region_get_ram_ptr(area) + mr_offset;

    memset(mshv_mr, 0, sizeof(*mshv_mr));
    mshv_mr->guest_phys_addr = start_addr;
    mshv_mr->memory_size = size;
    mshv_mr->readonly = !writable;
    mshv_mr->userspace_addr = (uint64_t)ram;

    ret = do_mshv_set_memory(mshv_mr, add);
    if (add && (ret == 17)) {
        // qemu may create a memory alias as rom.
        // However, mshv may not support the overlapped regions.
        // the mshv-qemu shim will handle the overlapped issue.
        return ret;
    }
    assert(ret == 0);
    return ret;
}

static void mshv_region_add(MemoryListener *listener,
                            MemoryRegionSection *section)
{
    MshvMemoryListener *mml =
        container_of(listener, MshvMemoryListener, listener);
    memory_region_ref(section->mr);
    mshv_set_phys_mem(mml, section, true, "add");
}

static void mshv_region_del(MemoryListener *listener,
                            MemoryRegionSection *section)
{
    MshvMemoryListener *mml =
        container_of(listener, MshvMemoryListener, listener);
    mshv_set_phys_mem(mml, section, false, "remove");
    memory_region_unref(section->mr);
}

static void mshv_mem_ioeventfd_add(MemoryListener *listener,
                                   MemoryRegionSection *section,
                                   bool match_data, uint64_t data,
                                   EventNotifier *e)
{
    int fd = event_notifier_get_fd(e);
    int r;
    bool is_64 = int128_get64(section->size) == 8;
    bool is_mmio = true;

    trace_mshv_mem_ioeventfd_add(section->offset_within_address_space,
                                 int128_get64(section->size), data);
    r = mshv_register_ioevent(mshv_state->vm, fd, is_mmio,
                              section->offset_within_address_space & 0xffffffff,
                              data, is_64, match_data);

    if (r < 0) {
        mshv_err("%s: error adding ioeventfd: %s (%d)\n", __func__,
                 strerror(-r), -r);
        abort();
    }
}

static void mshv_mem_ioeventfd_del(MemoryListener *listener,
                                   MemoryRegionSection *section,
                                   bool match_data, uint64_t data,
                                   EventNotifier *e)
{
    int fd = event_notifier_get_fd(e);
    int r = 0;

    trace_mshv_mem_ioeventfd_del(section->offset_within_address_space,
                                 int128_get64(section->size), data);
    r = mshv_unregister_ioevent(mshv_state->vm, fd, true,
                                section->offset_within_address_space &
                                    0xffffffff);

    if (r < 0) {
        mshv_err("%s: error adding ioeventfd: %s (%d)\n", __func__,
                 strerror(-r), -r);
        abort();
    }
}

static MemoryListener mshv_memory_listener = {
    .name = "mshv",
    .priority = MEMORY_LISTENER_PRIORITY_ACCEL,
    .region_add = mshv_region_add,
    .region_del = mshv_region_del,
#ifdef MSHV_USE_IOEVENTFD
    .eventfd_add = mshv_mem_ioeventfd_add,
    .eventfd_del = mshv_mem_ioeventfd_del,
#endif
};

static MemoryListener mshv_io_listener = {
    .name = "mshv", .priority = MEMORY_LISTENER_PRIORITY_DEV_BACKEND,
    // MSHV does not support PIO eventfd
};

static void mshv_memory_listener_register(MshvState *s, MshvMemoryListener *mml,
                                          AddressSpace *as, int as_id,
                                          const char *name)
{
    int i;

    mml->listener = mshv_memory_listener;
    mml->listener.name = name;
    memory_listener_register(&mml->listener, as);
    for (i = 0; i < s->nr_as; ++i) {
        if (!s->as[i].as) {
            s->as[i].as = as;
            s->as[i].ml = mml;
            break;
        }
    }
}

static void mshv_reset(void *param)
{
    fprintf(stderr, "mshv_reset\n");
}

static void mshv_init_irq(MshvState *s) {}

static int mshv_init(MachineState *ms)
{
    MshvState *s;
    uint64_t vm_type;

    s = MSHV_STATE(ms->accelerator);

    accel_blocker_init();

    mshv_new();
    s->vm = 0;

    // TODO: object_property_find(OBJECT(current_machine), "mshv-type")
    vm_type = 0;
    do {
        s->vm = mshv_create_vm_with_type(vm_type);
    } while (!s->vm);

    mshv_vm_resume(s->vm);

    // MAX number of address spaces:
    // address_space_memory
    s->nr_as = 1;
    s->as = g_new0(struct MshvAs, s->nr_as);

    mshv_state = s;

    qemu_register_reset(mshv_reset, NULL);

    mshv_init_irq(s);

    // register memory listener
    mshv_memory_listener_register(s, &s->memory_listener, &address_space_memory,
                                  0, "mshv-memory");
    memory_listener_register(&mshv_io_listener, &address_space_io);
    return 0;
}

static void mshv_accel_class_init(ObjectClass *oc, void *data)
{
    AccelClass *ac = ACCEL_CLASS(oc);

    ac->name = "MSHV";
    ac->init_machine = mshv_init;
    ac->allowed = &mshv_allowed;
}

static void mshv_accel_instance_init(Object *obj)
{
    MshvState *s = MSHV_STATE(obj);

    s->vm = 0;
}

static const TypeInfo mshv_accel_type = {
    .name = TYPE_MSHV_ACCEL,
    .parent = TYPE_ACCEL,
    .instance_init = mshv_accel_instance_init,
    .class_init = mshv_accel_class_init,
    .instance_size = sizeof(MshvState),
};

static inline MemTxAttrs mshv_get_mem_attrs(bool is_secure_mode)
{
    return ((MemTxAttrs){ .secure = is_secure_mode });
}

static void guest_mem_read_fn(uint64_t gpa, uint8_t *data, uintptr_t size,
                              bool is_secure_mode)
{
    int ret = 0;
    MemTxAttrs memattr = mshv_get_mem_attrs(is_secure_mode);
    ret = address_space_rw(&address_space_memory, gpa, memattr, (void *)data,
                           size, false);
    assert(ret == MEMTX_OK);
}

static int guest_mem_write_fn(uint64_t gpa, const uint8_t *data, uintptr_t size,
                              bool is_secure_mode)
{
    int ret = 0;
    MemTxAttrs memattr = mshv_get_mem_attrs(is_secure_mode);
    ret = address_space_rw(&address_space_memory, gpa, memattr, (void *)data,
                           size, true);
    return ret;
}

static void mmio_read_fn(uint64_t gpa, uint8_t *data, uintptr_t size,
                         bool is_secure_mode)
{
    int ret = 0;
    MemTxAttrs memattr = mshv_get_mem_attrs(is_secure_mode);
    ret = address_space_rw(&address_space_memory, gpa, memattr, (void *)data,
                           size, false);
    assert(ret == MEMTX_OK);
}

static int mmio_write_fn(uint64_t gpa, const uint8_t *data, uintptr_t size,
                         bool is_secure_mode)
{
    int ret = 0;
    MemTxAttrs memattr = mshv_get_mem_attrs(is_secure_mode);
    ret = address_space_rw(&address_space_memory, gpa, memattr, (void *)data,
                           size, true);
    return ret;
}

static void pio_read_fn(uint64_t port, uint8_t *data, uintptr_t size,
                        bool is_secure_mode)
{
    int ret = 0;
    MemTxAttrs memattr = mshv_get_mem_attrs(is_secure_mode);
    ret = address_space_rw(&address_space_io, port, memattr, (void *)data, size,
                           false);
    assert(ret == MEMTX_OK);
}

static int pio_write_fn(uint64_t port, const uint8_t *data, uintptr_t size,
                        bool is_secure_mode)
{
    int ret = 0;
    MemTxAttrs memattr = mshv_get_mem_attrs(is_secure_mode);
    ret = address_space_rw(&address_space_io, port, memattr, (void *)data, size,
                           true);
    return ret;
}

static int mshv_init_vcpu(CPUState *cpu)
{
    MshvOps mshv_ops = {
        .guest_mem_write_fn = guest_mem_write_fn,
        .guest_mem_read_fn = guest_mem_read_fn,
        .mmio_read_fn = mmio_read_fn,
        .mmio_write_fn = mmio_write_fn,
        .pio_read_fn = pio_read_fn,
        .pio_write_fn = pio_write_fn,
    };
    cpu->accel = g_new0(AccelCPUState, 1);
    mshv_vcpufd(cpu) = mshv_new_vcpu(mshv_state->vm, cpu->cpu_index, &mshv_ops);
    cpu->vcpu_dirty = false;

    return 0;
}

static int mshv_destroy_vcpu(CPUState *cpu)
{
    mshv_remove_vcpu(mshv_vcpufd(cpu));
    mshv_vcpufd(cpu) = 0;
    g_free(cpu->accel);
    return 0;
}

static int mshv_cpu_exec(CPUState *cpu)
{
    hv_message mshv_msg;
    MshvVmExit exit_reason;
    int ret = 0;

    bql_unlock();
    cpu_exec_start(cpu);

    do {
        if (cpu->vcpu_dirty) {
            ret = mshv_arch_put_registers(mshv_state, cpu);
            if (ret) {
                cpu->vcpu_dirty = false;
            }
        }

        if (qatomic_read(&cpu->exit_request)) {
            qemu_cpu_kick_self();
        }

        /* Read cpu->exit_request before KVM_RUN reads run->immediate_exit.
         * Matching barrier in kvm_eat_signals.
         */
        smp_rmb();

        exit_reason = mshv_run_vcpu(mshv_vcpufd(cpu), &mshv_msg);

        switch (exit_reason) {
        case Ignore:
            break;
        default:
            ret = EXCP_INTERRUPT;
            break;
        }
    } while (ret == 0);

    cpu_exec_end(cpu);
    bql_lock();

    if (ret < 0) {
        cpu_dump_state(cpu, stderr, CPU_DUMP_CODE);
        vm_stop(RUN_STATE_INTERNAL_ERROR);
    }

    qatomic_set(&cpu->exit_request, 0);
    return ret;
}

// The signal handler is triggered when QEMU's main thread receives a SIG_IPI
// (SIGUSR1). This signal causes the current CPU thread to be kicked, forcing a
// VM exit on the CPU. The VM exit generates an exit reason that breaks the loop
// (see mshv_cpu_exec). If the exit is due to a Ctrl+A+x command, the system
// will shut down. For other cases, the system will continue running.
static void sa_ipi_handler(int sig)
{
    qemu_cpu_kick_self();
}

static void mshv_init_signal(CPUState *cpu)
{
    /* init cpu signals */
    struct sigaction sigact;
    sigset_t set;

    memset(&sigact, 0, sizeof(sigact));
    sigact.sa_handler = sa_ipi_handler;
    sigaction(SIG_IPI, &sigact, NULL);

    pthread_sigmask(SIG_BLOCK, NULL, &set);
    sigdelset(&set, SIG_IPI);
    pthread_sigmask(SIG_SETMASK, &set, NULL);
}

static void *mshv_vcpu_thread_fn(void *arg)
{
    CPUState *cpu = arg;

    rcu_register_thread();

    bql_lock();
    qemu_thread_get_self(cpu->thread);
    cpu->thread_id = qemu_get_thread_id();
    current_cpu = cpu;
    mshv_init_vcpu(cpu);
    mshv_init_signal(cpu);

    /* signal CPU creation */
    cpu_thread_signal_created(cpu);
    qemu_guest_random_seed_thread_part2(cpu->random_seed);

    do {
        if (cpu_can_run(cpu)) {
            mshv_cpu_exec(cpu);
        }
        qemu_wait_io_event(cpu);
    } while (!cpu->unplug || cpu_can_run(cpu));
    mshv_destroy_vcpu(cpu);
    cpu_thread_signal_destroyed(cpu);
    bql_unlock();
    rcu_unregister_thread();
    return NULL;
}

static void mshv_start_vcpu_thread(CPUState *cpu)
{
    char thread_name[VCPU_THREAD_NAME_SIZE];

    cpu->thread = g_malloc0(sizeof(QemuThread));
    cpu->halt_cond = g_malloc0(sizeof(QemuCond));

    qemu_cond_init(cpu->halt_cond);

    trace_mshv_start_vcpu_thread(thread_name, cpu->cpu_index);
    qemu_thread_create(cpu->thread, thread_name, mshv_vcpu_thread_fn, cpu,
                       QEMU_THREAD_JOINABLE);
}

static void mshv_cpu_synchronize_post_init(CPUState *cpu)
{
    mshv_arch_put_registers(mshv_state, cpu);

    cpu->vcpu_dirty = false;
}

static void do_mshv_cpu_synchronize_pre_loadvm(CPUState *cpu,
                                               run_on_cpu_data arg)
{
    cpu->vcpu_dirty = true;
}

static void mshv_cpu_synchronize_pre_loadvm(CPUState *cpu)
{
    run_on_cpu(cpu, do_mshv_cpu_synchronize_pre_loadvm, RUN_ON_CPU_NULL);
}

static void do_mshv_cpu_synchronize(CPUState *cpu, run_on_cpu_data arg)
{
    mshv_arch_get_registers(mshv_state, cpu);
}

static void mshv_cpu_synchronize(CPUState *cpu)
{
    run_on_cpu(cpu, do_mshv_cpu_synchronize, RUN_ON_CPU_NULL);
}

static bool mshv_cpus_are_resettable(void)
{
    return false;
}

static void mshv_accel_ops_class_init(ObjectClass *oc, void *data)
{
    AccelOpsClass *ops = ACCEL_OPS_CLASS(oc);

    ops->create_vcpu_thread = mshv_start_vcpu_thread;
    ops->synchronize_post_init = mshv_cpu_synchronize_post_init;
    ops->synchronize_state = mshv_cpu_synchronize;
    ops->synchronize_pre_loadvm = mshv_cpu_synchronize_pre_loadvm;
    ops->cpus_are_resettable = mshv_cpus_are_resettable;
}

static const TypeInfo mshv_accel_ops_type = {
    .name = ACCEL_OPS_NAME("mshv"),
    .parent = TYPE_ACCEL_OPS,
    .class_init = mshv_accel_ops_class_init,
    .abstract = true,
};

static void mshv_type_init(void)
{
    type_register_static(&mshv_accel_type);
    type_register_static(&mshv_accel_ops_type);
}

type_init(mshv_type_init);