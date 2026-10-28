#include "changes.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>
static int order(const void *a,const void *b){const ch_entry *x=*(const ch_entry *const *)a,*y=*(const ch_entry *const *)b;if(x->parent.st_dev!=y->parent.st_dev)return x->parent.st_dev<y->parent.st_dev?-1:1;return x->parent.st_ino==y->parent.st_ino?0:x->parent.st_ino<y->parent.st_ino?-1:1;}
int ch_parents(ch_set *s){
    ch_entry *ordered[CH_COUNT];
    for(size_t i=0;i<s->count;i++){
        ch_entry *p=&s->entries[i];p->dir=f_parent(p->path,0,&p->name,s->error);if(p->dir<0 || fstat(p->dir,&p->parent)<0)return 0;
        j_value *recorded=j_get(p->meta,"parent");
        if(recorded){j_value *a=recorded->first;if(recorded->type!=J_ARRAY || !a || !a->next || a->number!=(uint64_t)p->parent.st_dev || a->next->number!=(uint64_t)p->parent.st_ino){f_fail(s->error,"FILE_CONFLICT","parent directory identity changed");return 0;}}
        for(size_t k=0;k<i;k++)if(p->parent.st_dev==s->entries[k].parent.st_dev && p->parent.st_ino==s->entries[k].parent.st_ino && !strcmp(p->name,s->entries[k].name)){f_fail(s->error,"CONFIG_INVALID","duplicate directory entry in changeset");return 0;}
        ordered[i]=p;
    }
    qsort(ordered,s->count,sizeof ordered[0],order);
    for(size_t i=0;i<s->count;i++){
        if(i && !order(&ordered[i-1],&ordered[i]))continue;
        if(flock(ordered[i]->dir,LOCK_EX)<0){f_fail(s->error,"STATE_UNAVAILABLE","cannot lock target parent");return 0;}ordered[i]->lock=1;
    }
    return 1;
}
int ch_check(ch_set *s,ch_entry *p,const j_value *version,const char *hash,int missing,int intent){
    struct stat st;
    if(fstatat(p->dir,p->name,&st,AT_SYMLINK_NOFOLLOW)<0){if(errno==ENOENT && missing)return 1;f_fail(s->error,"FILE_CONFLICT","target is missing or inaccessible");return 0;}
    if(missing || !S_ISREG(st.st_mode) || !ch_same(version,&st,intent)){f_fail(s->error,"FILE_CONFLICT","target revision changed");return 0;}
    int fd=f_open(p->dir,p->name,1,s->error);if(fd<0)return 0;char observed[65];struct stat after;
    int ok=f_hash_fd(fd,observed,NULL,s->error) && fstat(fd,&after)==0 && f_identity(&st,&after) && !strcmp(hash,observed);close(fd);
    if(!ok)f_fail(s->error,"FILE_CONFLICT","target bytes changed");return ok;
}
static int original(ch_set *s,ch_entry *p,j_buf *old,struct stat *st,int *exists){
    if(fstatat(p->dir,p->name,st,AT_SYMLINK_NOFOLLOW)<0){if(errno==ENOENT){*exists=0;return 1;}f_os_error(s->error,0);return 0;}
    *exists=1;if(!S_ISREG(st->st_mode)){f_fail(s->error,"INVALID_TARGET","changeset target is not a regular file");return 0;}
    if(st->st_size<0 || (uint64_t)st->st_size>F_EDIT_LIMIT){f_fail(s->error,"FILE_TOO_LARGE","changeset original exceeds byte limit");return 0;}
    int fd=f_open(p->dir,p->name,1,s->error);if(fd<0)return 0;struct stat before,after;
    int ok=fstat(fd,&before)==0 && f_identity(st,&before) && f_load(fd,old,s->error) && fstat(fd,&after)==0 && f_identity(&before,&after);close(fd);
    if(!ok)f_fail(s->error,"FILE_CONFLICT","changeset original changed while reading");return ok;
}
int ch_prepare(ch_set *s){
    size_t originals=0,replacements=0;
    for(size_t i=0;i<s->count;i++){
        ch_entry *p=&s->entries[i];j_buf old={0},next={0};struct stat st;int exists=0;int ok=original(s,p,&old,&st,&exists);
        const char *expected=ch_text(p->intent,"if_hash");unsigned char digest[32];char before[65],after[65];
        rh_sha256(old.data,old.size,digest);rh_hex(digest,32,before);
        int patch=!strcmp(ch_text(p->intent,"op"),"patch");
        if(ok && ((exists && (!*expected || strcmp(before,expected))) || (!exists && (*expected || patch)))){f_fail(s->error,exists && !*expected?"HASH_REQUIRED":"FILE_CONFLICT","changeset preflight hash failed");ok=0;}
        mode_t mode=exists?st.st_mode&07777:0600;
        if(ok)ok=f_mode(p->intent,&mode,s->error) && (patch?f_patch(j_get(p->intent,"edits"),&old,&next,s->error):f_decode(j_get(p->intent,"content"),&next,s->error));
        if(ok && (old.size>F_EDIT_LIMIT-originals || next.size>F_EDIT_LIMIT-replacements)){f_fail(s->error,"FILE_TOO_LARGE","aggregate changeset bytes exceed 8388608");ok=0;}
        if(ok){
            originals+=old.size;replacements+=next.size;rh_sha256(next.data,next.size,digest);rh_hex(digest,32,after);
            j_value *parent=j_new(J_ARRAY);ok=parent && j_integer(parent,NULL,(uint64_t)p->parent.st_dev) && j_integer(parent,NULL,(uint64_t)p->parent.st_ino) && ch_put(p->meta,"parent",parent);
            if(ok)ok=ch_bool(p->meta,"existed",exists) && ch_put(p->meta,"before",exists?ch_version(&st):j_new(J_NULL)) && ch_string(p->meta,"before_hash",before) && ch_string(p->meta,"after_hash",after) && ch_integer(p->meta,"mode",mode) && ch_archive(s,i,"before",&old) && ch_archive(s,i,"after",&next);
        }
        free(old.data);free(next.data);if(!ok)return 0;
    }
    if(!ch_string(s->manifest,"status","prepared"))return 0;return ch_save(s);
}
