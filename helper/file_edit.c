#include "files.h"
#include "state.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <sys/file.h>
#include <unistd.h>
int f_mode(const j_value *q,mode_t *out,f_error *e){
    j_value *v=j_get(q,"file_mode");if(!v || v->type==J_NULL)return 1;
    const char *s=j_string(v);if(!s)goto invalid;size_t n=strlen(s);if(!n)return 1;if(n!=3 && n!=4)goto invalid;
    mode_t value=0;for(size_t i=0;i<n;i++){if(s[i]<'0'||s[i]>'7')goto invalid;value=(value<<3)|(mode_t)(s[i]-'0');}*out=value;return 1;
invalid:f_fail(e,"CONFIG_INVALID","file_mode must be an octal permission such as 0644");return 0;
}
int f_save(int fd,const void *data,size_t size){
    const unsigned char *p=data;while(size){ssize_t n=write(fd,p,size);if(n<0 && errno==EINTR)continue;if(n<=0)return 0;p+=n;size-=(size_t)n;}return 1;
}
j_value *f_edit(const j_value *q,const char *path,int patch,f_error *e){
    j_value *expected_value=j_get(q,"if_hash"),*parents_value=j_get(q,"parents");
    const char *expected=expected_value?j_string(expected_value):"";
    if(!expected || (parents_value && parents_value->type!=J_BOOL))return f_fail(e,"CONFIG_INVALID","invalid edit policy");
    j_buf old={0},data={0};mode_t permissions=0600;
    if(!f_mode(q,&permissions,e) || (!patch && !f_decode(j_get(q,"content"),&data,e))){free(data.data);return NULL;}
    if(!patch && !f_context(e)){free(data.data);return NULL;}
    int parents=!patch && parents_value && parents_value->number;char *name=NULL;int dir=f_parent(path,parents,&name,e);if(dir<0){free(data.data);return NULL;}
    int fd=-1,exists=0;struct stat info,loaded;j_value *result=NULL;char temporary[64]={0};
    if(flock(dir,LOCK_EX)<0){f_fail(e,"STATE_UNAVAILABLE","filesystem locking is unavailable");goto done;}
    if(fstatat(dir,name,&info,AT_SYMLINK_NOFOLLOW)<0){if(errno!=ENOENT){f_os_error(e,0);goto done;}}
    else{
        exists=1;if(!S_ISREG(info.st_mode)){f_fail(e,"INVALID_TARGET","target is not a regular file");goto done;}
        if(info.st_size<0 || (uint64_t)info.st_size>F_EDIT_LIMIT){f_fail(e,"FILE_TOO_LARGE","editing limit is 8388608 bytes");goto done;}
        fd=f_open(dir,name,1,e);if(fd<0)goto done;
        if(fstat(fd,&loaded)<0){f_os_error(e,1);goto done;}
        if(!f_identity(&info,&loaded)){f_fail(e,"FILE_CONFLICT","target changed before reading");goto done;}
        if(!f_load(fd,&old,e))goto done;
        struct stat after;if(fstat(fd,&after)<0){f_os_error(e,1);goto done;}
        if(!f_identity(&loaded,&after)){f_fail(e,"FILE_CONFLICT","target changed while reading");goto done;}
        close(fd);fd=-1;permissions=info.st_mode&07777;
    }
    if(!f_mode(q,&permissions,e))goto done;
    unsigned char digest[32];char old_hash[65];rh_sha256(old.data,old.size,digest);rh_hex(digest,32,old_hash);
    if(exists && !*expected){f_fail(e,"HASH_REQUIRED","an existing file requires if_hash from fs read");goto done;}
    if((exists && strcmp(expected,old_hash)) || (!exists && (*expected || patch))){f_fail(e,"FILE_CONFLICT","file changed or expected target is missing");goto done;}
    if(patch){if(!f_patch(j_get(q,"edits"),&old,&data,e))goto done;}
    if(patch && !f_context(e))goto done;
    for(unsigned attempt=0;attempt<100;attempt++){
        char random[33];if(!rh_random(random,16)){f_fail(e,"INTERNAL","random source failed");goto done;}
        snprintf(temporary,sizeof temporary,".rhost-write-%s",random);fd=openat(dir,temporary,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);
        if(fd>=0)break;if(errno!=EEXIST){f_os_error(e,0);temporary[0]=0;goto done;}
        temporary[0]=0;
    }
    if(fd<0){f_fail(e,"INTERNAL","cannot allocate a temporary file");goto done;}
    if(!f_save(fd,data.data,data.size) || fchmod(fd,permissions)<0 || fsync(fd)<0){f_os_error(e,0);goto done;}close(fd);fd=-1;
    if(exists){
        char now[65];fd=f_open(dir,name,1,e);if(fd<0)goto done;
        if(fstat(fd,&loaded)<0 || !f_hash_fd(fd,now,NULL,e)){if(!e->code)f_os_error(e,1);goto done;}
        close(fd);fd=-1;
        if(!f_identity(&info,&loaded) || strcmp(now,expected)){f_fail(e,"FILE_CONFLICT","file changed before replacement");goto done;}
        e->mutation="unknown";
        if(renameat(dir,temporary,dir,name)<0){f_os_error(e,1);goto done;}
        temporary[0]=0;
    }else{
        e->mutation="unknown";
        if(linkat(dir,temporary,dir,name,0)<0){if(errno==EEXIST){e->mutation="not_applied";f_fail(e,"FILE_CONFLICT","target was concurrently created");}else f_os_error(e,0);goto done;}
    }
    e->mutation="applied";if(fsync(dir)<0){f_os_error(e,0);goto done;}
    fd=f_open(dir,name,1,e);if(fd<0)goto done;char verified[65],wanted[65];
    if(!f_hash_fd(fd,verified,NULL,e))goto done;rh_sha256(data.data,data.size,digest);rh_hex(digest,32,wanted);
    if(strcmp(wanted,verified)){f_fail(e,"FILE_CONFLICT","file changed during post-write verification");goto done;}
    result=j_new(J_OBJECT);
    if(!result || !j_text(result,"path",path,strlen(path)) || !j_text(result,"sha256",verified,64) || !j_integer(result,"bytes",data.size)){
        j_free(result);result=NULL;f_fail(e,"INTERNAL","allocation failed");
    }
done:if(fd>=0)close(fd);if(temporary[0])unlinkat(dir,temporary,0);close(dir);free(name);free(old.data);free(data.data);return result;
}
