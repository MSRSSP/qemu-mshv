/*
 * QEMU MSHV stub
 *
 * Copyright Microsoft, Corp. 2017
 *
 *
 * This work is licensed under the terms of the GNU GPL, version 2 or later.
 * See the COPYING file in the top-level directory.
 *
 */

#include "qemu/osdep.h"
#include "system/mshv.h"
#include "hw/pci/msi.h"

bool mshv_allowed;

int mshv_irqchip_add_msi_route(int vector, PCIDevice *dev)
{
    return -ENOSYS;
}

int mshv_irqchip_update_msi_route(int virq, MSIMessage msg, PCIDevice *dev)
{
    return -ENOSYS;
}

void mshv_irqchip_commit_routes(void) {}

void mshv_irqchip_release_virq(int virq) {}

int mshv_irqchip_add_irqfd_notifier_gsi(EventNotifier *n, EventNotifier *rn,
                                        int virq)
{
    return -ENOSYS;
}

int mshv_irqchip_remove_irqfd_notifier_gsi(EventNotifier *n, int virq)
{
    return -ENOSYS;
}