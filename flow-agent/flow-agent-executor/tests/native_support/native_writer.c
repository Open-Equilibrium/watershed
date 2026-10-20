/* The finite writer from probe-macos-self-protection.py, also used by npm. */
#include <errno.h>
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc != 3) return 70;
    FILE *output = fopen(argv[1], "wb");
    if (!output) return errno == EACCES || errno == EPERM || errno == EROFS ? 10 : 71;
    size_t length = strlen(argv[2]);
    if (fwrite(argv[2], 1, length, output) != length) return 72;
    return fclose(output) == 0 ? 0 : 73;
}
