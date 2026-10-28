#include "protocol.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
uint32_t rh_u32(const unsigned char *p) {
    return ((uint32_t)p[0]<<24)|((uint32_t)p[1]<<16)|((uint32_t)p[2]<<8)|p[3];
}
uint64_t rh_u64(const unsigned char *p) { return ((uint64_t)rh_u32(p)<<32)|rh_u32(p+4); }
void rh_put32(unsigned char *p, uint32_t n) {
    p[0]=(unsigned char)(n>>24); p[1]=(unsigned char)(n>>16);
    p[2]=(unsigned char)(n>>8); p[3]=(unsigned char)n;
}
void rh_raw(rh_buf *b, const void *p, size_t n) {
    if (b->failed || n>RH_LIMIT || b->size>RH_LIMIT-n) { b->failed=1; return; }
    if (b->size+n>b->capacity) {
        size_t capacity=b->size+n;
        unsigned char *next=realloc(b->data, capacity?capacity:1);
        if (!next) { b->failed=1; return; }
        b->data=next; b->capacity=capacity;
    }
    if (n) memcpy(b->data+b->size,p,n);
    b->size+=n;
}
void rh_byte(rh_buf *b,unsigned n) { unsigned char v=(unsigned char)n; rh_raw(b,&v,1); }
void rh_int(rh_buf *b,uint32_t n) { unsigned char v[4]; rh_put32(v,n); rh_raw(b,v,4); }
void rh_long(rh_buf *b,uint64_t n) { rh_int(b,(uint32_t)(n>>32)); rh_int(b,(uint32_t)n); }
void rh_bytes(rh_buf *b,const void *p,size_t n) {
    if (n>UINT32_MAX) { b->failed=1; return; }
    rh_int(b,(uint32_t)n); rh_raw(b,p,n);
}
void rh_text(rh_buf *b,const char *s) { rh_bytes(b,s,strlen(s)); }
static int take(rh_reader *r,size_t n) {
    if (r->failed || n>r->size-r->at) { r->failed=1; return 0; } return 1;
}
unsigned rh_read_byte(rh_reader *r) { if (!take(r,1)) return 0; return r->data[r->at++]; }
uint32_t rh_read_int(rh_reader *r) {
    if (!take(r,4)) return 0;
    uint32_t n=rh_u32(r->data+r->at); r->at+=4; return n;
}
uint64_t rh_read_long(rh_reader *r) {
    uint64_t hi=rh_read_int(r),lo=rh_read_int(r); return (hi<<32)|lo;
}
unsigned char *rh_read_bytes(rh_reader *r,size_t *n) {
    *n=rh_read_int(r); if (!take(r,*n)) return NULL;
    unsigned char *p=malloc(*n+1);
    if (!p) { r->failed=1; return NULL; }
    if (*n) memcpy(p,r->data+r->at,*n);
    p[*n]=0; r->at+=*n; return p;
}
int rh_utf8(const unsigned char *p,size_t n,int allow_nul) {
    size_t i=0;
    while(i<n) {
        unsigned a=p[i++],v,more,min;
        if(a<128) { if(!a && !allow_nul) return 0; continue; }
        if(a>=194 && a<=223) {more=1;v=a&31;min=128;}
        else if(a>=224 && a<=239) {more=2;v=a&15;min=2048;}
        else if(a>=240 && a<=244) {more=3;v=a&7;min=65536;}
        else return 0;
        if(more>n-i) return 0;
        while(more--) { unsigned c=p[i++]; if((c&192)!=128) return 0; v=(v<<6)|(c&63); }
        if(v<min || v>1114111 || (v>=55296 && v<=57343)) return 0;
    }
    return 1;
}
char *rh_read_text(rh_reader *r) {
    size_t n; unsigned char *p=rh_read_bytes(r,&n);
    if(p && !rh_utf8(p,n,0)) {free(p);p=NULL;r->failed=1;} return (char *)p;
}
int rh_done(const rh_reader *r) { return !r->failed && r->at==r->size; }
int rh_emit(uint32_t kind,const rh_buf *body) {
    unsigned char h[16]={'R','H','0','1'};
    if(body->failed || body->size>RH_LIMIT) return 255;
    rh_put32(h+4,kind|UINT32_C(0x80000000)); rh_put32(h+8,0); rh_put32(h+12,(uint32_t)body->size);
    if(fwrite(h,1,16,stdout)!=16 || fwrite(body->data,1,body->size,stdout)!=body->size || fflush(stdout)) return 255;
    return 0;
}
int rh_refuse(uint32_t kind,const char *code,const char *message) {
    rh_buf b={0}; rh_byte(&b,1); rh_text(&b,code); rh_text(&b,message); rh_byte(&b,0);
    int result=rh_emit(kind,&b); free(b.data); return result;
}
