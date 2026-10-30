#define _POSIX_C_SOURCE 200809L
#include "state.h"
#include <errno.h>
#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
int rh_random(char *out,size_t bytes) {
    unsigned char raw[32];
    if(bytes>sizeof raw) return 0;
    int fd=open("/dev/urandom",O_RDONLY|O_CLOEXEC);
    if(fd<0) return 0;
    size_t at=0;
    while(at<bytes) {ssize_t n=read(fd,raw+at,bytes-at);if(n<=0) {close(fd);return 0;} at+=(size_t)n;}
    close(fd);rh_hex(raw,bytes,out);return 1;
}
int rh_directory(const char *path,int create) {
    if(!path || path[0]!='/') return -1;
    int directory=open("/",O_RDONLY|O_DIRECTORY|O_CLOEXEC);
    char *copy=strdup(path),*save=NULL;
    if(directory<0 || !copy) {if(directory>=0)close(directory);free(copy);return -1;}
    for(char *p=strtok_r(copy,"/",&save);p;p=strtok_r(NULL,"/",&save)) {
        if(!strcmp(p,".") || !strcmp(p,"..")) {close(directory);free(copy);return -1;}
        if(create && mkdirat(directory,p,0700)<0 && errno!=EEXIST) {close(directory);free(copy);return -1;}
        int next=openat(directory,p,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        close(directory); directory=next;if(next<0)break;
        struct stat info;
        if(fstat(directory,&info)<0 || (info.st_uid!=0 && info.st_uid!=geteuid()) ||
           ((info.st_mode&0022) && !(info.st_mode&01000))) {close(directory);directory=-1;break;}
    }
    free(copy);return directory;
}
int rh_atomic(int directory,const char *name,const void *data,size_t size) {
    char token[33],temporary[64];
    if(!rh_random(token,16))return 0;
    int n=snprintf(temporary,sizeof temporary,".write-%s",token);if(n<0 || (size_t)n>=sizeof temporary)return 0;
    int fd=openat(directory,temporary,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);
    if(fd<0)return 0;
    size_t at=0;int ok=1;
    while(at<size) {ssize_t used=write(fd,(const char *)data+at,size-at);if(used<0 && errno==EINTR)continue;if(used<=0){ok=0;break;}at+=(size_t)used;}
    if(fsync(fd)<0)ok=0;
    if(close(fd)<0)ok=0;
    if(ok && renameat(directory,temporary,directory,name)<0)ok=0;
    if(ok && fsync(directory)<0)ok=0;
    unlinkat(directory,temporary,0);return ok;
}
int rh_clear_atomic(int directory){
    int copy=dup(directory);DIR *dir=copy<0?NULL:fdopendir(copy);if(!dir){if(copy>=0)close(copy);return 0;}
    struct dirent *p;size_t count=0;int ok=1,changed=0;
    while((errno=0,p=readdir(dir))){
        if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
        if(++count>1024){ok=0;break;}
        if(strncmp(p->d_name,".write-",7))continue;
        struct stat st;if(!rh_generation(p->d_name+7) || fstatat(directory,p->d_name,&st,AT_SYMLINK_NOFOLLOW)<0 || !S_ISREG(st.st_mode) || st.st_uid!=geteuid() || (st.st_mode&0077) || unlinkat(directory,p->d_name,0)<0){ok=0;break;}changed=1;
    }
    if(errno)ok=0;closedir(dir);if(changed && fsync(directory)<0)ok=0;return ok;
}
int rh_file(int directory,const char *name,unsigned char **data,size_t *size,size_t limit) {
    *data=NULL;*size=0;
    int fd=openat(directory,name,O_RDONLY|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC);
    if(fd<0)return 0;
    struct stat info;
    if(fstat(fd,&info)<0 || !S_ISREG(info.st_mode) || info.st_size<0 || (uint64_t)info.st_size>limit) {close(fd);return 0;}
    *size=(size_t)info.st_size;*data=malloc(*size+1);
    if(!*data) {close(fd);return 0;}
    size_t at=0;
    while(at<*size) {ssize_t n=read(fd,*data+at,*size-at);if(n<=0){free(*data);*data=NULL;close(fd);return 0;}at+=(size_t)n;}
    (*data)[*size]=0;close(fd);return 1;
}
int rh_generation(const char *s) {
    if(!s || strlen(s)!=32)return 0;
    for(size_t i=0;i<32;i++)if(!((s[i]>='0' && s[i]<='9')||(s[i]>='a' && s[i]<='f')))return 0;
    return 1;
}
int rh_state_open(rh_state *s,int create) {
    memset(s,0,sizeof *s);s->root=s->runs=s->registry=-1;
    const char *root=getenv("RHOST_REMOTE_STATE");char *default_root=NULL;
    if(!root) {
        const char *home=getenv("HOME");if(!home)return 0;
        size_t n=strlen(home)+32;default_root=malloc(n);if(!default_root)return 0;
        snprintf(default_root,n,"%s/.local/state/rhost/v5",home);root=default_root;
    }
    s->root=rh_directory(root,create);free(default_root);
    struct stat info;
    if(s->root<0 || fstat(s->root,&info)<0 || info.st_uid!=geteuid() || (info.st_mode&0077))goto fail;
    s->registry=openat(s->root,"registry.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC|(create?O_CREAT:0),0600);
    if(s->registry<0 || fstat(s->registry,&info)<0 || !S_ISREG(info.st_mode) || info.st_uid!=geteuid() || (info.st_mode&0077))goto fail;
    if(flock(s->registry,LOCK_EX)<0)goto fail;
    unsigned char *generation=NULL;size_t size=0;
    if(!rh_file(s->root,"generation",&generation,&size,32)) {
        struct stat existing;
        if(!create || fstatat(s->root,"generation",&existing,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT)goto fail;
        if(!rh_random(s->generation,16) || !rh_atomic(s->root,"generation",s->generation,32))goto fail;
    } else {
        if(size!=32 || !rh_generation((char *)generation)){free(generation);goto fail;}
        memcpy(s->generation,generation,33);free(generation);
    }
    if(create && mkdirat(s->root,"runs",0700)<0 && errno!=EEXIST)goto fail;
    s->runs=openat(s->root,"runs",O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(s->runs<0 || fstat(s->runs,&info)<0 || info.st_uid!=geteuid() || (info.st_mode&0077))goto fail;
    return 1;
fail:rh_state_close(s);return 0;
}
void rh_state_close(rh_state *s) {
    if(s->registry>=0)close(s->registry);
    if(s->runs>=0)close(s->runs);
    if(s->root>=0)close(s->root);
    s->root=s->runs=s->registry=-1;
}
int rh_clock_read(rh_state *s,uint64_t *now) {
    time_t value=time(NULL);if(value<0)return 0;*now=(uint64_t)value;
    unsigned char *data=NULL;size_t size=0;
    if(rh_file(s->root,"clock",&data,&size,8)) {
        if(size!=8){free(data);return 0;}
        uint64_t previous=rh_u64(data);free(data);if(previous>*now)*now=previous;
    }else {struct stat info;if(fstatat(s->root,"clock",&info,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT)return 0;}
    return 1;
}
int rh_clock(rh_state *s,uint64_t *now) {
    if(!rh_clock_read(s,now))return 0;
    unsigned char out[8];rh_put32(out,(uint32_t)(*now>>32));rh_put32(out+4,(uint32_t)*now);
    return rh_atomic(s->root,"clock",out,8);
}
int rh_status_write(int directory,const rh_status *s) {
    rh_buf b={0};rh_raw(&b,"RHST0001",8);
    rh_byte(&b,s->state);rh_byte(&b,s->completion);rh_int(&b,s->value);
    rh_byte(&b,s->out_open);rh_byte(&b,s->err_open);rh_byte(&b,s->incomplete);
    rh_long(&b,s->out_bytes);rh_long(&b,s->err_bytes);rh_long(&b,s->out_kept);rh_long(&b,s->err_kept);
    int ok=!b.failed && rh_atomic(directory,"status",b.data,b.size);free(b.data);return ok;
}
int rh_status_read(int directory,rh_status *s) {
    unsigned char *data=NULL;size_t size=0;
    if(!rh_file(directory,"status",&data,&size,128))return 0;
    if(size!=49 || memcmp(data,"RHST0001",8)){free(data);return 0;}
    rh_reader r={data,size,8,0};memset(s,0,sizeof *s);
    s->state=rh_read_byte(&r);s->completion=rh_read_byte(&r);s->value=rh_read_int(&r);
    s->out_open=rh_read_byte(&r);s->err_open=rh_read_byte(&r);s->incomplete=rh_read_byte(&r);
    s->out_bytes=rh_read_long(&r);s->err_bytes=rh_read_long(&r);s->out_kept=rh_read_long(&r);s->err_kept=rh_read_long(&r);
    int ok=rh_done(&r) && s->state>=1 && s->state<=5 && s->out_open<=1 && s->err_open<=1 && s->incomplete<=1 && s->out_kept<=s->out_bytes && s->err_kept<=s->err_bytes;
    if(s->state==4)ok=ok && ((s->completion==1 && s->value<=255)||(s->completion==2 && s->value>0));
    else ok=ok && !s->completion && !s->value;
    free(data);return ok;
}
int rh_key(const char *key,uint64_t *expiry) {
    if(!key || *key<'1' || *key>'9')return 0;
    uint64_t value=0;size_t at=0;
    while(key[at]>='0' && key[at]<='9') {
        if(at>=12)return 0;
        value=value*10+(unsigned)(key[at++]-'0');
    }
    if(key[at++]!='-' || strlen(key+at)!=64)return 0;
    for(size_t i=at;i<at+64;i++)if(!((key[i]>='0' && key[i]<='9')||(key[i]>='a' && key[i]<='f')))return 0;
    *expiry=value;return 1;
}
uint64_t rh_millis(void) {
    struct timespec t;if(clock_gettime(CLOCK_MONOTONIC,&t)<0)return 0;
    return (uint64_t)t.tv_sec*1000+(uint64_t)t.tv_nsec/1000000;
}
void rh_record(rh_buf *b,const rh_state *s,const char *key,const rh_status *status) {
    rh_text(b,s->generation);rh_text(b,key);rh_byte(b,status->state);
    if(status->state==4) {rh_byte(b,status->completion);rh_int(b,status->value);}
}
