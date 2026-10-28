#include "inspect.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
void i_skip(i_tree *t,const char *path,const char *code,unsigned reason){
    if(reason<6)t->reasons[reason]=1;size_t count=0;for(j_value *p=t->skipped->first;p;p=p->next)count++;
    if(count>=128)return;j_value *v=j_new(J_OBJECT);if(v && j_text(v,"path",path,strlen(path)) && j_text(v,"code",code,strlen(code)) && j_add(t->skipped,NULL,v))return;j_free(v);
}
static int order(const void *a,const void *b){return strcmp(*(const char *const *)a,*(const char *const *)b);}
int i_walk(i_tree *t,int fd,const char *root,unsigned depth){
    int copy=dup(fd);DIR *dir=copy<0?NULL:fdopendir(copy);if(!dir){if(copy>=0)close(copy);i_skip(t,root,"INVALID_TARGET",4);return 1;}
    char **names=calloc(I_COUNT,sizeof *names);if(!names){closedir(dir);return 0;}size_t count=0;struct dirent *p;int ok=1;
    errno=0;while((p=readdir(dir))){
        if(!strcmp(p->d_name,".") || !strcmp(p->d_name,".."))continue;
        if(t->visited==I_COUNT){t->reasons[0]=1;break;}t->visited++;
        if(!rh_utf8((const unsigned char *)p->d_name,strlen(p->d_name),0)){i_skip(t,root,"INVALID_TEXT",5);continue;}
        names[count]=strdup(p->d_name);if(!names[count]){ok=0;break;}count++;
    }
    if(!p && errno)i_skip(t,root,"INVALID_TARGET",4);closedir(dir);qsort(names,count,sizeof *names,order);
    for(size_t i=0;ok && i<count;i++){
        size_t size=strlen(root)+strlen(names[i])+2;char *path=malloc(size);if(!path){ok=0;break;}
        snprintf(path,size,"%s%s%s",root,!strcmp(root,"/")?"":"/",names[i]);struct stat st;
        if(strlen(path)>4096){i_skip(t,root,"INVALID_TARGET",4);free(path);continue;}
        if(fstatat(fd,names[i],&st,AT_SYMLINK_NOFOLLOW)<0){i_skip(t,path,"INVALID_TARGET",4);free(path);continue;}
        if(S_ISDIR(st.st_mode)){
            if(depth==32){i_skip(t,path,"INVALID_TARGET",1);free(path);continue;}
            int child=openat(fd,names[i],O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);struct stat held;
            if(child<0 || fstat(child,&held)<0 || held.st_dev!=st.st_dev || held.st_ino!=st.st_ino)i_skip(t,path,"FILE_CONFLICT",4);
            else ok=i_walk(t,child,path,depth+1);if(child>=0)close(child);free(path);
        }else if(S_ISREG(st.st_mode)){if(t->count==I_COUNT){t->reasons[0]=1;free(path);}else t->paths[t->count++]=path;}
        else free(path); /* Symlinks and nonregular entries are policy exclusions. */
    }
    for(size_t i=0;i<count;i++)free(names[i]);free(names);return ok;
}
