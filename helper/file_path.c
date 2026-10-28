#include "files.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
void *f_fail(f_error *e,const char *code,const char *message){if(!e->code){e->code=code;e->message=message;}return NULL;}
void *f_os_error(f_error *e,int changed){
    const char *code="INTERNAL";
    if(errno==ENOENT)code=changed?"FILE_CONFLICT":"FILE_NOT_FOUND";
    else if(errno==ELOOP)code=changed?"FILE_CONFLICT":"INVALID_TARGET";
    else if(errno==EISDIR || errno==ENOTDIR || errno==EACCES || errno==EPERM)code="INVALID_TARGET";
    return f_fail(e,code,"cannot access the requested filesystem entry");
}
int f_number(const j_value *q,const char *key,uint64_t fallback,uint64_t *out){
    j_value *v=j_get(q,key);if(!v){*out=fallback;return 1;}if(v->type!=J_INT || v->negative)return 0;*out=v->number;return 1;
}
char *f_path(const j_value *q,f_error *e){
    const char *input=j_string(j_get(q,"path"));
    if(!input || !*input){f_fail(e,"CONFIG_INVALID","path must be a nonempty string without NUL");return NULL;}
    size_t n=strlen(input);if(n>=2 && (input[0]=='\''||input[0]=='"') && input[n-1]==input[0]){f_fail(e,"CONFIG_INVALID","path contains outer shell quotes");return NULL;}
    char *base=NULL,*raw=NULL;
    if(input[0]=='~' && (input[1]=='/'||input[1]==0)){base=getenv("HOME");input++;}
    else if(input[0]!='/'){base=getcwd(NULL,0);if(!base){f_os_error(e,0);return NULL;}}
    size_t length=strlen(input)+(base?strlen(base):0)+2;
    raw=malloc(length);if(!raw){if(base && base!=getenv("HOME"))free(base);return f_fail(e,"INTERNAL","allocation failed");}
    if(base)snprintf(raw,length,"%s/%s",base,input);else snprintf(raw,length,"%s",input);
    if(base && base!=getenv("HOME"))free(base);
    if(raw[0]!='/'){free(raw);return f_fail(e,"CONFIG_INVALID","cannot resolve an absolute path");}
    char *result=malloc(length);if(!result){free(raw);return f_fail(e,"INTERNAL","allocation failed");}
    size_t used=0;result[used++]='/';char *save=NULL;
    for(char *part=strtok_r(raw,"/",&save);part;part=strtok_r(NULL,"/",&save)){
        if(!strcmp(part,"."))continue;
        if(!strcmp(part,"..")){while(used>1 && result[used-1]!='/')used--;if(used>1)used--;continue;}
        if(used>1)result[used++]='/';size_t size=strlen(part);memcpy(result+used,part,size);used+=size;
    }
    result[used]=0;free(raw);return result;
}
int f_directory(const char *path,int parents,f_error *e){
    int dir=open("/",O_RDONLY|O_DIRECTORY|O_CLOEXEC);char *copy=strdup(path);char *save=NULL;
    if(dir<0 || !copy){if(dir>=0)close(dir);free(copy);f_fail(e,"INTERNAL","cannot open directory");return -1;}
    for(char *part=strtok_r(copy,"/",&save);part;part=strtok_r(NULL,"/",&save)){
        int child=openat(dir,part,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        if(child<0 && errno==ENOENT && parents){
            if(mkdirat(dir,part,0700)<0 && errno!=EEXIST){f_os_error(e,0);goto fail;}
            child=openat(dir,part,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        }
        if(child<0){f_os_error(e,0);goto fail;}close(dir);dir=child;
    }
    free(copy);return dir;
fail:close(dir);free(copy);return -1;
}
int f_parent(const char *path,int parents,char **name,f_error *e){
    char *copy=strdup(path);if(!copy){f_fail(e,"INTERNAL","allocation failed");return -1;}
    char *slash=strrchr(copy,'/');if(!slash || !slash[1]){free(copy);f_fail(e,"INVALID_TARGET","the filesystem root is not a file");return -1;}
    *name=strdup(slash+1);if(!*name){free(copy);f_fail(e,"INTERNAL","allocation failed");return -1;}
    if(slash==copy)slash[1]=0;else *slash=0;
    int dir=f_directory(copy,parents,e);free(copy);if(dir<0){free(*name);*name=NULL;}return dir;
}
int f_open(int dir,const char *name,int changed,f_error *e){
    int fd=openat(dir,name,O_RDONLY|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC);
    if(fd<0){f_os_error(e,changed);return -1;}struct stat st;
    if(fstat(fd,&st)<0){f_os_error(e,changed);close(fd);return -1;}
    if(!S_ISREG(st.st_mode)){f_fail(e,"INVALID_TARGET","target is not a regular file");close(fd);return -1;}return fd;
}
int f_identity(const struct stat *a,const struct stat *b){
#ifdef __APPLE__
    return a->st_dev==b->st_dev && a->st_ino==b->st_ino && a->st_size==b->st_size && a->st_mtimespec.tv_sec==b->st_mtimespec.tv_sec && a->st_mtimespec.tv_nsec==b->st_mtimespec.tv_nsec && a->st_ctimespec.tv_sec==b->st_ctimespec.tv_sec && a->st_ctimespec.tv_nsec==b->st_ctimespec.tv_nsec;
#else
    return a->st_dev==b->st_dev && a->st_ino==b->st_ino && a->st_size==b->st_size && a->st_mtim.tv_sec==b->st_mtim.tv_sec && a->st_mtim.tv_nsec==b->st_mtim.tv_nsec && a->st_ctim.tv_sec==b->st_ctim.tv_sec && a->st_ctim.tv_nsec==b->st_ctim.tv_nsec;
#endif
}
