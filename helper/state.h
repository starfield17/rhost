#ifndef RHOST_STATE_H
#define RHOST_STATE_H
#include "protocol.h"
#include <sys/types.h>
typedef struct { int root, runs, registry; char generation[33]; } rh_state;
typedef struct {
    unsigned state, completion, value, out_open, err_open, incomplete;
    uint64_t out_bytes, err_bytes, out_kept, err_kept;
} rh_status;
int rh_random(char *out,size_t bytes);
int rh_directory(const char *path,int create);
int rh_atomic(int directory,const char *name,const void *data,size_t size);
int rh_file(int directory,const char *name,unsigned char **data,size_t *size,size_t limit);
int rh_state_open(rh_state *s,int create);
void rh_state_close(rh_state *s);
int rh_clock(rh_state *s,uint64_t *now);
int rh_clock_read(rh_state *s,uint64_t *now);
int rh_status_write(int directory,const rh_status *s);
int rh_status_read(int directory,rh_status *s);
int rh_key(const char *key,uint64_t *expiry);
int rh_generation(const char *s);
uint64_t rh_millis(void);
void rh_record(rh_buf *b,const rh_state *s,const char *key,const rh_status *status);
int rh_runs(uint32_t kind,rh_reader *request);
typedef struct {
    char *generation,*key,*cwd,**argv,**env;
    size_t argc,envc,input_size;
    unsigned inherit,login;
    unsigned char *input;
    uint64_t expiry,deadline;
    uint32_t wait;
} rh_start;
int rh_parse_start(rh_reader *r,rh_start *q);
void rh_start_free(rh_start *q);
void rh_monitor(int directory,int lock,rh_start *q);
int rh_install(const char *directory,const char *executable);
#endif
