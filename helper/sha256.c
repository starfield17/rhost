#include "protocol.h"
#include <string.h>
static uint32_t rot(uint32_t x,unsigned n) {return (x>>n)|(x<<(32-n));}
static const uint32_t constants[64]={
0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2};
static void block(uint32_t h[8],const unsigned char *p) {
    uint32_t w[64];
    for(unsigned i=0;i<16;i++) w[i]=rh_u32(p+4*i);
    for(unsigned i=16;i<64;i++) {
        uint32_t a=w[i-15],b=w[i-2];
        w[i]=w[i-16]+(rot(a,7)^rot(a,18)^(a>>3))+w[i-7]+(rot(b,17)^rot(b,19)^(b>>10));
    }
    uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],v=h[7];
    for(unsigned i=0;i<64;i++) {
        uint32_t t=v+(rot(e,6)^rot(e,11)^rot(e,25))+((e&f)^((~e)&g))+constants[i]+w[i];
        uint32_t u=(rot(a,2)^rot(a,13)^rot(a,22))+((a&b)^(a&c)^(b&c));
        v=g;g=f;f=e;e=d+t;d=c;c=b;b=a;a=t+u;
    }
    h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=v;
}
void rh_hash_init(rh_hash *h) {
    const uint32_t initial[8]={0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};
    memcpy(h->h,initial,sizeof initial);h->size=0;h->used=0;
}
void rh_hash_update(rh_hash *h,const void *data,size_t size) {
    const unsigned char *p=data;h->size+=size;
    while(size){size_t n=64-h->used;if(n>size)n=size;memcpy(h->pending+h->used,p,n);h->used+=n;p+=n;size-=n;
        if(h->used==64){block(h->h,h->pending);h->used=0;}}
}
void rh_hash_finish(rh_hash *h,unsigned char out[32]) {
    unsigned char tail[128]={0};size_t n=h->used;if(n)memcpy(tail,h->pending,n);tail[n]=128;
    size_t bytes=n<56?64:128; uint64_t bits=h->size*8;
    rh_put32(tail+bytes-8,(uint32_t)(bits>>32));rh_put32(tail+bytes-4,(uint32_t)bits);
    block(h->h,tail);if(bytes==128) block(h->h,tail+64);
    for(unsigned i=0;i<8;i++) rh_put32(out+4*i,h->h[i]);
}
void rh_sha256(const void *data,size_t size,unsigned char out[32]) {
    rh_hash h;rh_hash_init(&h);rh_hash_update(&h,data,size);rh_hash_finish(&h,out);
}
void rh_hex(const unsigned char *p,size_t n,char *out) {
    static const char hex[]="0123456789abcdef";
    for(size_t i=0;i<n;i++) {out[2*i]=hex[p[i]>>4];out[2*i+1]=hex[p[i]&15];} out[2*n]=0;
}
