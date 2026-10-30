#include "changes.h"
#include "registry.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>

static int hash(const char *s){if(!*s)return 1;if(strlen(s)!=64)return 0;for(size_t i=0;i<64;i++)if(!((s[i]>='0' && s[i]<='9')||(s[i]>='a' && s[i]<='f')))return 0;return 1;}
static j_value *intent(const j_value *q,f_error *e){
    j_value *copy=j_clone(q);if(!copy)return f_fail(e,"INTERNAL","cannot copy changeset intent");j_value *list=j_get(copy,"changes");size_t count=0,total=0;
    if(!list || list->type!=J_ARRAY)goto invalid;
    for(j_value *p=list->first;p;p=p->next){
        const char *op=ch_text(p,"op");int patch=!strcmp(op,"patch");
        if(++count>CH_COUNT || (!patch && strcmp(op,"write")) || !f_fields(p,patch?"op path if_hash edits":"op path if_hash content file_mode"))goto invalid;
        j_value *expected=j_get(p,"if_hash");if(expected && (!j_string(expected) || !hash(j_string(expected))))goto invalid;
        if(patch && (!expected || !*j_string(expected) || !j_get(p,"edits") || j_get(p,"edits")->type!=J_ARRAY || !j_get(p,"edits")->first))goto invalid;
        mode_t mode=0600;if(!f_mode(p,&mode,e)){j_free(copy);return NULL;}
        char *path=f_path(p,e);if(!path){j_free(copy);return NULL;}int ok=ch_string(p,"path",path);free(path);if(!ok)goto invalid;
        if(!patch){j_buf bytes={0};ok=f_decode(j_get(p,"content"),&bytes,e);if(ok && bytes.size<=F_EDIT_LIMIT-total)total+=bytes.size;else ok=0;free(bytes.data);if(!ok){j_free(copy);return f_fail(e,"FILE_TOO_LARGE","invalid or oversized changeset replacement");}}
    }
    if(!count)goto invalid;j_value *sorted=ch_canonical(copy);j_free(copy);return sorted;
invalid:j_free(copy);return f_fail(e,"CONFIG_INVALID","invalid closed changeset intent");
}
static int pending_cleanup(const ch_set *s){j_value *v=j_get(s->manifest,"cleanup_pending");return !v || v->type!=J_BOOL || v->number;}
int ch_prune(ch_set *s,uint64_t now){
    const char *status=ch_text(s->manifest,"status");
    if(now<s->expiry || pending_cleanup(s) || (strcmp(status,"applied") && strcmp(status,"rolled_back") && strcmp(status,"refused"))){f_fail(s->error,"FILE_CONFLICT","changeset is unexpired or recovery evidence must be retained");return 0;}
    return rg_remove(&s->space,1,s->id,s->dir,s->error);
}
int ch_can_prune(int directory,const char *generation,const char *id){
    f_error error={NULL,NULL,"not_applied"};ch_set s;memset(&s,0,sizeof s);s.error=&error;s.space.root=s.space.runs=s.space.registry=s.base=s.owner=-1;s.dir=dup(directory);s.id=(char *)id;
    for(size_t i=0;i<CH_COUNT;i++)s.entries[i].dir=-1;
    if(!rh_key(id,&s.expiry)){ch_close(&s);return 0;}strcpy(s.space.generation,generation);
    int ok=s.dir>=0 && ch_load(&s);const char *status=ok?ch_text(s.manifest,"status"):"";
    ok=ok && !pending_cleanup(&s) && (!strcmp(status,"applied") || !strcmp(status,"rolled_back") || !strcmp(status,"refused"));
    ch_close(&s);return ok;
}
static void failure(ch_set *s,int preparing){
    int ambiguous=0,applied=0;for(size_t i=0;i<s->count;i++){const char *state=ch_text(s->entries[i].meta,"state");if(!strcmp(state,"installing") || !strcmp(state,"restoring"))ambiguous=1;j_value *ever=j_get(s->entries[i].meta,"ever_applied");if(ever && ever->number)applied=1;}
    if(!s->error->code)f_fail(s->error,"STATE_UNAVAILABLE","changeset evidence is incomplete");
    if(!ch_string(s->manifest,"status",ambiguous?"unknown":preparing && !applied?"refused":"partial"))return;
    j_value *err=j_new(J_OBJECT);if(err && j_text(err,"code",s->error->code,strlen(s->error->code)) && j_text(err,"message",s->error->message,strlen(s->error->message)) && ch_put(s->manifest,"failure",err))(void)ch_save(s);
}
j_value *f_changes(const j_value *q,const char *op,f_error *e){
    int applying=!strcmp(op,"changeset.apply"),query=!strcmp(op,"changeset.query"),recover=!strcmp(op,"changeset.recover"),prune=!strcmp(op,"changeset.prune");
    if(!applying && !query && !recover && !prune)return f_fail(e,"CONFIG_INVALID","unknown changeset operation");
    const char *allowed=applying?"op generation key expires_unix changes":recover?"op generation id action":"op generation id";
    const char *id=ch_text(q,applying?"key":"id"),*generation=ch_text(q,"generation"),*action=ch_text(q,"action");uint64_t expiry=0;
    if(!f_fields(q,allowed) || !rh_generation(generation) || !rh_key(id,&expiry) || (applying && ch_num(q,"expires_unix")!=expiry) || (recover && strcmp(action,"resume") && strcmp(action,"rollback")))return f_fail(e,"CONFIG_INVALID","invalid changeset identity or action");
    ch_set s;memset(&s,0,sizeof s);s.space.root=s.space.runs=s.space.registry=s.base=s.dir=s.owner=-1;s.id=(char *)id;s.expiry=expiry;s.error=e;
    for(size_t i=0;i<CH_COUNT;i++)s.entries[i].dir=-1;
    char fingerprint[65]={0};
    if(applying){s.request=intent(q,e);if(!s.request)return NULL;j_buf raw={0};raw.limit=J_INPUT_LIMIT;if(!j_dump(s.request,&raw)){free(raw.data);ch_close(&s);return f_fail(e,"INTERNAL","cannot fingerprint changeset");}unsigned char digest[32];rh_sha256(raw.data,raw.size,digest);rh_hex(digest,32,fingerprint);free(raw.data);}
    if(!f_context(e)){ch_close(&s);return NULL;}
    int opened=ch_open(&s,q,applying);j_value *result=NULL;
    if(!opened){if(!e->code)f_fail(e,"STATE_UNAVAILABLE","changeset namespace or journal is unavailable");if(strcmp(e->code,"STATE_CAPACITY_EXCEEDED"))e->mutation="unknown";goto done;}
    if(applying && strcmp(fingerprint,ch_text(s.manifest,"fingerprint"))){f_fail(e,"IDEMPOTENCY_CONFLICT","key already binds a different changeset");goto done;}
    s.owner=openat(s.dir,"owner.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC);struct stat owner;
    if(s.owner<0 || fstat(s.owner,&owner)<0 || !S_ISREG(owner.st_mode) || owner.st_uid!=geteuid() || (owner.st_mode&0077)){f_fail(e,"STATE_UNAVAILABLE","changeset ownership is unavailable");goto done;}
    int active=flock(s.owner,LOCK_EX|LOCK_NB)<0;
    if(active && errno!=EAGAIN && errno!=EWOULDBLOCK){f_fail(e,"STATE_UNAVAILABLE","cannot observe changeset ownership");goto done;}
    uint64_t prune_now=0;
    if(prune && !active && !rh_clock(&s.space,&prune_now)){f_fail(e,"STATE_UNAVAILABLE","cannot persist expiry clock");goto done;}
    if(!prune && s.space.registry>=0){close(s.space.registry);s.space.registry=-1;}
    if(query || (applying && opened==1)){result=ch_result(&s,active);goto done;}
    if(active){f_fail(e,"STATE_UNAVAILABLE","changeset has an active owner");result=ch_result(&s,1);goto done;}
    if(!ch_load(&s)){f_fail(e,"STATE_UNAVAILABLE","changeset journal changed or is corrupt");e->mutation="unknown";goto done;}
    if(prune){
        if(!ch_prune(&s,prune_now)){result=ch_result(&s,0);goto done;}result=j_new(J_OBJECT);if(result && (!j_text(result,"generation",generation,32) || !j_text(result,"id",id,strlen(id)) || !j_boolean(result,"pruned",1))){j_free(result);result=NULL;}goto done;}
    if(!rh_clear_atomic(s.dir)){f_fail(e,"STATE_UNAVAILABLE","abandoned journal writes cannot be reconciled");e->mutation="unknown";result=ch_result(&s,0);goto done;}
    int preparing=!strcmp(ch_text(s.manifest,"status"),"registered") || !strcmp(ch_text(s.manifest,"status"),"refused");
    if(!ch_parents(&s)){failure(&s,preparing);result=ch_result(&s,0);goto done;}
    if(preparing && !ch_prepare(&s)){failure(&s,1);result=ch_result(&s,0);goto done;}
    if(!ch_apply(&s,recover && !strcmp(action,"rollback")))failure(&s,0);
    result=ch_result(&s,0);
done:ch_close(&s);return result;
}
