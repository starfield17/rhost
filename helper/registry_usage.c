#include "registry.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>
int rg_kind(const char *name){if(!name)return -1;return !strcmp(name,"runs")?0:!strcmp(name,"changesets")?1:!strcmp(name,"queries")?2:-1;}
const char *rg_parent(int kind){return kind==0?"runs":kind==1?"changes":"queries";}
const char *rg_label(int kind){return kind==0?"runs":kind==1?"changesets":"queries";}
int rg_private(int fd){struct stat st;return fd>=0 && fstat(fd,&st)==0 && S_ISDIR(st.st_mode) && st.st_uid==geteuid() && !(st.st_mode&0077);}
int rg_base(rh_state *s,int kind,int create){
    if(kind<0 || kind>2)return -1;
    if(create && mkdirat(s->root,rg_parent(kind),0700)<0 && errno!=EEXIST)return -1;
    int fd=openat(s->root,rg_parent(kind),O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(!rg_private(fd)){if(fd>=0)close(fd);return -1;}return fd;
}
int rg_identity(const char *name,char id[80],uint64_t *expiry,unsigned *phase){
    *phase=0;if(!strncmp(name,".pending-",9)){name+=9;*phase=1;}else if(!strncmp(name,".pruned-",8)){name+=8;*phase=2;}
    if(!rh_key(name,expiry) || strlen(name)>=80)return 0;strcpy(id,name);return 1;
}
static int logical(int dir,uint64_t *bytes){
    *bytes=0;int copy=dup(dir);DIR *entries=copy<0?NULL:fdopendir(copy);if(!entries){if(copy>=0)close(copy);return 0;}
    struct dirent *p;int ok=1;size_t count=0;
    while((errno=0,p=readdir(entries))){if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
        struct stat st;if(++count>1024 || fstatat(dir,p->d_name,&st,AT_SYMLINK_NOFOLLOW)<0 || st.st_uid!=geteuid()){ok=0;break;}
        if(S_ISSOCK(st.st_mode) && !strcmp(p->d_name,"control.sock"))continue;
        if(!S_ISREG(st.st_mode) || st.st_size<0 || UINT64_MAX-*bytes<(uint64_t)st.st_size){ok=0;break;}*bytes+=(uint64_t)st.st_size;
    }
    if(errno)ok=0;closedir(entries);return ok;
}
static const char *text(const j_value *o,const char *name){const char *s=j_string(j_get(o,name));return s?s:"";}
int rg_observe(int base,const char *name,int kind,const char *generation,rg_record *r){
    memset(r,0,sizeof *r);if(!rg_identity(name,r->id,&r->expiry,&r->phase))return 0;
    strcpy(r->status,r->phase==1?"building":r->phase==2?"pruning":"unknown");
    int dir=openat(base,name,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(!rg_private(dir)){r->error="STATE_UNAVAILABLE";if(dir>=0)close(dir);return 1;}
    int owner=openat(dir,"owner.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC);int missing=owner<0 && errno==ENOENT;struct stat held;
    if(owner>=0 && (fstat(owner,&held)<0 || !S_ISREG(held.st_mode) || held.st_uid!=geteuid() || (held.st_mode&0077))){r->error="STATE_UNAVAILABLE";close(owner);owner=-1;}
    if(owner>=0){if(flock(owner,LOCK_EX|LOCK_NB)<0){if(errno==EAGAIN || errno==EWOULDBLOCK)r->active=1;else r->error="STATE_UNAVAILABLE";}}
    else if(!r->phase || !missing)r->error="STATE_UNAVAILABLE";
    if(!r->phase && kind==0){
        rh_status status;if(!rh_status_read(dir,&status))r->error="STATE_UNAVAILABLE";
        else {const char *names[]={"","accepted","starting","running","exited","lost"};strcpy(r->status,status.state<4 && !r->active?"lost":names[status.state]);}
    }else if(!r->phase){
        unsigned char *raw=NULL;size_t size=0;j_value *meta=NULL;
        if(rh_file(dir,kind==1?"manifest.json":"meta.json",&raw,&size,256*1024))meta=j_parse(raw,size);free(raw);
        const char *status=text(meta,"status");j_value *expiry=j_get(meta,"expires_unix");
        if(!meta || strcmp(text(meta,"generation"),generation) || strcmp(text(meta,"id"),r->id) || !expiry || expiry->type!=J_INT || expiry->negative || expiry->number!=r->expiry || strlen(status)>=sizeof r->status || !*status)r->error="STATE_UNAVAILABLE";
        else strcpy(r->status,status);j_free(meta);
    }
    /* An unowned terminal Run can release its bound only after a fresh size
       observation under the same owner lock that proved draining is finished. */
    if(!logical(dir,&r->bytes))r->error="STATE_UNAVAILABLE";
    if(owner>=0)close(owner);close(dir);return 1;
}
int rg_account(rh_state *s,rg_usage *u){
    memset(u,0,sizeof *u);u->complete=1;
    for(int kind=0;kind<3;kind++){
        int base=rg_base(s,kind,0);if(base<0){struct stat st;if(fstatat(s->root,rg_parent(kind),&st,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT)u->complete=0;continue;}
        int copy=dup(base);DIR *entries=copy<0?NULL:fdopendir(copy);if(!entries){if(copy>=0)close(copy);close(base);u->complete=0;continue;}
        struct dirent *p;size_t visits=0;
        while((errno=0,p=readdir(entries))){if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
            if(++visits>1048576){u->complete=0;break;}
            rg_record r;if(!rg_observe(base,p->d_name,kind,s->generation,&r)){u->complete=0;continue;}
            uint64_t reserve=kind==0?RG_RUN_RESERVE:kind==1?RG_CHANGE_RESERVE:RG_QUERY_RESERVE;
            if(kind==0 && !r.error && !r.phase && !r.active && !strcmp(r.status,"exited"))reserve=r.bytes;
            if(reserve<r.bytes)reserve=r.bytes;
            if(UINT64_MAX-u->logical<r.bytes || UINT64_MAX-u->reserved<reserve){u->complete=0;break;}
            u->logical+=r.bytes;u->reserved+=reserve;u->records++;if(kind==2)u->queries++;if(r.error)u->complete=0;
        }
        if(errno)u->complete=0;closedir(entries);close(base);
    }
    return u->complete;
}
const char *rg_capacity(rh_state *s,int kind){
    rg_usage u;if(!rg_account(s,&u))return "STATE_UNAVAILABLE";
    uint64_t extra=kind==0?RG_RUN_RESERVE:kind==1?RG_CHANGE_RESERVE:RG_QUERY_RESERVE;
    if(u.records>=RG_RECORDS || u.reserved>RG_BYTES || extra>RG_BYTES-u.reserved || (kind==2 && u.queries>=RG_QUERIES))return "STATE_CAPACITY_EXCEEDED";
    return NULL;
}
