#define _POSIX_C_SOURCE 200809L
#include "state.h"
#include <stdlib.h>
#include <string.h>
static int env_name(const char *p) {
    if(!p || !((*p>='A' && *p<='Z')||(*p>='a' && *p<='z')||*p=='_'))return 0;
    for(p++;*p;p++)if(!((*p>='A' && *p<='Z')||(*p>='a' && *p<='z')||(*p>='0' && *p<='9')||*p=='_'))return 0;
    return 1;
}
int rh_parse_start(rh_reader *r,rh_start *q) {
    memset(q,0,sizeof *q);
    q->generation=rh_read_text(r);q->key=rh_read_text(r);q->expiry=rh_read_long(r);q->cwd=rh_read_text(r);
    unsigned program=rh_read_byte(r);
    if(program==1) {
        q->argc=rh_read_int(r);
        if(!q->argc || q->argc>4096 || q->argc>(r->size-r->at)/4)return 0;
        q->argv=calloc(q->argc+1,sizeof(char *));if(!q->argv)return 0;
        for(size_t i=0;i<q->argc;i++)q->argv[i]=rh_read_text(r);
    }else if(program==2) {
        char *interpreter=rh_read_text(r),*script=rh_read_text(r);
        q->login=rh_read_byte(r);
        if(!interpreter || !script || interpreter[0]!='/' || !*script || q->login>1) {free(interpreter);free(script);return 0;}
        q->argc=q->login?4:3;q->argv=calloc(q->argc+1,sizeof(char *));
        if(!q->argv){free(interpreter);free(script);return 0;}
        q->argv[0]=interpreter;
        if(q->login) {q->argv[1]=strdup("-l");q->argv[2]=strdup("-c");q->argv[3]=script;}
        else {q->argv[1]=strdup("-c");q->argv[2]=script;}
    }else return 0;
    for(size_t i=0;i<q->argc;i++)if(!q->argv[i])return 0;
    if(!*q->argv[0])return 0;
    q->inherit=rh_read_byte(r);q->envc=rh_read_int(r);
    if(q->inherit>1 || q->envc>4096 || q->envc>(r->size-r->at)/8)return 0;
    q->env=calloc(q->envc+1,sizeof(char *));if(!q->env)return 0;
    char *previous=NULL;
    for(size_t i=0;i<q->envc;i++) {
        char *name=rh_read_text(r),*value=rh_read_text(r);
        if(!name || !value || !env_name(name) || (previous && strcmp(previous,name)>=0)) {free(name);free(value);free(previous);return 0;}
        free(previous);previous=name;
        size_t size=strlen(name)+strlen(value)+2;q->env[i]=malloc(size);
        if(!q->env[i]){free(value);free(previous);return 0;}
        memcpy(q->env[i],name,strlen(name));q->env[i][strlen(name)]='=';
        strcpy(q->env[i]+strlen(name)+1,value);free(value);
    }
    free(previous);
    q->input=rh_read_bytes(r,&q->input_size);q->wait=rh_read_int(r);q->deadline=rh_read_long(r);
    uint64_t expiry;
    return rh_done(r) && q->input && rh_generation(q->generation) && rh_key(q->key,&expiry) && expiry==q->expiry && q->cwd && *q->cwd && q->wait<=300000 && q->deadline<=604800000;
}
void rh_start_free(rh_start *q) {
    free(q->generation);free(q->key);free(q->cwd);free(q->input);
    if(q->argv){for(size_t i=0;i<q->argc;i++)free(q->argv[i]);free(q->argv);}
    if(q->env){for(size_t i=0;i<q->envc;i++)free(q->env[i]);free(q->env);}
}
