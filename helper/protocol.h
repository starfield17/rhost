#ifndef RHOST_PROTOCOL_H
#define RHOST_PROTOCOL_H
#ifndef _DARWIN_C_SOURCE
#define _DARWIN_C_SOURCE 1
#endif
#ifndef _DEFAULT_SOURCE
#define _DEFAULT_SOURCE 1
#endif
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif
#include <stddef.h>
#include <stdint.h>
#define RH_LIMIT (8u * 1024u * 1024u)
typedef struct { unsigned char *data; size_t size, capacity; int failed; } rh_buf;
typedef struct { const unsigned char *data; size_t size, at; int failed; } rh_reader;
uint32_t rh_u32(const unsigned char *p);
uint64_t rh_u64(const unsigned char *p);
void rh_put32(unsigned char *p, uint32_t n);
void rh_byte(rh_buf *b, unsigned n);
void rh_int(rh_buf *b, uint32_t n);
void rh_long(rh_buf *b, uint64_t n);
void rh_bytes(rh_buf *b, const void *p, size_t n);
void rh_text(rh_buf *b, const char *s);
void rh_raw(rh_buf *b, const void *p, size_t n);
unsigned rh_read_byte(rh_reader *r);
uint32_t rh_read_int(rh_reader *r);
uint64_t rh_read_long(rh_reader *r);
unsigned char *rh_read_bytes(rh_reader *r, size_t *n);
char *rh_read_text(rh_reader *r);
int rh_done(const rh_reader *r);
int rh_emit(uint32_t kind, const rh_buf *body);
int rh_refuse(uint32_t kind, const char *code, const char *message);
void rh_sha256(const void *data, size_t size, unsigned char out[32]);
typedef struct {uint32_t h[8];uint64_t size;unsigned char pending[64];size_t used;} rh_hash;
void rh_hash_init(rh_hash *h);
void rh_hash_update(rh_hash *h,const void *data,size_t size);
void rh_hash_finish(rh_hash *h,unsigned char out[32]);
void rh_hex(const unsigned char *p, size_t n, char *out);
int rh_utf8(const unsigned char *p,size_t n,int allow_nul);
#endif
