#ifndef RHOST_INSPECT_H
#define RHOST_INSPECT_H
#include "files.h"
#define I_COUNT 100000
#define I_MANIFEST (8u*1024u*1024u)
#define I_SCAN (64u*1024u*1024u)
typedef struct {char **paths;size_t count,visited,bytes,diagnostics;unsigned reasons[6];j_value *skipped;const char *root,*glob;const j_value *excludes;} i_tree;
void i_skip(i_tree *t,const char *path,const char *code,unsigned reason);
int i_walk(i_tree *t,int dir,const char *root,unsigned depth);
j_value *f_search(const j_value *q,f_error *e);
j_value *f_read_many(const j_value *q,f_error *e);
int f_fields(const j_value *q,const char *allowed);
#endif
