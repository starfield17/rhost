#include "registry.h"
#include "changes.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>
static int known(const char *name,int kind){
    if(!strncmp(name,".write-",7) && rh_generation(name+7))return 1;
    if(!strcmp(name,"owner.lock"))return 1;
    if(kind==0){const char *names[]={"request","fingerprint","stdin","stdout","stderr","status","control.sock"};for(size_t i=0;i<sizeof names/sizeof names[0];i++)if(!strcmp(name,names[i]))return 1;}
    else if(kind==1){
        if(!strcmp(name,"manifest.json") || !strcmp(name,"request.json"))return 1;
        const char *number=!strncmp(name,"before-",7)?name+7:!strncmp(name,"after-",6)?name+6:NULL;
        if(number && *number){unsigned n=0;for(const char *p=number;*p;p++){if(*p<'0'||*p>'9')return 0;n=n*10+(unsigned)(*p-'0');if(n>=64)return 0;}return 1;}
    }else if(!strcmp(name,"meta.json") || !strcmp(name,"paths") || !strcmp(name,"paths.index"))return 1;
    return 0;
}
/* Caller holds the namespace registry and record owner, or a pruning tombstone. */
int rg_remove(rh_state *s,int kind,const char *id,int directory,f_error *e){
    int base=rg_base(s,kind,0);if(base<0)return f_fail(e,"STATE_UNAVAILABLE","record parent is unavailable"),0;
    char pending[96],tomb[96];snprintf(pending,sizeof pending,".pending-%s",id);snprintf(tomb,sizeof tomb,".pruned-%s",id);
    const char *names[]={id,pending,tomb},*current=NULL;struct stat held,observed;
    if(fstat(directory,&held)<0){close(base);return f_fail(e,"STATE_UNAVAILABLE","record identity is unavailable"),0;}
    for(unsigned i=0;i<3;i++)if(fstatat(base,names[i],&observed,AT_SYMLINK_NOFOLLOW)==0 && observed.st_dev==held.st_dev && observed.st_ino==held.st_ino){current=names[i];break;}
    if(!current){close(base);return f_fail(e,"FILE_CONFLICT","record identity changed"),0;}
    int copy=dup(directory);DIR *dir=copy<0?NULL:fdopendir(copy);if(!dir){if(copy>=0)close(copy);close(base);return f_fail(e,"STATE_UNAVAILABLE","cannot enumerate record bytes"),0;}
    char *files[1024];size_t count=0;struct dirent *p;int ok=1;
    while((errno=0,p=readdir(dir))){
        if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
        struct stat st;if(count==1024 || !known(p->d_name,kind) || fstatat(directory,p->d_name,&st,AT_SYMLINK_NOFOLLOW)<0 || st.st_uid!=geteuid() || (!S_ISREG(st.st_mode) && !(kind==0 && S_ISSOCK(st.st_mode) && !strcmp(p->d_name,"control.sock")))){ok=0;break;}
        files[count]=strdup(p->d_name);if(!files[count]){ok=0;break;}count++;
    }
    if(errno)ok=0;closedir(dir);
    if(ok && strcmp(current,tomb)){
        if(fstatat(base,tomb,&observed,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT)ok=0;
        else if(renameat(base,current,base,tomb)<0 || fsync(base)<0)ok=0;
    }
    if(ok)for(size_t i=0;i<count;i++)if(unlinkat(directory,files[i],0)<0 && errno!=ENOENT){ok=0;break;}
    for(size_t i=0;i<count;i++)free(files[i]);
    if(ok)ok=fsync(directory)==0 && unlinkat(base,tomb,AT_REMOVEDIR)==0 && fsync(base)==0;
    close(base);if(!ok)f_fail(e,"STATE_UNAVAILABLE","pruning is incomplete; retain the tombstone and retry explicitly");return ok;
}
static j_value *receipt(const char *id,int eligible,const char *reason,int pruned,const f_error *e){
    j_value *v=j_new(J_OBJECT),*error=j_new(e->code?J_OBJECT:J_NULL);if(!v || !error)goto fail;
    if(e->code && (!j_text(error,"code",e->code,strlen(e->code)) || !j_text(error,"message",e->message,strlen(e->message))))goto fail;
    if(!j_text(v,"id",id,strlen(id)) || !j_boolean(v,"eligible",eligible) || !j_text(v,"reason",reason,strlen(reason)) || !j_boolean(v,"pruned",pruned) || !j_take(v,"error",&error))goto fail;return v;
fail:j_free(v);j_free(error);return NULL;
}
j_value *rg_prune(rh_state *s,const j_value *q,f_error *e){
    int kind=rg_kind(j_string(j_get(q,"kind")));j_value *ids=j_get(q,"ids"),*dry=j_get(q,"dry_run");size_t count=0;
    if(kind<0 || !ids || ids->type!=J_ARRAY || !ids->first || (dry && dry->type!=J_BOOL))return f_fail(e,"CONFIG_INVALID","prune needs a kind and explicit IDs");
    for(j_value *p=ids->first;p;p=p->next){uint64_t expiry;const char *id=j_string(p);if(++count>64 || !rh_key(id,&expiry))return f_fail(e,"CONFIG_INVALID","invalid prune identity");for(j_value *a=ids->first;a!=p;a=a->next)if(!strcmp(j_string(a),id))return f_fail(e,"CONFIG_INVALID","duplicate prune identity");}
    int preview=!dry || dry->number;uint64_t now;
    if(!(preview?rh_clock_read(s,&now):rh_clock(s,&now)))return f_fail(e,"STATE_UNAVAILABLE","effective prune clock is unavailable");
    j_value *items=j_new(J_ARRAY),*out=j_new(J_OBJECT);if(!items || !out)goto fail;unsigned failed=0;
    for(j_value *p=ids->first;p;p=p->next){
        const char *id=j_string(p);f_error error={NULL,NULL,"not_applied"};int base=rg_base(s,kind,0),directory=-1,owner=-1,eligible=0,removed=0;const char *reason="not_found";
        char pending[96],tomb[96];snprintf(pending,sizeof pending,".pending-%s",id);snprintf(tomb,sizeof tomb,".pruned-%s",id);const char *names[]={id,pending,tomb};rg_record record={0};
        if(base<0){struct stat st;if(fstatat(s->root,rg_parent(kind),&st,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT)f_fail(&error,"STATE_UNAVAILABLE","record parent is unavailable");}
        else for(unsigned i=0;i<3;i++){
            directory=openat(base,names[i],O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
            if(directory<0){if(errno!=ENOENT)f_fail(&error,"STATE_UNAVAILABLE","record directory cannot be inspected");continue;}
            if(!rg_private(directory) || !rg_observe(base,names[i],kind,s->generation,&record)){f_fail(&error,"STATE_UNAVAILABLE","record identity is unavailable");break;}
            owner=openat(directory,"owner.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC);
            if(owner>=0){struct stat st;if(fstat(owner,&st)<0 || !S_ISREG(st.st_mode) || st.st_uid!=geteuid() || (st.st_mode&0077)){f_fail(&error,"STATE_UNAVAILABLE","record owner is invalid");break;}if(flock(owner,LOCK_EX|LOCK_NB)<0){if(errno==EAGAIN || errno==EWOULDBLOCK)reason="owner_active";else f_fail(&error,"STATE_UNAVAILABLE","record ownership cannot be established");break;}}
            else if(record.phase==0 || errno!=ENOENT){f_fail(&error,"STATE_UNAVAILABLE","record owner evidence is unavailable");break;}
            if(record.expiry>now){reason="not_expired";break;}
            if(record.phase)eligible=1;
            else if(record.error){f_fail(&error,"STATE_UNAVAILABLE","record evidence is unavailable");break;}
            else if(kind==0)eligible=!strcmp(record.status,"exited");
            else if(kind==1)eligible=ch_can_prune(directory,s->generation,id);
            else eligible=1;
            reason=eligible?"expired_terminal":"recovery_required";
            if(eligible && !preview){removed=rg_remove(s,kind,id,directory,&error);if(!removed)reason="prune_incomplete";}
            break;
        }
        if(error.code){failed++;if(!strcmp(reason,"not_found"))reason="evidence_unavailable";}
        j_value *row=receipt(id,eligible,reason,removed,&error);if(owner>=0)close(owner);if(directory>=0)close(directory);if(base>=0)close(base);
        if(!row || !j_add(items,NULL,row))goto fail;
    }
    if(!j_text(out,"generation",s->generation,32) || !j_text(out,"kind",rg_label(kind),strlen(rg_label(kind))) || !j_boolean(out,"dry_run",preview) || !j_integer(out,"failed",failed) || !j_take(out,"items",&items))goto fail;return out;
fail:j_free(items);j_free(out);return f_fail(e,"INTERNAL","cannot allocate bounded prune receipts");
}
