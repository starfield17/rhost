#ifndef RHOST_FILES_H
#define RHOST_FILES_H
#include "protocol.h"
#include "json.h"
#include <sys/stat.h>
#define F_EDIT_LIMIT (8u*1024u*1024u)
typedef struct {const char *code,*message,*mutation;} f_error;
void *f_fail(f_error *e,const char *code,const char *message);
void *f_os_error(f_error *e,int changed);
int f_context(f_error *e);
int f_fields(const j_value *q,const char *allowed);
int f_number(const j_value *q,const char *key,uint64_t fallback,uint64_t *out);
char *f_path(const j_value *q,f_error *e);
int f_directory(const char *path,int parents,f_error *e);
int f_parent(const char *path,int parents,char **name,f_error *e);
int f_open(int dir,const char *name,int changed,f_error *e);
int f_identity(const struct stat *a,const struct stat *b);
int f_hash_fd(int fd,char hash[65],uint64_t *lines,f_error *e);
int f_load(int fd,j_buf *out,f_error *e);
j_value *f_read(const j_value *q,const char *path,f_error *e);
j_value *f_edit(const j_value *q,const char *path,int patch,f_error *e);
int f_patch(const j_value *edits,const j_buf *old,j_buf *out,f_error *e);
int f_mode(const j_value *q,mode_t *out,f_error *e);
int f_save(int fd,const void *data,size_t size);
int f_decode(const j_value *content,j_buf *out,f_error *e);
#endif
