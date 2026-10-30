#include "collection.h"
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static int contains(const unsigned char *text,size_t size,const j_value *pattern){if(!pattern)return 1;if(pattern->size>size)return 0;for(size_t i=0;i<=size-pattern->size;i++)if(!memcmp(text+i,pattern->text,pattern->size))return 1;return 0;}
static j_value *item(const char *path,const char *hash,const j_buf *bytes,const j_value *pattern,uint64_t *matches,uint64_t *remaining){
    j_value *v=j_new(J_OBJECT),*lines=j_new(J_ARRAY);if(!v || !lines)goto fail;uint64_t line=1;size_t at=0;int truncated=0;
    if(pattern)while(at<bytes->size){size_t end=at;while(end<bytes->size && bytes->data[end]!='\n')end++;
        if(contains(bytes->data+at,end-at,pattern)){
            if(!*matches || end-at>*remaining)truncated=1;
            else{j_value *row=j_new(J_OBJECT);if(!row || !j_integer(row,"line",line) || !j_text(row,"text",bytes->data+at,end-at) || !j_take(lines,NULL,&row)){j_free(row);goto fail;}(*matches)--;*remaining-=end-at;}
        }
        line++;at=end+(end<bytes->size);
    }
    if(!j_text(v,"path",path,strlen(path)) || !j_text(v,"sha256",hash,64) || !j_integer(v,"bytes",bytes->size) || !j_boolean(v,"matches_truncated",truncated) || !j_take(v,"matches",&lines))goto fail;return v;
fail:j_free(v);j_free(lines);return NULL;
}
static int load(int fd,j_buf *bytes,uint64_t *total,f_error *e){
    unsigned char block[65536];uint64_t available=I_SCAN-*total;bytes->limit=(size_t)(available<F_EDIT_LIMIT?available:F_EDIT_LIMIT);
    while(bytes->size<bytes->limit){size_t left=bytes->limit-bytes->size;size_t count=left<sizeof block?left:sizeof block;ssize_t n=read(fd,block,count);
        if(n<0 && errno==EINTR)continue;if(n<0){f_os_error(e,0);return 0;}if(!n)break;
        *total+=(uint64_t)n;j_append(bytes,block,(size_t)n);if(bytes->failed){f_fail(e,"INTERNAL","allocation failed");return 0;}
    }return 1;
}
j_value *f_search(const j_value *q,f_error *e){
    int find=!strcmp(j_string(j_get(q,"op")),"find");uint64_t limit,matches,remaining,ttl;
    const char *glob=j_get(q,"glob")?j_string(j_get(q,"glob")):"*";j_value *pattern=j_get(q,"pattern");
    if(!f_fields(q,find?"op root glob cursor max_results max_bytes exclude_dirs collection_ttl_seconds":"op root glob pattern cursor max_results max_matches max_bytes exclude_dirs collection_ttl_seconds") || !glob || !*glob || strlen(glob)>4096 || (pattern && (pattern->type!=J_TEXT || !pattern->size || pattern->size>4096 || memchr(pattern->text,'\n',pattern->size) || memchr(pattern->text,'\r',pattern->size))) || !f_number(q,"max_results",64,&limit) || !limit || limit>128 || !f_number(q,"max_matches",128,&matches) || !matches || matches>512 || !f_number(q,"max_bytes",262144,&remaining) || !remaining || remaining>1048576 || !f_number(q,"collection_ttl_seconds",600,&ttl) || !ttl || ttl>3600 || (j_get(q,"cursor") && !j_string(j_get(q,"cursor"))))return f_fail(e,"CONFIG_INVALID","invalid structured inspection bounds");
    j_value *root_request=j_new(J_OBJECT);if(!root_request || !j_add(root_request,"path",j_clone(j_get(q,"root")))){j_free(root_request);return f_fail(e,"CONFIG_INVALID","inspection needs a root");}
    char *root=f_path(root_request,e);j_free(root_request);if(!root)return NULL;
    i_collection c;memset(&c,0,sizeof c);c.directory=c.owner=c.paths=c.index=-1;i_tree tree={0};j_value *items=NULL,*reasons=NULL,*result=NULL,*identity=NULL;
    if(!f_context(e) || !i_collection_open(&c,q,root,e))goto done;
    tree.skipped=j_clone(j_get(c.meta,"skipped"));items=j_new(J_ARRAY);reasons=j_new(J_ARRAY);identity=j_new(J_OBJECT);
    const char *names[]={"entry_limit","depth_limit","scan_byte_limit","file_limit","unreadable","invalid_text"};
    for(const j_value *r=j_get(c.meta,"reasons")->first;r;r=r->next){const char *name=j_string(r);if(!name)goto corrupt;int known=0;for(unsigned i=0;i<6;i++)if(!strcmp(name,names[i])){tree.reasons[i]=1;known=1;}if(!known)goto corrupt;}
    if(!tree.skipped || !items || !reasons || !identity)goto allocation;
    for(const j_value *s=tree.skipped->first;s;s=s->next){const char *p=j_string(j_get(s,"path"));if(!p)goto corrupt;tree.diagnostics+=strlen(p);if(tree.diagnostics>16384)goto corrupt;}
    size_t count=0;uint64_t scanned=0,total=0;
    while(c.offset<c.size && count<limit){
        uint64_t offset=c.offset,position=c.position;char *path=i_collection_next(&c,e);if(!path)goto done;
        if(find){j_value *v=j_new(J_OBJECT);int ok=v && j_text(v,"path",path,strlen(path));free(path);if(!ok){j_free(v);goto allocation;}if(!j_add(items,NULL,v))goto allocation;count++;scanned++;continue;}
        f_error failure={NULL,NULL,"not_applied"};char *name=NULL;int parent=f_parent(path,0,&name,&failure),fd=parent<0?-1:f_open(parent,name,0,&failure);if(parent>=0)close(parent);free(name);
        struct stat before,after;j_buf bytes={0};int ok=fd>=0 && fstat(fd,&before)==0;
        if(ok && before.st_size>=0 && (uint64_t)before.st_size<=F_EDIT_LIMIT && (uint64_t)before.st_size>I_SCAN-total){close(fd);free(path);c.offset=offset;c.position=position;tree.reasons[2]=1;break;}
        scanned++;
        if(ok && (before.st_size<0 || (uint64_t)before.st_size>F_EDIT_LIMIT)){i_skip(&tree,path,"FILE_TOO_LARGE",3);ok=0;}
        else if(ok){ok=load(fd,&bytes,&total,&failure) && fstat(fd,&after)==0 && f_identity(&before,&after) && after.st_size>=0 && (uint64_t)after.st_size==bytes.size;if(!ok)i_skip(&tree,path,"FILE_CONFLICT",4);}
        else i_skip(&tree,path,failure.code?failure.code:"INVALID_TARGET",4);
        if(fd>=0)close(fd);if(!ok){free(bytes.data);free(path);continue;}
        if(!rh_utf8(bytes.data,bytes.size,1)){i_skip(&tree,path,"INVALID_TEXT",5);free(bytes.data);free(path);continue;}
        if(contains(bytes.data,bytes.size,pattern)){
            unsigned char digest[32];char hash[65];rh_sha256(bytes.data,bytes.size,digest);rh_hex(digest,32,hash);
            j_value *v=item(path,hash,&bytes,pattern,&matches,&remaining);if(!v){free(path);free(bytes.data);goto allocation;}if(!j_add(items,NULL,v)){free(path);free(bytes.data);goto allocation;}count++;
        }
        free(bytes.data);free(path);
    }
    int incomplete=0;for(unsigned i=0;i<6;i++)if(tree.reasons[i]){incomplete=1;if(!j_text(reasons,NULL,names[i],strlen(names[i])))goto allocation;}
    result=j_new(J_OBJECT);
    if(!j_text(identity,"generation",c.generation,32) || !j_text(identity,"id",c.id,strlen(c.id)) || !j_integer(identity,"expires_unix",c.expiry) || !j_text(identity,"query_sha256",c.fingerprint,64) || !result || !j_text(result,"root",root,strlen(root)) || !j_take(result,"collection",&identity) || !j_integer(result,"scanned_files",scanned) || !j_integer(result,"scanned_bytes",total) || !j_boolean(result,"incomplete",incomplete))goto allocation;
    if(c.offset<c.size){char next[160];snprintf(next,sizeof next,"%s:%s:%llu",c.generation,c.id,(unsigned long long)c.offset);if(!j_text(result,"next_cursor",next,strlen(next)))goto allocation;}else if(!j_add(result,"next_cursor",j_new(J_NULL)))goto allocation;
    if(!j_take(result,"items",&items) || !j_take(result,"skipped",&tree.skipped) || !j_take(result,"reasons",&reasons))goto allocation;goto done;
corrupt:f_fail(e,"STATE_UNAVAILABLE","collection diagnostics are corrupt");goto done;
allocation:j_free(result);result=NULL;f_fail(e,"INTERNAL","cannot allocate inspection result");
done:i_collection_close(&c);free(root);j_free(items);j_free(tree.skipped);j_free(reasons);j_free(identity);return result;
}
