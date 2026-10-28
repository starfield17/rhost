#include "files.h"
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static j_value *resolve(const j_value *q,const char *path,f_error *e){
    char *actual=realpath(path,NULL);if(!actual){
        if(errno!=ENOENT)return f_os_error(e,0);
        /* Resolve existing ancestors too: realpath need not accept a missing leaf. */
        char *copy=strdup(path),*parent=NULL;size_t offset=0;if(!copy)return f_fail(e,"INTERNAL","allocation failed");
        while(!parent){
            char *slash=strrchr(copy,'/');if(!slash){free(copy);return f_fail(e,"INVALID_TARGET","cannot resolve the target");}
            offset=(size_t)(slash-copy);if(slash==copy)slash[1]=0;else *slash=0;
            parent=realpath(copy,NULL);if(!parent && errno!=ENOENT){free(copy);return f_os_error(e,0);}
        }
        const char *suffix=path+offset;size_t n=strlen(parent)+strlen(suffix)+1;actual=malloc(n);
        if(actual)snprintf(actual,n,"%s%s",!strcmp(parent,"/")?"":parent,suffix);free(parent);free(copy);
        if(!actual)return f_fail(e,"INTERNAL","allocation failed");
    }
    j_value *deletion=j_get(q,"delete");if(deletion && deletion->type!=J_BOOL){free(actual);return f_fail(e,"CONFIG_INVALID","delete must be a boolean");}
    if(deletion && deletion->number){
        char *home=getenv("HOME"),*resolved_home=home?realpath(home,NULL):NULL;size_t slashes=0;for(const char *s=actual;*s;s++)if(*s=='/')slashes++;
        int broad=!strcmp(actual,"/") || slashes<2 || (resolved_home && !strcmp(actual,resolved_home));free(resolved_home);
        if(broad){free(actual);return f_fail(e,"SYNC_REJECTED","refusing to prune root, home or a top-level directory");}
    }
    j_value *result=j_new(J_OBJECT);if(!result || !j_text(result,"path",actual,strlen(actual))){j_free(result);result=NULL;f_fail(e,"INTERNAL","allocation failed");}free(actual);return result;
}
static j_value *operation(const j_value *q,f_error *e){
    const char *op=j_string(j_get(q,"op"));if(!op)return f_fail(e,"CONFIG_INVALID","request needs an operation");
    int reading=!strcmp(op,"read"),writing=!strcmp(op,"write"),patch=!strcmp(op,"patch"),resolving=!strcmp(op,"resolve");
    if(!reading && !writing && !patch && !resolving)return f_fail(e,"CONFIG_INVALID","unknown operation");
    for(const j_value *v=q->first;v;v=v->next){
        const char *allowed=resolving?"op path delete":reading?"op path start lines max_bytes":patch?"op path edits if_hash max_bytes":"op path content if_hash parents file_mode max_bytes";
        int known=0;for(const char *a=allowed;*a;){const char *end=strchr(a,' ');size_t n=end?(size_t)(end-a):strlen(a);if(n==v->key_size && !memcmp(a,v->key,n)){known=1;break;}a=end?end+1:a+n;}
        if(!known)return f_fail(e,"CONFIG_INVALID","unknown request field");
    }
    uint64_t cap;if(!resolving && (!f_number(q,"max_bytes",0,&cap)||!cap||cap>F_EDIT_LIMIT))return f_fail(e,"CONFIG_INVALID","max_bytes must be between 1 and 8388608");
    char *path=f_path(q,e);if(!path)return NULL;j_value *result;
    if(resolving)result=resolve(q,path,e);else if(reading)result=f_read(q,path,e);else result=f_edit(q,path,patch,e);free(path);return result;
}
int rh_files_json(void){
    j_buf input={0};input.limit=J_INPUT_LIMIT;unsigned char bytes[65536];f_error error={NULL,NULL,"not_applied"};j_value *result=NULL,*q=NULL;
    for(;;){size_t n=fread(bytes,1,sizeof bytes,stdin);j_append(&input,bytes,n);if(input.failed || n<sizeof bytes)break;}
    if(input.failed || ferror(stdin))f_fail(&error,"CONFIG_INVALID","request exceeds bounded input or cannot be read");
    else{q=j_parse(input.data,input.size);if(!q || q->type!=J_OBJECT)f_fail(&error,"CONFIG_INVALID","request must be one valid JSON object");else result=operation(q,&error);}
    if(!result){
        result=j_new(J_OBJECT);const char *code=error.code?error.code:"INTERNAL",*message=error.message?error.message:"native file operation failed";
        if(!result || !j_text(result,"error",code,strlen(code)) || !j_text(result,"message",message,strlen(message))){j_free(result);j_free(q);free(input.data);return 255;}
    }
    j_buf out={0};out.limit=J_OUTPUT_LIMIT;int ok=j_dump(result,&out);
    if(ok)ok=fwrite(out.data,1,out.size,stdout)==out.size && fputc('\n',stdout)!=EOF && fflush(stdout)==0;
    free(input.data);free(out.data);j_free(result);j_free(q);return ok?0:255;
}
