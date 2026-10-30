#include "collection.h"
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static int order(const void *a,const void *b){return strcmp(*(const char *const *)a,*(const char *const *)b);}
int i_collection_metadata(i_collection *c,i_tree *tree,const char *status,f_error *e){
    j_value *v=j_new(J_OBJECT),*reasons=j_new(J_ARRAY),*skipped=tree?j_clone(tree->skipped):j_new(J_ARRAY);j_buf b={0};b.limit=256*1024;
    const char *names[]={"entry_limit","depth_limit","scan_byte_limit","file_limit","unreadable","invalid_text"};int ok=v && reasons && skipped;
    for(unsigned i=0;ok && i<6;i++)if(tree && tree->reasons[i])ok=j_text(reasons,NULL,names[i],strlen(names[i]));
    if(ok)ok=j_integer(v,"version",1) && j_text(v,"generation",c->generation,32) && j_text(v,"id",c->id,strlen(c->id)) && j_integer(v,"expires_unix",c->expiry) && j_text(v,"status",status,strlen(status)) && j_text(v,"query_sha256",c->fingerprint,64) && j_text(v,"root",c->root,strlen(c->root)) && j_integer(v,"manifest_size",c->size) && j_integer(v,"count",c->count) && j_take(v,"reasons",&reasons) && j_take(v,"skipped",&skipped) && j_dump(v,&b) && rh_atomic(c->directory,"meta.json",b.data,b.size);
    j_free(reasons);j_free(skipped);free(b.data);if(!ok){j_free(v);f_fail(e,"STATE_UNAVAILABLE","cannot publish collection evidence");return 0;}j_free(c->meta);c->meta=v;return 1;
}
int i_collection_build(i_collection *c,const j_value *q,int root,f_error *e){
    i_tree tree={0};tree.root=c->root;tree.glob=j_string(j_get(q,"glob"));if(!tree.glob)tree.glob="*";tree.excludes=j_get(q,"exclude_dirs");tree.skipped=j_new(J_ARRAY);tree.paths=calloc(I_COUNT,sizeof *tree.paths);
    j_buf manifest={0},index={0};manifest.limit=I_MANIFEST;index.limit=(I_COUNT+1)*8;int ok=tree.skipped && tree.paths && i_walk(&tree,root,c->root,0);
    if(ok){qsort(tree.paths,tree.count,sizeof *tree.paths,order);
        for(size_t i=0;i<tree.count;i++){
            unsigned char offset[8],length[4];rh_put32(offset,(uint32_t)(manifest.size>>32));rh_put32(offset+4,(uint32_t)manifest.size);j_append(&index,offset,8);
            unsigned char digest[32];i_path_digest(c,i,manifest.size,tree.paths[i],digest);
            size_t n=strlen(tree.paths[i]);rh_put32(length,(uint32_t)n);j_append(&manifest,length,4);j_append(&manifest,tree.paths[i],n);j_append(&manifest,digest,32);
        }
        unsigned char offset[8];rh_put32(offset,(uint32_t)(manifest.size>>32));rh_put32(offset+4,(uint32_t)manifest.size);j_append(&index,offset,8);
        c->size=manifest.size;c->count=tree.count;
        ok=!manifest.failed && !index.failed && rh_atomic(c->directory,"paths",manifest.data,manifest.size) && rh_atomic(c->directory,"paths.index",index.data,index.size) && i_collection_metadata(c,&tree,"ready",e);
    }
    if(!ok){(void)i_collection_metadata(c,NULL,"failed",e);f_fail(e,"STATE_UNAVAILABLE","collection construction failed; retain its identity for explicit pruning");}
    for(size_t i=0;i<tree.count;i++)free(tree.paths[i]);free(tree.paths);j_free(tree.skipped);free(manifest.data);free(index.data);return ok;
}
