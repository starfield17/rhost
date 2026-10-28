#ifndef RHOST_INSPECT_H
#define RHOST_INSPECT_H
#include "files.h"
#define I_COUNT 4096
#define I_SCAN (64u*1024u*1024u)
typedef struct {char *paths[I_COUNT];size_t count,visited;unsigned reasons[6];j_value *skipped;} i_tree;
void i_skip(i_tree *t,const char *path,const char *code,unsigned reason);
int i_walk(i_tree *t,int dir,const char *root,unsigned depth);
j_value *f_search(const j_value *q,f_error *e);
j_value *f_read_many(const j_value *q,f_error *e);
int f_fields(const j_value *q,const char *allowed);
#endif
