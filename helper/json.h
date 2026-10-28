#ifndef RHOST_JSON_H
#define RHOST_JSON_H
#include <stddef.h>
#include <stdint.h>
#define J_INPUT_LIMIT (12u*1024u*1024u)
#define J_OUTPUT_LIMIT (48u*1024u*1024u+65536u)
enum {J_NULL,J_BOOL,J_INT,J_REAL,J_TEXT,J_ARRAY,J_OBJECT};
typedef struct j_value {
    unsigned type,negative;uint64_t number;
    unsigned char *text;size_t size;
    char *key;size_t key_size;
    struct j_value *first,*next;
} j_value;
typedef struct {unsigned char *data;size_t size,capacity,limit;int failed;} j_buf;
void j_append(j_buf *b,const void *p,size_t n);
j_value *j_parse(const unsigned char *bytes,size_t size);
void j_free(j_value *v);
j_value *j_new(unsigned type);
int j_add(j_value *parent,const char *name,j_value *child);
j_value *j_get(const j_value *parent,const char *name);
const char *j_string(const j_value *v);
int j_text(j_value *parent,const char *name,const void *text,size_t size);
int j_integer(j_value *parent,const char *name,uint64_t value);
int j_boolean(j_value *parent,const char *name,int value);
int j_dump(const j_value *v,j_buf *out);
int rh_files_json(void);
#endif
