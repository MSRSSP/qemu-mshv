#include "qemu/osdep.h"
#include "hw/pci/msi.h"
#include "sysemu/mshv.h"

int mshv_irqchip_add_msi_route(int vector, PCIDevice *dev)
{
    MSIMessage msg = { 0, 0 };
    int virq = 0;

    if (pci_available && dev) {
        msg = pci_get_msi_message(dev, vector);

        virq = mshv_add_msi_gsi_routing(mshv_state->vm, msg.address,
                                        le32_to_cpu(msg.data));
    }

    return virq;
}

void mshv_irqchip_release_virq(int virq)
{
    mshv_remove_gsi_routing(mshv_state->vm, virq);
}

int mshv_irqchip_update_msi_route(int virq, MSIMessage msg, PCIDevice *dev)
{
    mshv_set_msi_gsi_routing(mshv_state->vm, virq, msg.address,
                             le32_to_cpu(msg.data));
    return 0;
}

void mshv_irqchip_commit_routes(void)
{
    mshv_enable_msi_routing(mshv_state->vm);
}

static int mshv_irqchip_update_irqfd_notifier_gsi(MshvState *s,
                                                  EventNotifier *event,
                                                  EventNotifier *resample,
                                                  int virq, bool add)
{
    int fd = event_notifier_get_fd(event);
    int rfd = resample ? event_notifier_get_fd(resample) : -1;

    if (add) {
        if (rfd > 0) {
            mshv_register_irqfd_with_resample(s->vm, fd, rfd, virq);
        } else {
            mshv_register_irqfd(s->vm, fd, virq);
        }
    } else {
        mshv_unregister_irqfd(s->vm, fd, virq);
    }

    return 0;
}

int mshv_irqchip_add_irqfd_notifier_gsi(EventNotifier *n, EventNotifier *rn,
                                        int virq)
{
    return mshv_irqchip_update_irqfd_notifier_gsi(mshv_state, n, rn, virq,
                                                  true);
}

int mshv_irqchip_remove_irqfd_notifier_gsi(EventNotifier *n, int virq)
{
    return mshv_irqchip_update_irqfd_notifier_gsi(mshv_state, n, NULL, virq,
                                                  false);
}
