#include "inspect.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <fnmatch.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
void i_skip(i_tree *t,const char *path,const char *code,unsigned reason){
    if(reason<6)t->reasons[reason]=1;size_t count=0;for(j_value *p=t->skipped->first;p;p=p->next)count++;
    size_t size=strlen(path);if(count>=128 || size>16384-t->diagnostics)return;
    j_value *v=j_new(J_OBJECT);if(!v)return;
    if(!j_text(v,"path",path,size) || !j_text(v,"code",code,strlen(code))){j_free(v);return;}
    if(j_add(t->skipped,NULL,v))t->diagnostics+=size;
}
static int excluded(const i_tree *t,const char *relative){
    for(const j_value *p=t->excludes?t->excludes->first:NULL;p;p=p->next){const char *name=j_string(p);size_t n=strlen(name);if(!strncmp(relative,name,n) && (!relative[n] || relative[n]=='/'))return 1;}return 0;
}
int i_walk(i_tree *t,int fd,const char *root,unsigned depth){
    int copy=dup(fd);DIR *dir=copy<0?NULL:fdopendir(copy);if(!dir){if(copy>=0)close(copy);i_skip(t,root,"INVALID_TARGET",4);return 1;}
    struct dirent *p;int ok=1;
    while((errno=0,p=readdir(dir))){
        if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
        if(!rh_utf8((const unsigned char *)p->d_name,strlen(p->d_name),0)){i_skip(t,root,"INVALID_TEXT",5);if(++t->visited>=I_COUNT){t->reasons[0]=1;break;}continue;}
        size_t size=strlen(root)+strlen(p->d_name)+2;char *path=malloc(size);if(!path){ok=0;break;}
        snprintf(path,size,"%s%s%s",root,!strcmp(root,"/")?"":"/",p->d_name);
        const char *relative=path+strlen(t->root);if(*relative=='/')relative++;
        struct stat st;int observed=fstatat(fd,p->d_name,&st,AT_SYMLINK_NOFOLLOW)==0;
        /* Exclusions are directory policy, before descent and traversal charging. */
        if(observed && S_ISDIR(st.st_mode) && excluded(t,relative)){free(path);continue;}
        if(t->visited==I_COUNT){t->reasons[0]=1;free(path);break;}t->visited++;
        if(strlen(path)>4096 || !observed){i_skip(t,path,"INVALID_TARGET",4);free(path);continue;}
        if(S_ISDIR(st.st_mode)){
            if(depth==32){i_skip(t,path,"INVALID_TARGET",1);free(path);continue;}
            int child=openat(fd,p->d_name,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);struct stat held;
            if(child<0 || fstat(child,&held)<0 || held.st_dev!=st.st_dev || held.st_ino!=st.st_ino)i_skip(t,path,"FILE_CONFLICT",4);
            else ok=i_walk(t,child,path,depth+1);if(child>=0)close(child);free(path);if(!ok)break;
        }else if(S_ISREG(st.st_mode) && !fnmatch(t->glob,relative,0)){
            size_t length=strlen(path)+36;if(t->count==I_COUNT || length>I_MANIFEST-t->bytes){t->reasons[0]=1;free(path);break;}
            t->paths[t->count++]=path;t->bytes+=length;
        }else free(path);
        if(t->visited==I_COUNT){t->reasons[0]=1;break;}
    }
    if(!p && errno)i_skip(t,root,"INVALID_TARGET",4);closedir(dir);return ok;
}
