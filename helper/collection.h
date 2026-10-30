#ifndef RHOST_COLLECTION_H
#define RHOST_COLLECTION_H
#include "inspect.h"
#include "registry.h"
typedef struct {
    int directory,owner,paths,index;char generation[33],id[80],fingerprint[65];
    uint64_t expiry,size,count,offset,position;char *root;j_value *meta;
} i_collection;
int i_fingerprint(const j_value *q,const char *root,char hash[65],f_error *e);
void i_path_digest(const i_collection *c,uint64_t position,uint64_t offset,const char *path,unsigned char digest[32]);
int i_collection_open(i_collection *c,const j_value *q,const char *root,f_error *e);
void i_collection_close(i_collection *c);
/* Returns one path and advances a validated byte offset; NULL at end or error. */
char *i_collection_next(i_collection *c,f_error *e);
int i_collection_metadata(i_collection *c,i_tree *tree,const char *status,f_error *e);
int i_collection_build(i_collection *c,const j_value *q,int root,f_error *e);
#endif
