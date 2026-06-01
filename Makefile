# Builds guest.img = [4 KiB boot stub | Rust kernel] for `minikvm run guest.img`.
KERNEL_ELF := target/x86_64-unknown-none/debug/kernel

# Freestanding C toolchain for the ring-3 user program (slice 5).
# Each flag is documented in docs/superpowers/specs/2026-05-31-minikvm-month3-c-userspace-design.md §8.
CC := gcc
USER_CFLAGS  := -ffreestanding -nostdlib -fno-pic -fno-pie -mno-red-zone \
                -fno-stack-protector -fno-stack-clash-protection \
                -fcf-protection=none -O1
USER_LDFLAGS := -nostartfiles -static -T user/link.ld \
                -Wl,-z,max-page-size=0x1000 \
                -Wl,-z,noexecstack \
                -Wl,--build-id=none

.PHONY: all clean kernel
all: guest.img

build:
	mkdir -p build

build/boot.bin: guest/boot.asm | build
	nasm -f bin guest/boot.asm -o build/boot.bin

# Ring-3 program. Link order: crt0.s BEFORE hello.c so _start is first in
# .text and the ELF entry point lands exactly at 0x800000.
build/hello.elf: user/crt0.s user/hello.c user/link.ld | build
	$(CC) $(USER_CFLAGS) $(USER_LDFLAGS) \
	    user/crt0.s user/hello.c -o build/hello.elf

# kernel depends on build/hello.elf so cargo rebuilds the kernel when the
# user program changes (the kernel embeds build/hello.elf via include_bytes!).
kernel: build/hello.elf | build
	cd crates/kernel && cargo build
	objcopy -O binary $(KERNEL_ELF) build/kernel.bin

guest.img: build/boot.bin kernel
	cat build/boot.bin build/kernel.bin > guest.img

clean:
	rm -rf build guest.img
