/* user/hello.c — freestanding ring-3 program for slice 5.
 *
 * Compiled with gcc -ffreestanding -nostdlib (no libc). The write/exit
 * declarations are satisfied by user/crt0.s at link time.
 */

extern long write(int fd, const char *buf, unsigned long len);
extern void exit(int code) __attribute__((noreturn));

int main(void) {
    write(1, "[user] hello from C userspace\n", 30);
    return 0;
}
