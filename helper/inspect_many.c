#include "inspect.h"
#include <stdlib.h>
#include <string.h>
j_value *f_read_many(const j_value *q,f_error *e){
    j_value *paths=j_get(q,"paths");uint64_t cap,start,lines;size_t count=0;
    if(!f_fields(q,"op paths start lines max_bytes") || !paths || paths->type!=J_ARRAY || !f_number(q,"max_bytes",262144,&cap) || !cap || cap>F_EDIT_LIMIT || !f_number(q,"start",1,&start) || !start || !f_number(q,"lines",200,&lines) || !lines)return f_fail(e,"CONFIG_INVALID","invalid read-many bounds");
    char *names[64]={0};j_value *result=NULL,*items=NULL,*unread=NULL;
    for(j_value *p=paths->first;p;p=p->next){if(count==64){f_fail(e,"CONFIG_INVALID","read-many accepts at most 64 paths");goto done;}j_value *one=j_new(J_OBJECT);if(!one || !j_add(one,"path",j_clone(p))){j_free(one);goto done;}names[count]=f_path(one,e);j_free(one);if(!names[count])goto done;count++;}
    if(!count){f_fail(e,"CONFIG_INVALID","read-many needs named paths");goto done;}if(!f_context(e))goto done;
    result=j_new(J_OBJECT);items=j_new(J_ARRAY);unread=j_new(J_ARRAY);if(!result || !items || !unread)goto allocation;
    for(size_t i=0;i<count;i++){
        if(!cap){if(!j_text(unread,NULL,names[i],strlen(names[i])))goto allocation;continue;}
        j_value *request=j_new(J_OBJECT);if(!request || !j_integer(request,"start",start) || !j_integer(request,"lines",lines) || !j_integer(request,"max_bytes",cap)){j_free(request);goto allocation;}
        f_error error={NULL,NULL,"not_applied"};j_value *page=f_read(request,names[i],&error);j_free(request);
        j_value *item=j_new(J_OBJECT),*failure=j_new(page?J_NULL:J_OBJECT);
        if(!item || !failure){j_free(item);j_free(failure);j_free(page);goto allocation;}
        if(page){j_value *text=j_get(page,"content");if(!text || text->type!=J_TEXT || text->size>cap){j_free(item);j_free(failure);j_free(page);goto allocation;}cap-=text->size;}
        else if(!j_text(failure,"code",error.code?error.code:"INTERNAL",strlen(error.code?error.code:"INTERNAL")) || !j_text(failure,"message",error.message?error.message:"read failed",strlen(error.message?error.message:"read failed"))){j_free(item);j_free(failure);goto allocation;}
        if(!j_text(item,"path",names[i],strlen(names[i])) || !j_boolean(item,"ok",page!=NULL)){j_free(item);j_free(page);j_free(failure);goto allocation;}
        if(!page)page=j_new(J_NULL);
        if(!j_take(item,"data",&page) || !j_take(item,"error",&failure)){j_free(item);j_free(page);j_free(failure);goto allocation;}
        if(!j_take(items,NULL,&item))goto allocation;
    }
    if(!j_take(result,"items",&items))goto allocation;items=NULL;if(!j_take(result,"unread",&unread))goto allocation;unread=NULL;goto done;
allocation:j_free(result);result=NULL;f_fail(e,"INTERNAL","allocation failed");
done:j_free(items);j_free(unread);for(size_t i=0;i<count;i++)free(names[i]);return result;
}
