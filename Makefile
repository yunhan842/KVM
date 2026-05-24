# Builds guest.img = [4 KiB boot stub | Rust kernel] for `minikvm run guest.img`.
KERNEL_ELF := target/x86_64-unknown-none/debug/kernel

.PHONY: all clean kernel
all: guest.img

build:
	mkdir -p build

build/boot.bin: guest/boot.asm | build
	nasm -f bin guest/boot.asm -o build/boot.bin

kernel: | build
	cd crates/kernel && cargo build
	objcopy -O binary $(KERNEL_ELF) build/kernel.bin

guest.img: build/boot.bin kernel
	cat build/boot.bin build/kernel.bin > guest.img

clean:
	rm -rf build guest.img
