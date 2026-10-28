#include "inspect.h"
#include <errno.h>
#include <fnmatch.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static int order(const void *a,const void *b){return strcmp(*(const char *const *)a,*(const char *const *)b);}
static void hash_text(rh_hash *hash,const void *p,size_t size){unsigned char length[4];rh_put32(length,(uint32_t)size);rh_hash_update(hash,length,4);rh_hash_update(hash,p,size);}
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
    unsigned char block[65536];uint64_t available=I_SCAN-*total;
    bytes->limit=(size_t)(available<F_EDIT_LIMIT?available:F_EDIT_LIMIT);
    while(bytes->size<bytes->limit){size_t left=bytes->limit-bytes->size;size_t count=left<sizeof block?left:sizeof block;ssize_t n=read(fd,block,count);
        if(n<0 && errno==EINTR)continue;if(n<0){f_os_error(e,0);return 0;}if(!n)break;
        *total+=(uint64_t)n;j_append(bytes,block,(size_t)n);if(bytes->failed){f_fail(e,"INTERNAL","allocation failed");return 0;}
    }
    return 1;
}
j_value *f_search(const j_value *q,f_error *e){
    uint64_t limit,matches,remaining;const char *glob=j_get(q,"glob")?j_string(j_get(q,"glob")):"*",*cursor=j_get(q,"cursor")?j_string(j_get(q,"cursor")):NULL;j_value *pattern=j_get(q,"pattern");size_t offset=0;
    if(!f_fields(q,"op root glob pattern cursor max_results max_matches max_bytes") || !glob || !*glob || strlen(glob)>4096 || (pattern && (pattern->type!=J_TEXT || !pattern->size || pattern->size>4096 || memchr(pattern->text,'\n',pattern->size) || memchr(pattern->text,'\r',pattern->size))) || !f_number(q,"max_results",64,&limit) || !limit || limit>128 || !f_number(q,"max_matches",128,&matches) || !matches || matches>512 || !f_number(q,"max_bytes",262144,&remaining) || !remaining || remaining>1048576 || (j_get(q,"cursor") && !cursor))return f_fail(e,"CONFIG_INVALID","invalid structured search bounds");
    if(cursor){size_t n=strlen(cursor);if(n<66 || n>69 || cursor[64]!=':')return f_fail(e,"CONFIG_INVALID","invalid search cursor");for(size_t i=0;i<64;i++)if(!((cursor[i]>='0' && cursor[i]<='9')||(cursor[i]>='a' && cursor[i]<='f')))return f_fail(e,"CONFIG_INVALID","invalid search revision");for(size_t i=65;i<n;i++){if(cursor[i]<'0'||cursor[i]>'9')return f_fail(e,"CONFIG_INVALID","invalid search offset");offset=offset*10+(unsigned)(cursor[i]-'0');}}
    j_value *root_request=j_new(J_OBJECT);if(!root_request || !j_add(root_request,"path",j_clone(j_get(q,"root")))){j_free(root_request);return f_fail(e,"CONFIG_INVALID","search needs root");}char *root=f_path(root_request,e);j_free(root_request);if(!root)return NULL;
    if(!f_context(e)){free(root);return NULL;}int dir=f_directory(root,0,e);if(dir<0){free(root);return NULL;}
    i_tree tree={0};tree.skipped=j_new(J_ARRAY);j_value *items=j_new(J_ARRAY),*reasons=j_new(J_ARRAY),*result=NULL;
    if(!tree.skipped || !items || !reasons || !i_walk(&tree,dir,root,0)){f_fail(e,"INTERNAL","cannot collect bounded search paths");goto done;}
    qsort(tree.paths,tree.count,sizeof tree.paths[0],order);if(offset>tree.count){f_fail(e,"CONFIG_INVALID","search offset is outside collection");goto done;}
    rh_hash revision;rh_hash_init(&revision);hash_text(&revision,root,strlen(root));hash_text(&revision,glob,strlen(glob));hash_text(&revision,pattern?pattern->text:NULL,pattern?pattern->size:0);
    size_t count=0,next=tree.count;uint64_t scanned=0,total=0;
    for(size_t i=0;i<tree.count;i++){
        const char *path=tree.paths[i],*relative=path+strlen(root);if(*relative=='/')relative++;if(fnmatch(glob,relative,0))continue;
        hash_text(&revision,path,strlen(path));f_error failure={NULL,NULL,"not_applied"};char *name=NULL;int parent=f_parent(path,0,&name,&failure),fd=parent<0?-1:f_open(parent,name,0,&failure);if(parent>=0)close(parent);free(name);struct stat before,after;j_buf bytes={0};int ok=fd>=0 && fstat(fd,&before)==0;scanned++;
        if(ok && (before.st_size<0 || (uint64_t)before.st_size>F_EDIT_LIMIT)){i_skip(&tree,path,"FILE_TOO_LARGE",3);ok=0;}
        else if(ok && (uint64_t)before.st_size>I_SCAN-total){i_skip(&tree,path,"FILE_TOO_LARGE",2);ok=0;}
        else if(ok){ok=load(fd,&bytes,&total,&failure) && fstat(fd,&after)==0 && f_identity(&before,&after) && after.st_size>=0 && (uint64_t)after.st_size==bytes.size;if(!ok)i_skip(&tree,path,"FILE_CONFLICT",4);}
        else i_skip(&tree,path,failure.code?failure.code:"INVALID_TARGET",4);
        if(fd>=0)close(fd);if(!ok){hash_text(&revision,"unavailable",11);free(bytes.data);continue;}
        unsigned char digest[32];char hash[65];rh_sha256(bytes.data,bytes.size,digest);rh_hex(digest,32,hash);hash_text(&revision,hash,64);
        if(!rh_utf8(bytes.data,bytes.size,1)){i_skip(&tree,path,"INVALID_TEXT",5);free(bytes.data);continue;}
        if(i>=offset && contains(bytes.data,bytes.size,pattern)){
            if(count==limit){if(next==tree.count)next=i;}
            else{j_value *v=item(path,hash,&bytes,pattern,&matches,&remaining);if(!v || !j_add(items,NULL,v)){free(bytes.data);f_fail(e,"INTERNAL","cannot construct search page");goto done;}count++;}
        }
        free(bytes.data);
    }
    unsigned char digest[32];char hash[65];rh_hash_finish(&revision,digest);rh_hex(digest,32,hash);
    if(cursor && memcmp(cursor,hash,64)){f_fail(e,"FILE_CONFLICT","search collection changed; restart without cursor");goto done;}
    const char *names[]={"entry_limit","depth_limit","scan_byte_limit","file_limit","unreadable","invalid_text"};int incomplete=0;for(size_t i=0;i<6;i++)if(tree.reasons[i]){incomplete=1;if(!j_text(reasons,NULL,names[i],strlen(names[i])))goto allocation;}
    result=j_new(J_OBJECT);if(!result || !j_text(result,"root",root,strlen(root)) || !j_text(result,"revision",hash,64) || !j_integer(result,"scanned_files",scanned) || !j_integer(result,"scanned_bytes",total) || !j_boolean(result,"incomplete",incomplete))goto allocation;
    if(next<tree.count && !incomplete){char continuation[80];snprintf(continuation,sizeof continuation,"%s:%zu",hash,next);if(!j_text(result,"next_cursor",continuation,strlen(continuation)))goto allocation;}else if(!j_add(result,"next_cursor",j_new(J_NULL)))goto allocation;
    if(!j_take(result,"items",&items))goto allocation;items=NULL;if(!j_take(result,"skipped",&tree.skipped))goto allocation;tree.skipped=NULL;if(!j_take(result,"reasons",&reasons))goto allocation;reasons=NULL;goto done;
allocation:j_free(result);result=NULL;f_fail(e,"INTERNAL","cannot allocate search result");
done:close(dir);free(root);for(size_t i=0;i<tree.count;i++)free(tree.paths[i]);j_free(items);j_free(tree.skipped);j_free(reasons);return result;
}
