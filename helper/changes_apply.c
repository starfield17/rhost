#include "changes.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static int existed(ch_entry *p){j_value *v=j_get(p->meta,"existed");return v && v->type==J_BOOL && v->number;}
static int observed(ch_set *s,ch_entry *p,const char *version,const char *hash,int missing,int intent){f_error error={NULL,NULL,"not_applied"};ch_set probe=*s;probe.error=&error;return ch_check(&probe,p,j_get(p->meta,version),ch_text(p->meta,hash),missing,intent);}
static int remember(ch_entry *p,const char *key){struct stat st;return fstatat(p->dir,p->name,&st,AT_SYMLINK_NOFOLLOW)==0 && ch_put(p->meta,key,ch_version(&st));}
int ch_reconcile(ch_set *s){
    for(size_t i=0;i<s->count;i++){
        ch_entry *p=&s->entries[i];const char *phase=ch_text(p->meta,"state");
        if(!strcmp(phase,"installing")){
            if(observed(s,p,"stage","after_hash",0,1)){
                if(!ch_cleanup(s) || !remember(p,"installed") || !ch_string(p->meta,"state","installed") || !ch_bool(p->meta,"ever_applied",1))return 0;
            }else if(observed(s,p,"before","before_hash",!existed(p),0)){if(!ch_string(p->meta,"state","pending"))return 0;}
            else{f_fail(s->error,"FILE_CONFLICT","cannot reconcile interrupted target replacement");return 0;}
        }else if(!strcmp(phase,"restoring")){
            if(observed(s,p,"stage","before_hash",!existed(p),1)){
                if(!ch_cleanup(s))return 0;
                if(existed(p) && !remember(p,"restored"))return 0;if(!ch_string(p->meta,"state","restored"))return 0;
            }else if(observed(s,p,"installed","after_hash",0,0)){if(!ch_string(p->meta,"state","installed"))return 0;}
            else{f_fail(s->error,"FILE_CONFLICT","cannot reconcile interrupted rollback");return 0;}
        }
    }
    return ch_save(s);
}
int ch_cleanup(ch_set *s){
    int ok=1;
    for(size_t i=0;i<s->count;i++){
        ch_entry *p=&s->entries[i];const char *name=ch_text(p->meta,"temporary");if(!*name)continue;
        if(strncmp(name,".rhost-change-",14) || strchr(name,'/')){ok=0;break;}
        struct stat st;if(fstatat(p->dir,name,&st,AT_SYMLINK_NOFOLLOW)==0){
            j_value *v=j_get(p->meta,"stage"),*a=v?v->first:NULL;
            if(!a || !a->next || a->number!=(uint64_t)st.st_dev || a->next->number!=(uint64_t)st.st_ino || !S_ISREG(st.st_mode) || unlinkat(p->dir,name,0)<0 || fsync(p->dir)<0){ok=0;break;}
        }else if(errno!=ENOENT){ok=0;break;}
        if(!ch_string(p->meta,"temporary","")){ok=0;break;}
        /* Keep the staged identity until the installed/restored marker is durable. */
    }
    if(!ch_bool(s->manifest,"cleanup_pending",!ok))return 0;
    if(!ok)f_fail(s->error,"FILE_CONFLICT","staging cleanup needs preserved inode evidence");
    return ch_save(s) && ok;
}
static int stage(ch_set *s,ch_entry *p,const j_buf *bytes,mode_t mode){
    char token[33],name[64];if(!rh_random(token,16))return 0;snprintf(name,sizeof name,".rhost-change-%s",token);
    if(!ch_string(p->meta,"temporary",name) || !ch_put(p->meta,"stage",j_new(J_NULL)) || !ch_bool(s->manifest,"cleanup_pending",1) || !ch_save(s))return 0;
    int fd=openat(p->dir,name,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);if(fd<0){f_os_error(s->error,0);return 0;}
    struct stat st;int ok=fstat(fd,&st)==0 && ch_put(p->meta,"stage",ch_version(&st)) && ch_save(s);
    if(ok)ok=f_save(fd,bytes->data,bytes->size) && fchmod(fd,mode)==0 && fsync(fd)==0 && fstat(fd,&st)==0 && ch_put(p->meta,"stage",ch_version(&st)) && fsync(p->dir)==0 && ch_save(s);
    close(fd);if(!ok)f_fail(s->error,"STATE_UNAVAILABLE","cannot durably stage replacement");return ok;
}
static int replace(ch_set *s,size_t i,int rollback){
    ch_entry *p=&s->entries[i];int removing=rollback && !existed(p);j_buf bytes={0};
    const char *hash=rollback?"before_hash":"after_hash";
    mode_t mode=(mode_t)ch_num(p->meta,"mode");if(rollback){j_value *v=j_get(p->meta,"before");j_value *a=v?v->first:NULL;for(unsigned k=0;a && k<3;k++)a=a->next;if(a)mode=(mode_t)a->number;}
    if(!removing && (!ch_bytes(s,i,rollback?"before":"after",&bytes,ch_text(p->meta,hash)) || !stage(s,p,&bytes,mode))){free(bytes.data);return 0;}free(bytes.data);
    if(!ch_check(s,p,j_get(p->meta,rollback?"installed":"before"),ch_text(p->meta,rollback?"after_hash":"before_hash"),!rollback && !existed(p),0))return 0;
    if(!ch_string(p->meta,"state",rollback?"restoring":"installing") || !ch_save(s) || !f_context(s->error))return 0;
    s->error->mutation="unknown";int ok;
    if(removing)ok=unlinkat(p->dir,p->name,0)==0;
    else if(rollback || existed(p))ok=renameat(p->dir,ch_text(p->meta,"temporary"),p->dir,p->name)==0;
    else ok=linkat(p->dir,ch_text(p->meta,"temporary"),p->dir,p->name,0)==0;
    if(!ok){f_os_error(s->error,1);return 0;}
    if(!rollback && !ch_bool(p->meta,"ever_applied",1))return 0;
    if(fsync(p->dir)<0){f_fail(s->error,"STATE_UNAVAILABLE","target directory sync failed");return 0;}
    if(!removing && !ch_check(s,p,j_get(p->meta,"stage"),ch_text(p->meta,hash),0,1))return 0;
    if(!ch_cleanup(s))return 0;
    if(!removing && !remember(p,rollback?"restored":"installed"))return 0;
    if(!ch_string(p->meta,"state",rollback?"restored":"installed") || !ch_save(s))return 0;
    return ch_cleanup(s);
}
int ch_apply(ch_set *s,int rollback){
    const char *status=ch_text(s->manifest,"status");
    if((!rollback && !strcmp(status,"applied")) || (rollback && !strcmp(status,"rolled_back")))return 1;
    if(!rollback && !strcmp(status,"rolled_back")){f_fail(s->error,"FILE_CONFLICT","a restored changeset cannot be applied again");return 0;}
    if(!ch_reconcile(s) || !ch_cleanup(s))return 0;
    /* All guarded versions pass before the first target change of this action. */
    for(size_t i=0;i<s->count;i++){
        ch_entry *p=&s->entries[i];const char *phase=ch_text(p->meta,"state");
        if(!strcmp(phase,"installed")){if(!ch_check(s,p,j_get(p->meta,"installed"),ch_text(p->meta,"after_hash"),0,0))return 0;}
        else if(!rollback && !strcmp(phase,"pending")){if(!ch_check(s,p,j_get(p->meta,"before"),ch_text(p->meta,"before_hash"),!existed(p),0))return 0;}
        else if(!rollback && !strcmp(phase,"restored")){f_fail(s->error,"FILE_CONFLICT","partially restored changeset cannot resume");return 0;}
    }
    if(!ch_string(s->manifest,"status",rollback?"rolling_back":"applying") || !ch_save(s))return 0;
    for(size_t k=0;k<s->count;k++){
        size_t i=rollback?s->count-1-k:k;const char *phase=ch_text(s->entries[i].meta,"state");
        if((rollback && !strcmp(phase,"installed")) || (!rollback && !strcmp(phase,"pending")))if(!replace(s,i,rollback))return 0;
    }
    if(!ch_string(s->manifest,"status",rollback?"rolled_back":"applied") || !ch_put(s->manifest,"failure",j_new(J_NULL)))return 0;return ch_save(s);
}
