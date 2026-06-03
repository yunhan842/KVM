/* user/hello.c — freestanding ring-3 program for slices 5-6.
 *
 * Compiled with gcc -ffreestanding -nostdlib (no libc). The write/exit
 * declarations are satisfied by user/crt0.s at link time.
 *
 * Slice 6 additions:
 *   - bench_start / bench_end emit the multi-byte protocol the host's serial
 *     state machine recognizes (ESC 'B' '0' + LE iter count for start;
 *     ESC 'B' '1' for end). See spec §5 for the wire format.
 *   - main runs BENCH_ITERS noop syscalls bracketed by the markers, then
 *     does the existing hello write and exits.
 *
 * BENCH_ITERS lives ONLY here — the host learns the iteration count from
 * the start marker's payload, so there's no cross-language constant drift.
 */

extern long write(int fd, const char *buf, unsigned long len);
extern void exit(int code) __attribute__((noreturn));

#define BENCH_ITERS 10000UL

/* Emit the 11-byte start marker:
 *   0x1B 0x42 0x30 + 8 little-endian bytes of `n`.
 * The host's Uart::handle_data_byte recognizes this sequence, records
 * Instant::now() as bench_start, and stores n as bench_iters. */
static void bench_start(unsigned long n) {
    unsigned char buf[11] = {
        0x1B, 0x42, 0x30,
        (unsigned char)(n      ), (unsigned char)(n >>  8),
        (unsigned char)(n >> 16), (unsigned char)(n >> 24),
        (unsigned char)(n >> 32), (unsigned char)(n >> 40),
        (unsigned char)(n >> 48), (unsigned char)(n >> 56),
    };
    write(1, (const char *)buf, sizeof(buf));
}

/* Emit the 3-byte end marker. */
static void bench_end(void) {
    static const unsigned char buf[3] = { 0x1B, 0x42, 0x31 };
    write(1, (const char *)buf, sizeof(buf));
}

int main(void) {
    bench_start(BENCH_ITERS);

    /* Tight loop of noop syscalls. write(fd, _, 0) takes the full SYSCALL
     * round-trip (entry stub, dispatch, sys_write, return path, sysretq)
     * but from_raw_parts(buf, 0) builds an empty slice and write_bytes
     * loops zero times — measures pure syscall plumbing, not workload. */
    for (unsigned long i = 0; i < BENCH_ITERS; i++) {
        write(1, "", 0);
    }

    bench_end();

    /* Existing slice-5 demo line — still the slice-5 load-bearing assertion. */
    write(1, "[user] hello from C userspace\n", 30);
    return 0;
}
