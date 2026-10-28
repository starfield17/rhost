#include "changes.h"
#include <stdlib.h>
#include <string.h>
const char *ch_text(const j_value *v,const char *key){const char *s=j_string(j_get(v,key));return s?s:"";}
uint64_t ch_num(const j_value *v,const char *key){j_value *n=j_get(v,key);return n && n->type==J_INT && !n->negative?n->number:0;}
int ch_put(j_value *v,const char *key,j_value *value){
    if(!v || !value){j_free(value);return 0;}
    j_value **link=&v->first;while(*link){j_value *old=*link;if(old->key && !strcmp(old->key,key)){*link=old->next;old->next=NULL;j_free(old);break;}link=&old->next;}
    return j_add(v,key,value);
}
int ch_string(j_value *v,const char *key,const char *text){j_value *box=j_new(J_OBJECT);if(!box || !j_text(box,"v",text,strlen(text))){j_free(box);return 0;}j_value *child=box->first;box->first=NULL;j_free(box);free(child->key);child->key=NULL;return ch_put(v,key,child);}
int ch_integer(j_value *v,const char *key,uint64_t value){j_value *n=j_new(J_INT);if(n)n->number=value;return ch_put(v,key,n);}
int ch_bool(j_value *v,const char *key,int value){j_value *n=j_new(J_BOOL);if(n)n->number=value?1:0;return ch_put(v,key,n);}

static int compare(const void *a,const void *b){return strcmp((*(const j_value *const *)a)->key,(*(const j_value *const *)b)->key);}
j_value *ch_canonical(const j_value *v){
    j_value *copy=j_clone(v);if(!copy)return NULL;
    if(copy->type==J_OBJECT || copy->type==J_ARRAY){
        size_t count=0;for(j_value *p=copy->first;p;p=p->next)count++;
        j_value **values=calloc(count?count:1,sizeof *values);if(!values){j_free(copy);return NULL;}
        size_t at=0;for(j_value *p=copy->first;p;p=p->next)values[at++]=p;
        if(copy->type==J_OBJECT)qsort(values,count,sizeof *values,compare);
        j_value *result=j_new(copy->type);
        for(size_t i=0;result && i<count;i++){j_value *child=ch_canonical(values[i]);if(!child){j_free(result);result=NULL;break;}free(child->key);child->key=NULL;if(!j_add(result,values[i]->key,child)){j_free(result);result=NULL;}}
        free(values);j_free(copy);return result;
    }
    return copy;
}
j_value *ch_version(const struct stat *s){
#ifdef __APPLE__
    uint64_t mt=(uint64_t)s->st_mtimespec.tv_sec,mn=(uint64_t)s->st_mtimespec.tv_nsec,ct=(uint64_t)s->st_ctimespec.tv_sec,cn=(uint64_t)s->st_ctimespec.tv_nsec;
#else
    uint64_t mt=(uint64_t)s->st_mtim.tv_sec,mn=(uint64_t)s->st_mtim.tv_nsec,ct=(uint64_t)s->st_ctim.tv_sec,cn=(uint64_t)s->st_ctim.tv_nsec;
#endif
    uint64_t fields[]={(uint64_t)s->st_dev,(uint64_t)s->st_ino,(uint64_t)s->st_size,(uint64_t)(s->st_mode&07777),mt,mn,ct,cn};j_value *v=j_new(J_ARRAY);
    for(size_t i=0;i<8;i++)if(!j_integer(v,NULL,fields[i])){j_free(v);return NULL;}return v;
}
int ch_same(const j_value *v,const struct stat *s,int intent){
    j_value *current=ch_version(s);if(!v || v->type!=J_ARRAY || !current){j_free(current);return 0;}
    j_value *a=v->first,*b=current->first;size_t n=0;int ok=1;
    for(;a && b;a=a->next,b=b->next,n++)if(a->type!=J_INT || a->negative || ((n<6 || !intent) && a->number!=b->number))ok=0;
    ok=ok && !a && !b && n==8;j_free(current);return ok;
}
