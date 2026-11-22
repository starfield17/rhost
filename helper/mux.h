#ifndef RHOST_MUX_H
#define RHOST_MUX_H
#include "protocol.h"
#define RH_FOLLOW 11u
#define RH_DATA 12u
#define RH_COMPLETION 13u
#define RH_CANCEL 14u
#define RH_FILES 15u
#define RH_TCP_OPEN 16u
#define RH_TCP_DATA 17u
#define RH_TCP_CLOSE 18u
#define RH_MUX_HEADER 20u
#define RH_REPLY UINT32_C(0x80000000)
void rh_mux_header(unsigned char header[RH_MUX_HEADER],uint64_t id,uint32_t kind,size_t size);
int rh_mux(void);
int rh_mux_active(void);
int rh_mux_cancelled(void);
uint64_t rh_mux_reply_id(void);
int rh_follow(rh_reader *request);
int rh_mux_files(rh_reader *request);
#endif
