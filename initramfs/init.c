/*
 * Minimal /init for moss-kernel x86_64 boot testing.
 *
 * This is the first userspace process. It:
 *   1. Mounts /proc and /sys
 *   2. Opens /dev/console for stdin/stdout/stderr
 *   3. Prints a greeting
 *   4. Loops: reads a line from stdin, echoes it (basic shell)
 *
 * Build:
 *   gcc -static -O2 -o init init.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <errno.h>

static void early_print(const char *msg) {
    write(STDOUT_FILENO, msg, strlen(msg));
}

/* Try to set up standard file descriptors from /dev/console.
 * The kernel should already have set these up, but just in case. */
static void setup_stdio(void) {
    int fd;

    /* If stdin isn't a tty, try opening /dev/console */
    if (isatty(STDIN_FILENO) == 0) {
        fd = open("/dev/console", O_RDWR);
        if (fd >= 0) {
            dup2(fd, STDIN_FILENO);
            dup2(fd, STDOUT_FILENO);
            dup2(fd, STDERR_FILENO);
            if (fd > 2)
                close(fd);
        }
    }
}

static void try_mount(const char *source, const char *target,
                       const char *fstype, unsigned long flags) {
    mkdir(target, 0755);
    if (mount(source, target, fstype, flags, NULL) < 0) {
        /* Not fatal — kernel may not support all filesystems */
        (void)errno;
    }
}

int main(void) {
    setup_stdio();

    early_print("\n");
    early_print("============================================\n");
    early_print("  moss-kernel x86_64 init (Phase 0 test)\n");
    early_print("============================================\n");
    early_print("\n");

    /* Mount essential virtual filesystems */
    try_mount("proc",     "/proc", "proc",     0);
    try_mount("sysfs",    "/sys",  "sysfs",    0);
    try_mount("devtmpfs", "/dev",  "devtmpfs", 0);

    early_print("moss: /init running — type commands (basic echo shell)\n");
    early_print("moss: waiting for input...\n\n");

    /* Simple echo shell loop */
    char buf[256];
    for (;;) {
        early_print("$ ");
        memset(buf, 0, sizeof(buf));

        ssize_t n = read(STDIN_FILENO, buf, sizeof(buf) - 1);
        if (n <= 0) {
            /* EOF or error — wait a bit and retry */
            if (n == 0) {
                /* True EOF — nothing connected to stdin */
                early_print("\nmoss: stdin EOF, halting.\n");
                for (;;) { asm volatile("hlt"); }
            }
            break;
        }

        /* Strip trailing newline */
        if (buf[n - 1] == '\n')
            buf[n - 1] = '\0';

        /* Handle built-in commands */
        if (strcmp(buf, "exit") == 0 || strcmp(buf, "quit") == 0) {
            early_print("moss: goodbye!\n");
            break;
        }

        if (strcmp(buf, "help") == 0) {
            early_print("moss builtins: help, exit, info, halt\n");
            continue;
        }

        if (strcmp(buf, "info") == 0) {
            early_print("kernel: moss-kernel x86_64\n");
            early_print("arch:   x86_64\n");
            early_print("phase:  0 (boot-to-bash)\n");
            /* Try to read kernel version from uname */
            continue;
        }

        if (strcmp(buf, "halt") == 0) {
            early_print("moss: halting CPU\n");
            for (;;) { asm volatile("hlt"); }
        }

        /* Echo unknown input */
        write(STDOUT_FILENO, buf, n);
        write(STDOUT_FILENO, "\n", 1);
    }

    return 0;
}
