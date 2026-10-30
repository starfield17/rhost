#ifndef RHOST_REGISTRY_H
#define RHOST_REGISTRY_H
#include "state.h"
#include "files.h"
#define RG_RECORDS 16384u
#define RG_BYTES UINT64_C(8589934592)
#define RG_QUERIES 32u
#define RG_RUN_RESERVE (UINT64_C(145)*1024*1024)
#define RG_CHANGE_RESERVE (UINT64_C(41)*1024*1024)
#define RG_QUERY_RESERVE (UINT64_C(10)*1024*1024)
typedef struct {uint64_t logical,reserved,records,queries;int complete;} rg_usage;
typedef struct {
    char id[80],status[32];uint64_t expiry,bytes;unsigned phase;int active;
    const char *error;
} rg_record;
int rg_kind(const char *name);
const char *rg_parent(int kind);
const char *rg_label(int kind);
int rg_base(rh_state *s,int kind,int create);
int rg_private(int fd);
int rg_identity(const char *name,char id[80],uint64_t *expiry,unsigned *phase);
int rg_observe(int base,const char *name,int kind,const char *generation,rg_record *record);
int rg_account(rh_state *s,rg_usage *usage);
const char *rg_capacity(rh_state *s,int kind);
int rg_remove(rh_state *s,int kind,const char *id,int directory,f_error *e);
j_value *rg_list(rh_state *s,int kind,const j_value *q,f_error *e);
j_value *rg_prune(rh_state *s,const j_value *q,f_error *e);
j_value *rg_json(const j_value *q,f_error *e);
#endif
