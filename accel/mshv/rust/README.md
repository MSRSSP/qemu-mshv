# QEMU-MSHV

This is a experimental PoC to run qemu-mshv using Rust libraries. Since
Cloud-hypervisor already supports MSHV, we existing crates from CH and related
projects.

This is going to be replaced by pure C by implementing emulator and directly
interacting with MSHV device via ioctl.

## Build qemu with mshv support

```
cd qemu
mkdir build
cd build
../configure --target-list=x86_64-softmmu --enable-mshv
make -j
```

You should see
```
Targets and accelerators
    MSHV support                                 : YES
```

## Run qemu

1. Download images: `scripts/download-image.sh`
2. Run `scripts/run-qemu.sh`
