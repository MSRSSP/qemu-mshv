#ifndef QEMU_MSHV_INT_H
#define QEMU_MSHV_INT_H

#include "exec/memory.h"
#include "hw/hyperv/hyperv-proto.h"
#include "qapi/qapi-types-common.h"
#include "qemu/accel.h"
#include "qemu/log.h"
#include "qemu/queue.h"

#ifdef COMPILING_PER_TARGET
#ifdef CONFIG_MSHV
#define CONFIG_MSHV_IS_POSSIBLE
#endif
#else
#define CONFIG_MSHV_IS_POSSIBLE
#endif

typedef struct hyperv_message hv_message;

// Set to 0 if we do not want to use eventfd to optimize the MMIO events.
// Set to 1 so that mshv kernel driver receives doorbell when the VM access
// MMIO memory and then signal eventfd to notify the qemu device
// without extra switching to qemu to emulate mmio access.
#define MSHV_USE_IOEVENTFD 1

#define MSHV_USE_KERNEL_GSI_IRQFD 1

#define mshv_err(FMT, ...)                                                     \
  do {                                                                         \
    fprintf(stderr, FMT, ##__VA_ARGS__);                                       \
  } while (0)

#ifdef DEBUG_MSHV
#define mshv_debug()                                                           \
  do {                                                                         \
    fprintf(stderr, "%s:%d\n", __func__, __LINE__);                            \
  } while (0)

#else
#define mshv_debug()
#endif

#ifdef CONFIG_MSHV_IS_POSSIBLE
#include <qemu-mshv.h>
extern bool mshv_allowed;
#define mshv_enabled() (mshv_allowed)

typedef struct MshvMemoryListener {
  MemoryListener listener;
  int as_id;
} MshvMemoryListener;

typedef struct MshvState {
  AccelState parent_obj;
  int vm;
  MshvMemoryListener memory_listener;
  int nr_as; // number of listener;
  struct MshvAs {
    MshvMemoryListener *ml;
    AddressSpace *as;
  } * as;
} MshvState;
extern MshvState *mshv_state;

struct AccelCPUState {
  int cpufd;
};

#define mshv_vcpufd(cpu) (cpu->accel->cpufd)

int mshv_arch_put_registers(MshvState *s, CPUState *cpu);

int mshv_arch_get_registers(MshvState *s, CPUState *cpu);

#else //! CONFIG_MSHV_IS_POSSIBLE
#define mshv_enabled() false
#endif
#ifdef MSHV_USE_KERNEL_GSI_IRQFD
#define mshv_msi_via_irqfd_enabled() mshv_enabled()
#else
#define mshv_msi_via_irqfd_enabled() false
#endif

int mshv_irqchip_add_msi_route(int vector, PCIDevice *dev);
int mshv_irqchip_update_msi_route(int virq, MSIMessage msg, PCIDevice *dev);
void mshv_irqchip_commit_routes(void);
void mshv_irqchip_release_virq(int virq);
int mshv_irqchip_add_irqfd_notifier_gsi(EventNotifier *n, EventNotifier *rn,
                                        int virq);
int mshv_irqchip_remove_irqfd_notifier_gsi(EventNotifier *n, int virq);

#endif
