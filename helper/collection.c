#include "collection.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <unistd.h>
static const char *text(const j_value *v,const char *key){const char *s=j_string(j_get(v,key));return s?s:"";}
static int number(const j_value *v,const char *key,uint64_t *n){j_value *p=j_get(v,key);if(!p || p->type!=J_INT || p->negative)return 0;*n=p->number;return 1;}
static int order(const void *a,const void *b){return strcmp(*(const char *const *)a,*(const char *const *)b);}
static void hash_text(rh_hash *h,const char *s){unsigned char n[4];size_t size=strlen(s);rh_put32(n,(uint32_t)size);rh_hash_update(h,n,4);rh_hash_update(h,s,size);}
void i_path_digest(const i_collection *c,uint64_t position,uint64_t offset,const char *path,unsigned char digest[32]){
    rh_hash h;rh_hash_init(&h);hash_text(&h,c->generation);hash_text(&h,c->id);hash_text(&h,c->fingerprint);
    unsigned char n[16];rh_put32(n,(uint32_t)(position>>32));rh_put32(n+4,(uint32_t)position);rh_put32(n+8,(uint32_t)(offset>>32));rh_put32(n+12,(uint32_t)offset);rh_hash_update(&h,n,16);hash_text(&h,path);rh_hash_finish(&h,digest);
}
int i_fingerprint(const j_value *q,const char *root,char hash[65],f_error *e){
    const j_value *excludes=j_get(q,"exclude_dirs");const char *names[64];size_t count=0;
    if(excludes && excludes->type!=J_ARRAY)return f_fail(e,"CONFIG_INVALID","exclusions must be an array"),0;
    for(const j_value *p=excludes?excludes->first:NULL;p;p=p->next){
        const char *s=j_string(p);if(count==64 || !s || !*s || *s=='/' || strlen(s)>4096)return f_fail(e,"CONFIG_INVALID","invalid root-relative exclusion"),0;
        for(const char *a=s;*a;){const char *end=strchr(a,'/');size_t n=end?(size_t)(end-a):strlen(a);if(!n || (n==1 && *a=='.') || (n==2 && !memcmp(a,"..",2)) || (end && !end[1]))return f_fail(e,"CONFIG_INVALID","exclusions require canonical relative directories"),0;a=end?end+1:a+n;}
        names[count++]=s;
    }
    qsort(names,count,sizeof *names,order);for(size_t i=1;i<count;i++)if(!strcmp(names[i-1],names[i]))return f_fail(e,"CONFIG_INVALID","duplicate directory exclusion"),0;
    rh_hash h;rh_hash_init(&h);hash_text(&h,text(q,"op"));hash_text(&h,root);hash_text(&h,j_get(q,"glob")?text(q,"glob"):"*");hash_text(&h,text(q,"pattern"));
    for(size_t i=0;i<count;i++)hash_text(&h,names[i]);unsigned char digest[32];rh_hash_finish(&h,digest);rh_hex(digest,32,hash);return 1;
}
static int cursor(i_collection *c,const char *s){
    if(!s || strlen(s)>128)return 0;const char *first=strchr(s,':'),*last=strrchr(s,':');
    if(!first || first==last || first-s!=32 || last-first<2 || (size_t)(last-first-1)>=sizeof c->id)return 0;
    memcpy(c->generation,s,32);c->generation[32]=0;memcpy(c->id,first+1,(size_t)(last-first-1));c->id[last-first-1]=0;
    if(!rh_generation(c->generation) || !rh_key(c->id,&c->expiry) || !last[1] || strlen(last+1)>8 || (last[1]=='0' && last[2]))return 0;
    for(const char *p=last+1;*p;p++){if(*p<'0'||*p>'9')return 0;c->offset=c->offset*10+(unsigned)(*p-'0');}return c->offset<=I_MANIFEST;
}
static int private_file(int dir,const char *name,uint64_t size){
    int fd=openat(dir,name,O_RDONLY|O_NONBLOCK|O_NOFOLLOW|O_CLOEXEC);struct stat st;
    if(fd<0 || fstat(fd,&st)<0 || !S_ISREG(st.st_mode) || st.st_uid!=geteuid() || (st.st_mode&0077) || st.st_size<0 || (uint64_t)st.st_size!=size){if(fd>=0)close(fd);return -1;}return fd;
}
static int at(int fd,void *bytes,size_t n,uint64_t offset){
    size_t used=0;while(used<n){ssize_t r=pread(fd,(unsigned char *)bytes+used,n-used,(off_t)(offset+used));if(r<0 && errno==EINTR)continue;if(r<=0)return 0;used+=(size_t)r;}return 1;
}
static int index_at(i_collection *c,uint64_t i,uint64_t *offset){unsigned char bytes[8];if(i>c->count || !at(c->index,bytes,8,i*8))return 0;*offset=rh_u64(bytes);return *offset<=c->size;}
static int manifest(i_collection *c,f_error *e){
    unsigned char *raw=NULL;size_t size=0;uint64_t version,expiry;
    if(!rh_file(c->directory,"meta.json",&raw,&size,256*1024))goto bad;c->meta=j_parse(raw,size);free(raw);raw=NULL;
    if(!number(c->meta,"version",&version) || version!=1 || strcmp(text(c->meta,"generation"),c->generation) || strcmp(text(c->meta,"id"),c->id) || !number(c->meta,"expires_unix",&expiry) || expiry!=c->expiry || strcmp(text(c->meta,"status"),"ready") || !number(c->meta,"manifest_size",&c->size) || c->size>I_MANIFEST || !number(c->meta,"count",&c->count) || c->count>I_COUNT)goto bad;
    if(strcmp(text(c->meta,"root"),c->root) || strcmp(text(c->meta,"query_sha256"),c->fingerprint))return f_fail(e,"FILE_CONFLICT","cursor belongs to a different query"),0;
    j_value *reasons=j_get(c->meta,"reasons"),*skipped=j_get(c->meta,"skipped");if(!reasons || reasons->type!=J_ARRAY || !skipped || skipped->type!=J_ARRAY)goto bad;
    c->paths=private_file(c->directory,"paths",c->size);c->index=private_file(c->directory,"paths.index",(c->count+1)*8);if(c->paths<0 || c->index<0)goto bad;
    uint64_t first,last;if(!index_at(c,0,&first) || first || !index_at(c,c->count,&last) || last!=c->size)goto bad;
    uint64_t lo=0,hi=c->count;
    while(lo<hi){uint64_t mid=lo+(hi-lo)/2,value;if(!index_at(c,mid,&value))goto bad;if(value<c->offset)lo=mid+1;else hi=mid;}
    if(!index_at(c,lo,&first))goto bad;
    if(first!=c->offset)return f_fail(e,"CONFIG_INVALID","cursor is not a manifest record boundary"),0;
    c->position=lo;return 1;
bad:free(raw);return f_fail(e,"STATE_UNAVAILABLE","collection manifest is incomplete or corrupt"),0;
}
int i_collection_open(i_collection *c,const j_value *q,const char *root,f_error *e){
    memset(c,0,sizeof *c);c->directory=c->owner=c->paths=c->index=-1;c->root=strdup(root);
    if(!c->root || !i_fingerprint(q,root,c->fingerprint,e))return 0;
    const char *continuation=j_string(j_get(q,"cursor"));if(continuation && !cursor(c,continuation))return f_fail(e,"CONFIG_INVALID","invalid collection cursor"),0;
    rh_state space;if(!rh_state_open(&space,!continuation))return f_fail(e,"STATE_UNAVAILABLE","collection namespace is unavailable"),0;
    int base=-1,ok=0;uint64_t now;
    if(continuation && strcmp(c->generation,space.generation)){f_fail(e,"NAMESPACE_MISMATCH","collection namespace changed");goto done;}
    memcpy(c->generation,space.generation,33);
    if(!(continuation?rh_clock_read(&space,&now):rh_clock(&space,&now))){f_fail(e,"STATE_UNAVAILABLE","collection clock is unavailable");goto done;}
    if(continuation && c->expiry<=now){f_fail(e,"QUERY_EXPIRED","collection expired; start a new query explicitly");goto done;}
    base=rg_base(&space,2,!continuation);if(base<0){f_fail(e,"QUERY_NOT_FOUND","collection parent is unavailable");goto done;}
    if(!continuation){
        uint64_t ttl;if(!f_number(q,"collection_ttl_seconds",600,&ttl) || !ttl || ttl>3600 || now>UINT64_C(999999999999)-ttl){f_fail(e,"CONFIG_INVALID","invalid collection lifetime");goto done;}
        const char *capacity=rg_capacity(&space,2);if(capacity){f_fail(e,capacity,"collection admission refused before registration");goto done;}
        char random[65],pending[96];c->expiry=now+ttl;
        if(!rh_random(random,32)){f_fail(e,"STATE_UNAVAILABLE","cannot allocate collection identity");goto done;}
        snprintf(c->id,sizeof c->id,"%llu-%s",(unsigned long long)c->expiry,random);snprintf(pending,sizeof pending,".pending-%s",c->id);
        if(mkdirat(base,pending,0700)<0 || fsync(base)<0){f_fail(e,"STATE_UNAVAILABLE","cannot reserve collection identity");goto done;}
        c->directory=openat(base,pending,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        if(!rg_private(c->directory))goto storage;
        c->owner=openat(c->directory,"owner.lock",O_RDWR|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);
        if(c->owner<0 || flock(c->owner,LOCK_EX)<0 || fsync(c->owner)<0 || !i_collection_metadata(c,NULL,"building",e) || fsync(c->directory)<0 || renameat(base,pending,base,c->id)<0 || fsync(base)<0)goto storage;
        close(base);base=-1;rh_state_close(&space);
        int fd=f_directory(root,0,e);if(fd<0)return 0;ok=i_collection_build(c,q,fd,e);close(fd);if(!ok)return 0;
        j_free(c->meta);c->meta=NULL;return manifest(c,e);
    }
    c->directory=openat(base,c->id,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(c->directory<0){f_fail(e,errno==ENOENT?"QUERY_NOT_FOUND":"STATE_UNAVAILABLE","collection is unavailable");goto done;}
    if(!rg_private(c->directory))goto storage;
    c->owner=openat(c->directory,"owner.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC);struct stat st;
    if(c->owner<0 || fstat(c->owner,&st)<0 || !S_ISREG(st.st_mode) || st.st_uid!=geteuid() || (st.st_mode&0077) || flock(c->owner,LOCK_EX|LOCK_NB)<0)goto storage;
    close(base);rh_state_close(&space);return manifest(c,e);
storage:f_fail(e,"STATE_UNAVAILABLE","collection ownership or durable registration is unavailable");
done:if(base>=0)close(base);rh_state_close(&space);return ok;
}
char *i_collection_next(i_collection *c,f_error *e){
    if(c->offset==c->size)return NULL;
    unsigned char length[4];uint64_t start,next;
    if(!index_at(c,c->position,&start) || start!=c->offset || !index_at(c,c->position+1,&next) || next<=start || next-start<37 || !at(c->paths,length,4,start))goto bad;
    uint32_t n=rh_u32(length);if(n>4096 || n+36!=next-start)goto bad;
    char *path=malloc((size_t)n+1);if(!path)return f_fail(e,"INTERNAL","allocation failed");
    if(!at(c->paths,path,n,start+4)){free(path);goto bad;}path[n]=0;
    size_t root=strlen(c->root);if(!rh_utf8((unsigned char *)path,n,0) || memchr(path,0,n) || strncmp(path,c->root,root) || (root>1 && path[root]!='/')){free(path);goto bad;}
    for(const char *p=path+1;*p;){const char *end=strchr(p,'/');size_t size=end?(size_t)(end-p):strlen(p);if(!size || (size==1 && *p=='.') || (size==2 && !memcmp(p,"..",2)) || (end && !end[1])){free(path);goto bad;}p=end?end+1:p+size;}
    unsigned char stored[32],expected[32];i_path_digest(c,c->position,start,path,expected);
    if(!at(c->paths,stored,32,start+4+n) || memcmp(stored,expected,32)){free(path);goto bad;}
    c->offset=next;c->position++;return path;
bad:return f_fail(e,"STATE_UNAVAILABLE","collection record is corrupt");
}
void i_collection_close(i_collection *c){if(c->paths>=0)close(c->paths);if(c->index>=0)close(c->index);if(c->owner>=0)close(c->owner);if(c->directory>=0)close(c->directory);free(c->root);j_free(c->meta);}
