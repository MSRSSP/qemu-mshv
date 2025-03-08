QEMU=../qemu/build/qemu-system-x86_64
## sudo is required for qemu-ifup.
sudo ${QEMU} \
	-kernel hypervisor-fw \
	-trace "mshv_*" -trace file=trace.log \
	-drive file=./focal-server-cloudimg-amd64.raw,format=raw,id=os,if=none \
	-drive file=./ubuntu-cloudinit.img,format=raw,if=none,id=data \
	--nographic -m 512M \
	-device virtio-blk-pci,drive=os,disable-legacy=on \
	-device virtio-blk-pci,drive=data,disable-legacy=on \
	-machine q35,accel=mshv -smp cpus=8 \
	-netdev tap,id=net0,ifname=tap1,script=./qemu-ifup,downscript=./qemu-ifdown \
	-device virtio-net-pci,netdev=net0,mac=12:34:56:78:90:ab\
	-rtc clock=vm \
	2> qemu.err
