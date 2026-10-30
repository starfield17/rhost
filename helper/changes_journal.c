#include "changes.h"
#include "registry.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>
static int private_dir(int fd){struct stat st;return fd>=0 && fstat(fd,&st)==0 && S_ISDIR(st.st_mode) && st.st_uid==geteuid() && !(st.st_mode&0077);}
int ch_save(ch_set *s){j_buf b={0};b.limit=256*1024;int ok=j_dump(s->manifest,&b) && rh_atomic(s->dir,"manifest.json",b.data,b.size);free(b.data);if(!ok)f_fail(s->error,"STATE_UNAVAILABLE","cannot persist changeset evidence");return ok;}
int ch_archive(ch_set *s,size_t i,const char *prefix,const j_buf *bytes){char name[32];snprintf(name,sizeof name,"%s-%zu",prefix,i);if(!rh_atomic(s->dir,name,bytes->data,bytes->size)){f_fail(s->error,"STATE_UNAVAILABLE","cannot archive changeset bytes");return 0;}return 1;}
int ch_bytes(ch_set *s,size_t i,const char *prefix,j_buf *out,const char *hash){
    char name[32];snprintf(name,sizeof name,"%s-%zu",prefix,i);out->limit=F_EDIT_LIMIT;
    if(!rh_file(s->dir,name,&out->data,&out->size,F_EDIT_LIMIT)){f_fail(s->error,"STATE_UNAVAILABLE","archived file bytes are unavailable");return 0;}
    unsigned char raw[32];char hex[65];rh_sha256(out->data,out->size,raw);rh_hex(raw,32,hex);
    if(strcmp(hex,hash)){free(out->data);out->data=NULL;f_fail(s->error,"STATE_UNAVAILABLE","archived file hash changed");return 0;}return 1;
}
static int digest_valid(const char *s){if(strlen(s)!=64)return 0;for(size_t i=0;i<64;i++)if(!((s[i]>='0' && s[i]<='9')||(s[i]>='a' && s[i]<='f')))return 0;return 1;}
static int version_valid(const j_value *v,size_t count){if(!v || v->type!=J_ARRAY)return 0;size_t n=0;for(j_value *p=v->first;p;p=p->next){if(p->type!=J_INT || p->negative)return 0;n++;}return n==count;}
static int one_of(const char *value,const char *list){for(const char *p=list;*p;){const char *end=strchr(p,' ');size_t n=end?(size_t)(end-p):strlen(p);if(strlen(value)==n && !memcmp(p,value,n))return 1;p=end?end+1:p+n;}return 0;}
static int entry_valid(const j_value *p,int ready){
    const char *phase=ch_text(p,"state");j_value *ever=j_get(p,"ever_applied");
    if(!one_of(phase,"pending installing installed restoring restored") || !ever || ever->type!=J_BOOL)return 0;
    if(!ready)return !ever->number && !strcmp(phase,"pending");
    j_value *exists=j_get(p,"existed"),*mode=j_get(p,"mode"),*before=j_get(p,"before");
    if(!exists || exists->type!=J_BOOL || !mode || mode->type!=J_INT || mode->negative || mode->number>07777 || !digest_valid(ch_text(p,"before_hash")) || !digest_valid(ch_text(p,"after_hash")) || !version_valid(j_get(p,"parent"),2))return 0;
    if(exists->number?!version_valid(before,8):!before || before->type!=J_NULL)return 0;
    if(!strcmp(phase,"installing") && !version_valid(j_get(p,"stage"),8))return 0;
    if((!strcmp(phase,"installed") || !strcmp(phase,"restoring")) && (!ever->number || !version_valid(j_get(p,"installed"),8)))return 0;
    if(!strcmp(phase,"restoring") && exists->number && !version_valid(j_get(p,"stage"),8))return 0;
    if(!strcmp(phase,"restored") && (!ever->number || (exists->number && !version_valid(j_get(p,"restored"),8))))return 0;
    return 1;
}
int ch_load(ch_set *s){
    unsigned char *raw=NULL;size_t size=0;j_free(s->manifest);s->manifest=NULL;
    if(!rh_file(s->dir,"manifest.json",&raw,&size,256*1024))return 0;s->manifest=j_parse(raw,size);free(raw);
    if(!s->manifest || !digest_valid(ch_text(s->manifest,"fingerprint")) || ch_num(s->manifest,"expires_unix")!=s->expiry || !one_of(ch_text(s->manifest,"status"),"registered prepared applying applied partial rolling_back rolled_back refused unknown") || strcmp(ch_text(s->manifest,"id"),s->id) || strcmp(ch_text(s->manifest,"generation"),s->space.generation) || ch_num(s->manifest,"version")!=1)return 0;
    if(!rh_file(s->dir,"request.json",&raw,&size,J_INPUT_LIMIT))return 0;j_free(s->request);s->request=j_parse(raw,size);
    unsigned char digest[32];char hash[65];rh_sha256(raw,size,digest);rh_hex(digest,32,hash);free(raw);
    if(!s->request || strcmp(ch_text(s->manifest,"fingerprint"),hash))return 0;
    j_value *list=j_get(s->manifest,"files"),*intent=j_get(s->request,"changes");
    if(!list || list->type!=J_ARRAY || !intent || intent->type!=J_ARRAY)return 0;s->count=0;
    j_value *p=list->first,*q=intent->first;
    for(;p && q;p=p->next,q=q->next){if(s->count==CH_COUNT)return 0;ch_entry *entry=&s->entries[s->count++];entry->meta=p;entry->intent=q;entry->path=(char *)ch_text(p,"path");entry->dir=-1;entry->lock=0;
        int ready=strcmp(ch_text(s->manifest,"status"),"registered") && strcmp(ch_text(s->manifest,"status"),"refused");
        if(!entry_valid(p,ready) || !*entry->path || strcmp(entry->path,ch_text(q,"path")) || !*ch_text(p,"state"))return 0;}
    return !p && !q && s->count>0;
}
int ch_open(ch_set *s,const j_value *q,int apply){
    if(!rh_state_open(&s->space,0))return 0;
    if(strcmp(ch_text(q,"generation"),s->space.generation)){f_fail(s->error,"NAMESPACE_MISMATCH","remote namespace changed");return 0;}
    if(apply && mkdirat(s->space.root,"changes",0700)<0 && errno!=EEXIST)return 0;
    s->base=openat(s->space.root,"changes",O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);if(!private_dir(s->base))return 0;
    s->dir=openat(s->base,s->id,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(s->dir>=0)return private_dir(s->dir) && ch_load(s)?1:0;
    if(errno!=ENOENT || !apply){if(errno==ENOENT)f_fail(s->error,"FILE_NOT_FOUND","changeset record is not retained");return 0;}
    uint64_t now;if(!rh_clock(&s->space,&now))return 0;
    if(s->expiry<=now || s->expiry-now>604800){f_fail(s->error,"REQUEST_EXPIRED","new changeset key is expired or exceeds seven days");return 0;}
    const char *capacity=rg_capacity(&s->space,1);if(capacity){f_fail(s->error,capacity,"changeset registration exceeds namespace capacity");return 0;}
    char pending[96];snprintf(pending,sizeof pending,".pending-%s",s->id);
    if(mkdirat(s->base,pending,0700)<0)return 0;s->dir=openat(s->base,pending,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);if(!private_dir(s->dir))return 0;
    s->manifest=j_new(J_OBJECT);j_value *files=j_new(J_ARRAY);if(!s->manifest || !files)return 0;
    if(!j_integer(s->manifest,"version",1) || !j_text(s->manifest,"generation",s->space.generation,32) || !j_text(s->manifest,"id",s->id,strlen(s->id)) || !j_integer(s->manifest,"expires_unix",s->expiry) || !j_text(s->manifest,"status","registered",10) || !j_boolean(s->manifest,"cleanup_pending",0) || !j_add(s->manifest,"files",files))return 0;
    j_buf raw={0};raw.limit=J_INPUT_LIMIT;if(!j_dump(s->request,&raw)){free(raw.data);return 0;}
    unsigned char digest[32];char hash[65];rh_sha256(raw.data,raw.size,digest);rh_hex(digest,32,hash);
    int ok=j_text(s->manifest,"fingerprint",hash,64) && rh_atomic(s->dir,"request.json",raw.data,raw.size);free(raw.data);if(!ok)return 0;
    for(j_value *p=j_get(s->request,"changes")->first;p;p=p->next){j_value *entry=j_new(J_OBJECT);if(!entry || !j_text(entry,"path",ch_text(p,"path"),strlen(ch_text(p,"path"))) || !j_text(entry,"state","pending",7) || !j_boolean(entry,"ever_applied",0) || !j_add(files,NULL,entry))return 0;}
    int fd=openat(s->dir,"owner.lock",O_RDWR|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);if(fd<0)return 0;ok=fsync(fd)==0;close(fd);
    if(!ok || !ch_save(s) || renameat(s->base,pending,s->base,s->id)<0 || fsync(s->base)<0)return 0;
    return ch_load(s)?2:0;
}
void ch_close(ch_set *s){for(size_t i=0;i<s->count;i++){if(s->entries[i].dir>=0)close(s->entries[i].dir);free(s->entries[i].name);}if(s->owner>=0)close(s->owner);if(s->dir>=0)close(s->dir);if(s->base>=0)close(s->base);rh_state_close(&s->space);j_free(s->manifest);j_free(s->request);}
j_value *ch_result(ch_set *s,int active){
    j_value *result=j_new(J_OBJECT),*record=j_new(J_OBJECT),*files=j_new(J_ARRAY);if(!result || !record || !files)goto fail;
    if(!j_text(record,"generation",s->space.generation,32) || !j_text(record,"id",s->id,strlen(s->id)) || !j_text(record,"fingerprint",ch_text(s->manifest,"fingerprint"),64) || !j_integer(record,"expires_unix",s->expiry) || !j_text(record,"status",ch_text(s->manifest,"status"),strlen(ch_text(s->manifest,"status"))) || !j_boolean(record,"owner_active",active) || !j_boolean(record,"cleanup_pending",(j_get(s->manifest,"cleanup_pending") && j_get(s->manifest,"cleanup_pending")->number)))goto fail;
    size_t applied=0;int unknown=active || !strcmp(ch_text(s->manifest,"status"),"unknown");
    for(size_t i=0;i<s->count;i++){j_value *m=s->entries[i].meta,*v=j_new(J_OBJECT);int ever=j_get(m,"ever_applied") && j_get(m,"ever_applied")->number;applied+=ever?1:0;
        const char *phase=ch_text(m,"state");if(!strcmp(phase,"installing") || !strcmp(phase,"restoring"))unknown=1;
        if(!v || !j_text(v,"path",ch_text(m,"path"),strlen(ch_text(m,"path"))) || !j_text(v,"state",ch_text(m,"state"),strlen(ch_text(m,"state"))) || !j_boolean(v,"ever_applied",ever))goto fail;
        const char *keys[]={"before_hash","after_hash"};for(size_t k=0;k<2;k++){j_value *h=j_get(m,keys[k]);if(!j_add(v,keys[k],h?j_clone(h):j_new(J_NULL))){j_free(v);goto fail;}}
        if(!j_add(files,NULL,v))goto fail;
    }
    const char *mutation=unknown?"unknown":!applied?"not_applied":applied==s->count?"applied":"partial";s->error->mutation=mutation;
    if(!j_text(record,"mutation",mutation,strlen(mutation)) || !j_take(record,"files",&files))goto fail;files=NULL;
    j_value *failure=j_get(s->manifest,"failure");if(!j_add(record,"failure",failure?j_clone(failure):j_new(J_NULL)))goto fail;
    if(!j_take(result,"record",&record))goto fail;record=NULL;
    j_value *error=j_new(s->error->code?J_OBJECT:J_NULL);if(!error)goto fail;
    if(s->error->code && (!j_text(error,"code",s->error->code,strlen(s->error->code)) || !j_text(error,"message",s->error->message,strlen(s->error->message)) || !j_text(error,"mutation",mutation,strlen(mutation)))){j_free(error);goto fail;}
    if(!j_add(result,"error",error))goto fail;return result;
fail:j_free(files);j_free(record);j_free(result);return NULL;
}
