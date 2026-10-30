#ifndef RHOST_CHANGES_H
#define RHOST_CHANGES_H
#include "files.h"
#include "state.h"
#define CH_COUNT 64
/* A record describes historical observations. Recovery must recheck every target. */
typedef struct {j_value *meta,*intent;char *path,*name;int dir,lock;struct stat parent;} ch_entry;
typedef struct {rh_state space;int base,dir,owner;char *id;uint64_t expiry;j_value *manifest,*request;ch_entry entries[CH_COUNT];size_t count;f_error *error;} ch_set;
const char *ch_text(const j_value *v,const char *key);
uint64_t ch_num(const j_value *v,const char *key);
int ch_put(j_value *v,const char *key,j_value *value);
int ch_string(j_value *v,const char *key,const char *text);
int ch_integer(j_value *v,const char *key,uint64_t value);
int ch_bool(j_value *v,const char *key,int value);
j_value *ch_canonical(const j_value *v);
j_value *ch_version(const struct stat *s);
int ch_same(const j_value *version,const struct stat *s,int intent);
int ch_save(ch_set *s);
int ch_archive(ch_set *s,size_t i,const char *prefix,const j_buf *bytes);
int ch_bytes(ch_set *s,size_t i,const char *prefix,j_buf *out,const char *hash);
int ch_open(ch_set *s,const j_value *q,int apply);
int ch_load(ch_set *s);
void ch_close(ch_set *s);
j_value *ch_result(ch_set *s,int active);
int ch_prepare(ch_set *s);
int ch_parents(ch_set *s);
int ch_check(ch_set *s,ch_entry *p,const j_value *version,const char *hash,int missing,int intent);
int ch_reconcile(ch_set *s);
int ch_apply(ch_set *s,int rollback);
int ch_cleanup(ch_set *s);
int ch_prune(ch_set *s,uint64_t now);
int ch_can_prune(int directory,const char *generation,const char *id);
j_value *f_changes(const j_value *q,const char *op,f_error *e);
int f_mode(const j_value *q,mode_t *out,f_error *e);
int f_save(int fd,const void *data,size_t size);
#endif
