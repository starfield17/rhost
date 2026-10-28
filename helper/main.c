/* Standalone C99 helper: bounded requests and remote-owned process evidence. */
#define _POSIX_C_SOURCE 200809L
#include "state.h"
#include "json.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef RHOST_SOURCE_DIGEST
#error RHOST_SOURCE_DIGEST must identify the helper build inputs
#endif

#if defined(__linux__)
#define TARGET_OS "linux"
#elif defined(__APPLE__)
#define TARGET_OS "darwin"
#elif defined(__FreeBSD__)
#define TARGET_OS "freebsd"
#elif defined(__OpenBSD__)
#define TARGET_OS "openbsd"
#elif defined(__NetBSD__)
#define TARGET_OS "netbsd"
#else
#error unsupported helper target OS
#endif
#if defined(__aarch64__) || defined(__arm64__)
#define TARGET_ARCH "aarch64"
#elif defined(__x86_64__)
#define TARGET_ARCH "x86_64"
#else
#define TARGET_ARCH "other"
#endif

#define BODY_LIMIT (8u * 1024u * 1024u)

static uint32_t u32(const unsigned char *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
           ((uint32_t)p[2] << 8) | (uint32_t)p[3];
}
static uint64_t u64(const unsigned char *p) {
    return ((uint64_t)u32(p) << 32) | u32(p + 4);
}
static void put32(unsigned char *p, uint32_t n) {
    p[0] = (unsigned char)(n >> 24); p[1] = (unsigned char)(n >> 16);
    p[2] = (unsigned char)(n >> 8); p[3] = (unsigned char)n;
}
static int emit(uint32_t kind, const unsigned char *body, size_t size) {
    unsigned char header[16] = {'R', 'H', '0', '1'};
    if (size > BODY_LIMIT) return 255;
    put32(header + 4, kind | UINT32_C(0x80000000));
    put32(header + 8, 0); put32(header + 12, (uint32_t)size);
    if (fwrite(header, 1, sizeof header, stdout) != sizeof header ||
        fwrite(body, 1, size, stdout) != size || fflush(stdout) != 0) return 255;
    return 0;
}
static size_t append_text(unsigned char *out, const char *text) {
    size_t size = strlen(text);
    put32(out, (uint32_t)size);
    memcpy(out + 4, text, size);
    return size + 4;
}
static int refuse(uint32_t kind, const char *code, const char *message) {
    unsigned char body[512];
    size_t size = 1;
    body[0] = 1;
    /* All callers supply bounded compile-time literals, never input text. */
    size += append_text(body + size, code);
    size += append_text(body + size, message);
    body[size++] = 0;
    return emit(kind, body, size);
}
static int probe(uint32_t kind) {
    unsigned char body[256];
    size_t size = 1;
    const char *digest = RHOST_SOURCE_DIGEST;
    if (strlen(digest) != 64) return refuse(kind, "HELPER_INCOMPATIBLE", "invalid build digest");
    for (size_t i = 0; i < 64; ++i) {
        if (!((digest[i] >= '0' && digest[i] <= '9') ||
              (digest[i] >= 'a' && digest[i] <= 'f')))
            return refuse(kind, "HELPER_INCOMPATIBLE", "invalid build digest");
    }
    body[0] = 0;
    size += append_text(body + size, digest);
    size += append_text(body + size, TARGET_OS);
    size += append_text(body + size, TARGET_ARCH);
    put32(body + size, 7); size += 4;
    char boot[37];(void)rh_boot_id(boot);size+=append_text(body+size,boot);
    const uint32_t limits[]={8388608,12582912,8388608,67108864,1048576};
    for(size_t i=0;i<sizeof limits/sizeof limits[0];i++){put32(body+size,limits[i]);size+=4;}
    return emit(kind, body, size);
}
int main(int argc,char **argv) {
    if(argc==2 && !strcmp(argv[1],"--files-json"))return rh_files_json();
    if(argc==4 && !strcmp(argv[1],"--install-cache")) {
        if(strcmp(argv[3],RHOST_SOURCE_DIGEST))return refuse(0,"HELPER_INCOMPATIBLE","candidate source identity does not match");
        return rh_install(argv[2],argv[0]);
    }
    if(argc==3 && !strcmp(argv[1],"--install-cache"))return rh_install(argv[2],argv[0]);
    if(argc!=1)return refuse(0,"PROTOCOL_ERROR","invalid helper invocation");
    unsigned char header[16];
    unsigned char *body = NULL;
    uint32_t kind = 0;
    uint64_t size;
    size_t got = fread(header, 1, sizeof header, stdin);
    if (got >= 8) kind = u32(header + 4);
    if (got != sizeof header || memcmp(header, "RH01", 4) != 0)
        return refuse(kind, "PROTOCOL_ERROR", "invalid or incomplete header");
    size = u64(header + 8);
    if (size > BODY_LIMIT)
        return refuse(kind, "PROTOCOL_ERROR", "request exceeds frame limit");
    if (kind == 0 || (kind & UINT32_C(0x80000000)) != 0)
        return refuse(kind, "PROTOCOL_ERROR", "invalid request kind");
    if (kind > 8)
        return refuse(kind, "PROTOCOL_ERROR", "unknown request kind");
    if (size != 0) {
        body = malloc((size_t)size);
        if (body == NULL) return refuse(kind, "STATE_UNAVAILABLE", "allocation failed");
        if (fread(body, 1, (size_t)size, stdin) != size) {
            free(body);
            return refuse(kind, "PROTOCOL_ERROR", "incomplete body");
        }
    }
    if (fgetc(stdin) != EOF || ferror(stdin)) {
        free(body);
        return refuse(kind, "PROTOCOL_ERROR", "trailing or unreadable input");
    }
    if (kind == 1) {
        free(body);
        if (size != 0) return refuse(kind, "PROTOCOL_ERROR", "probe body must be empty");
        return probe(kind);
    }
    rh_reader request={body,(size_t)size,0,0};
    int result=rh_runs(kind,&request);free(body);return result;
}
