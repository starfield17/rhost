#include "files.h"
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int f_hash_fd(int fd,char hash[65],uint64_t *lines,f_error *e){
    if(lseek(fd,0,SEEK_SET)<0){f_os_error(e,0);return 0;}
    rh_hash h;rh_hash_init(&h);unsigned char buffer[65536],last=0;uint64_t count=0,total=0;
    for(;;){ssize_t n=read(fd,buffer,sizeof buffer);if(n<0 && errno==EINTR)continue;if(n<0){f_os_error(e,0);return 0;}if(!n)break;
        rh_hash_update(&h,buffer,(size_t)n);total+=(uint64_t)n;last=buffer[n-1];
        if(lines)for(ssize_t i=0;i<n;i++)if(buffer[i]=='\n')count++;
    }
    if(lines)*lines=count+(total && last!='\n');unsigned char digest[32];rh_hash_finish(&h,digest);rh_hex(digest,32,hash);return 1;
}
int f_load(int fd,j_buf *out,f_error *e){
    unsigned char buffer[65536];out->limit=F_EDIT_LIMIT;
    for(;;){ssize_t n=read(fd,buffer,sizeof buffer);if(n<0 && errno==EINTR)continue;if(n<0){f_os_error(e,0);return 0;}if(!n)break;
        j_append(out,buffer,(size_t)n);if(out->failed){f_fail(e,"FILE_TOO_LARGE","editing limit is 8388608 bytes");return 0;}}
    return 1;
}
typedef struct {j_buf page,line;uint64_t index,start,count,rows;int blocked,oversized;} page;
static int row(page *p,f_error *e){
    p->index++;
    if(p->index>=p->start && p->rows<p->count && !p->blocked){
        if(p->oversized || p->line.size>p->page.limit-p->page.size)p->blocked=1;
        else{
            if(!rh_utf8(p->line.data,p->line.size,1)){f_fail(e,"INVALID_TEXT","content must be UTF-8");return 0;}
            j_append(&p->page,p->line.data,p->line.size);if(p->page.failed){f_fail(e,"INTERNAL","allocation failed");return 0;}p->rows++;
        }
    }
    p->line.size=0;p->line.failed=0;p->oversized=0;return 1;
}
j_value *f_read(const j_value *q,const char *path,f_error *e){
    uint64_t start,count,cap;
    if(!f_number(q,"start",1,&start) || !f_number(q,"lines",200,&count) || !start || !count || !f_number(q,"max_bytes",0,&cap) || !cap || cap>F_EDIT_LIMIT)return f_fail(e,"CONFIG_INVALID","invalid read bounds");
    if(!f_context(e))return NULL;
    char *name=NULL;int dir=f_parent(path,0,&name,e);if(dir<0)return NULL;
    int fd=f_open(dir,name,0,e);close(dir);free(name);if(fd<0)return NULL;
    struct stat before,after;j_value *result=NULL;page p={0};p.start=start;p.count=count;p.page.limit=p.line.limit=(size_t)cap;
    rh_hash h;rh_hash_init(&h);unsigned char bytes[65536];int pending=0;
    if(fstat(fd,&before)<0){f_os_error(e,0);goto done;}
    for(;;){
        ssize_t n=read(fd,bytes,sizeof bytes);if(n<0 && errno==EINTR)continue;if(n<0){f_os_error(e,0);goto done;}if(!n)break;
        rh_hash_update(&h,bytes,(size_t)n);
        for(ssize_t i=0;i<n;i++){
            pending=1;
            if(!p.oversized){if(p.line.size==p.line.limit)p.oversized=1;else {j_append(&p.line,bytes+i,1);if(p.line.failed){f_fail(e,"INTERNAL","allocation failed");goto done;}}}
            if(bytes[i]=='\n'){if(!row(&p,e))goto done;pending=0;}
        }
    }
    if(pending && !row(&p,e))goto done;
    if(fstat(fd,&after)<0){f_os_error(e,0);goto done;}
    if(!f_identity(&before,&after)){f_fail(e,"FILE_CONFLICT","file changed while reading");goto done;}
    unsigned char hash[32];char hex[65];rh_hash_finish(&h,hash);rh_hex(hash,32,hex);result=j_new(J_OBJECT);
    if(!result || !j_text(result,"path",path,strlen(path)) || !j_text(result,"sha256",hex,64) || !j_text(result,"content",p.page.data,p.page.size) || !j_integer(result,"start",start) || !j_integer(result,"lines",p.rows) || !j_integer(result,"total_lines",p.index) || !j_boolean(result,"truncated",p.blocked)){
        j_free(result);result=NULL;f_fail(e,"INTERNAL","allocation failed");
    }
done:free(p.page.data);free(p.line.data);close(fd);return result;
}
