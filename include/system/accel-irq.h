/*
 * QEMU IRQ support
 *
 * Copyright Microsoft, Corp. 2017
 *
 *
 * This work is licensed under the terms of the GNU GPL, version 2 or later.
 * See the COPYING file in the top-level directory.
 *
 */
#ifndef ACCEL_IRQ_H
#define ACCEL_IRQ_H
#include "hw/pci/msi.h"
#include "qemu/osdep.h"
#include "system/kvm.h"
#include "system/mshv.h"

static inline bool accel_msi_via_irqfd_enabled(void) {
  return mshv_msi_via_irqfd_enabled() || kvm_msi_via_irqfd_enabled();
}

static inline bool accel_irqchip_is_split(void) {
  return mshv_msi_via_irqfd_enabled() || kvm_irqchip_is_split();
}

int accel_irqchip_add_msi_route(KVMRouteChange *c, int vector, PCIDevice *dev);
int accel_irqchip_update_msi_route(int vector, MSIMessage msg, PCIDevice *dev);
void accel_irqchip_commit_route_changes(KVMRouteChange *c);
void accel_irqchip_commit_routes(void);
void accel_irqchip_release_virq(int virq);
int accel_irqchip_add_irqfd_notifier_gsi(EventNotifier *n, EventNotifier *rn,
                                         int virq);
int accel_irqchip_remove_irqfd_notifier_gsi(EventNotifier *n, int virq);
#endif
