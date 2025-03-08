ch=`which cloud-hypervisor`
sudo setcap cap_net_admin+ep $ch
$ch --disk path=focal-server-cloudimg-amd64.raw path=./ubuntu-cloudinit.img \
	--kernel hypervisor-fw \
	--cpus boot=1 \
	--memory size=1024M \
	--serial tty --seccomp false \
	--console off --net "mac=12:34:56:78:90:ab,tap="
