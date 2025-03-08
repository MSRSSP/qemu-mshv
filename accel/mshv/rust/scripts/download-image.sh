## dnf install mtools dosfstools
script_path=$(realpath "$0")
rootdir=`dirname $script_path`/..
targetdir=${rootdir}/scripts
#baseimage="ubuntu-20.04-minimal-cloudimg-amd64"
#imageurl="https://cloud-images.ubuntu.com/minimal/releases/focal/release/${baseimage}.img"
baseimage="focal-server-cloudimg-amd64"
imageurl="https://cloud-images.ubuntu.com/focal/current/${baseimage}.img"
image=${targetdir}/${baseimage}
if [ ! -f ${image}.img ]; then
wget  ${imageurl}
fi
if [ ! -f ${image}.raw ]; then
qemu-img convert -p -f qcow2 -O raw ${image}.img ${image}.raw
fi
if [ ! -f ${targetdir}/hypervisor-fw ]; then
wget https://github.com/cloud-hypervisor/rust-hypervisor-firmware/releases/download/0.4.2/hypervisor-fw
fi
if [ ! -f ${targetdir}/ubuntu-cloudinit.img ]; then
rm -f ${targetdir}/ubuntu-cloudinit.img
mkdosfs -n CIDATA -C ${targetdir}/ubuntu-cloudinit.img 8192
mcopy -oi ${targetdir}/ubuntu-cloudinit.img -s ${targetdir}/local/user-data ::
mcopy -oi ${targetdir}/ubuntu-cloudinit.img -s ${targetdir}/local/meta-data ::
mcopy -oi ${targetdir}/ubuntu-cloudinit.img -s ${targetdir}/local/network-config ::
fi
