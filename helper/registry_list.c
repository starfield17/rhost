#include "registry.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static j_value *entry_json(const rg_record *r,int kind){
    j_value *v=j_new(J_OBJECT),*error=j_new(r->error?J_OBJECT:J_NULL);if(!v || !error)goto fail;
    if(r->error && (!j_text(error,"code",r->error,strlen(r->error)) || !j_text(error,"message","record evidence is unavailable",30)))goto fail;
    if(!j_text(v,"id",r->id,strlen(r->id)) || !j_text(v,"kind",rg_label(kind),strlen(rg_label(kind))) || !j_integer(v,"expires_unix",r->expiry) || !j_text(v,"status",r->status,strlen(r->status)) || !j_boolean(v,"owner_active",r->active) || !j_integer(v,"logical_bytes",r->bytes) || !j_take(v,"error",&error))goto fail;
    return v;
fail:j_free(v);j_free(error);return NULL;
}
j_value *rg_list(rh_state *s,int kind,const j_value *q,f_error *e){
    uint64_t limit=64;if(!f_number(q,"limit",64,&limit) || !limit || limit>256)return f_fail(e,"CONFIG_INVALID","invalid state page size");
    const char *cursor=j_string(j_get(q,"cursor")),*after="";char prefix[64];snprintf(prefix,sizeof prefix,"%s:%s:",s->generation,rg_label(kind));
    if(j_get(q,"cursor")){
        uint64_t expiry;if(!cursor || strncmp(cursor,prefix,strlen(prefix)) || !rh_key(cursor+strlen(prefix),&expiry))return f_fail(e,"CONFIG_INVALID","state cursor does not bind this namespace and kind");
        after=cursor+strlen(prefix);
    }
    int base=rg_base(s,kind,0);if(base<0){struct stat st;if(fstatat(s->root,rg_parent(kind),&st,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT)return f_fail(e,"STATE_UNAVAILABLE","state index is unavailable");}
    char *names[257]={0};char ids[257][80];size_t count=0,visits=0;int ok=1;
    if(base>=0){
        int copy=dup(base);DIR *dir=copy<0?NULL:fdopendir(copy);if(!dir){if(copy>=0)close(copy);close(base);return f_fail(e,"STATE_UNAVAILABLE","cannot enumerate state");}
        struct dirent *p;
        while((errno=0,p=readdir(dir))){
            if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
            if(++visits>1048576){ok=0;break;}
            char id[80];uint64_t expiry;unsigned phase;
            if(!rg_identity(p->d_name,id,&expiry,&phase))continue;
            if(strcmp(id,after)<=0)continue;
            size_t at=0;while(at<count && strcmp(ids[at],id)<0)at++;
            if(at<count && !strcmp(ids[at],id)){ok=0;break;}
            if(at>limit)continue;
            char *name=strdup(p->d_name);if(!name){ok=0;break;}
            if(count==limit+1){free(names[count-1]);count--;}
            for(size_t i=count;i>at;i--){names[i]=names[i-1];strcpy(ids[i],ids[i-1]);}
            names[at]=name;strcpy(ids[at],id);count++;
        }
        if(errno)ok=0;closedir(dir);
    }
    j_value *out=j_new(J_OBJECT),*items=j_new(J_ARRAY);if(!out || !items)ok=0;
    size_t returned=count>limit?(size_t)limit:count;
    for(size_t i=0;ok && i<returned;i++){
        rg_record record;if(!rg_observe(base,names[i],kind,s->generation,&record) || !j_add(items,NULL,entry_json(&record,kind)))ok=0;
    }
    if(ok)ok=j_text(out,"generation",s->generation,32) && j_take(out,"items",&items);
    if(ok && count>limit){char next[160];snprintf(next,sizeof next,"%s%s",prefix,ids[returned-1]);ok=j_text(out,"next_cursor",next,strlen(next));}
    else if(ok)ok=j_add(out,"next_cursor",j_new(J_NULL));
    for(size_t i=0;i<count;i++)free(names[i]);if(base>=0)close(base);j_free(items);
    if(!ok){j_free(out);return f_fail(e,"STATE_UNAVAILABLE","cannot complete bounded state listing");}return out;
}
j_value *rg_json(const j_value *q,f_error *e){
    const char *op=j_string(j_get(q,"op")),*generation=j_string(j_get(q,"generation"));
    int listing=op && !strcmp(op,"state.list"),usage=op && !strcmp(op,"state.usage"),prune=op && !strcmp(op,"state.prune");
    int kind=rg_kind(j_string(j_get(q,"kind")));
    if((!listing && !usage && !prune) || !rh_generation(generation) || ((!usage) && kind<0) || !f_fields(q,listing?"op generation kind limit cursor":usage?"op generation":"op generation kind ids dry_run"))return f_fail(e,"CONFIG_INVALID","invalid closed state operation");
    if(!f_context(e))return NULL;rh_state s;if(!rh_state_open(&s,0))return f_fail(e,"STATE_UNAVAILABLE","prepare the namespace before inspecting state");
    j_value *out=NULL;if(strcmp(generation,s.generation)){f_fail(e,"NAMESPACE_MISMATCH","remote namespace changed");goto done;}
    if(listing)out=rg_list(&s,kind,q,e);
    else if(prune)out=rg_prune(&s,q,e);
    else {
        rg_usage u;(void)rg_account(&s,&u);out=j_new(J_OBJECT);j_value *limits=j_new(J_OBJECT);
        if(!out || !limits || !j_integer(limits,"records",RG_RECORDS) || !j_integer(limits,"reserved_bytes",RG_BYTES) || !j_integer(limits,"collections",RG_QUERIES) || !j_text(out,"generation",s.generation,32) || !j_integer(out,"logical_bytes",u.logical) || !j_integer(out,"reserved_bytes",u.reserved) || !j_integer(out,"records",u.records) || !j_integer(out,"collections",u.queries) || !j_boolean(out,"complete",u.complete) || !j_take(out,"limits",&limits)){j_free(out);j_free(limits);out=NULL;f_fail(e,"INTERNAL","cannot allocate state usage");}
    }
done:rh_state_close(&s);return out;
}
