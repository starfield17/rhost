/* Opt-in Linux cgroup v2 containment under an operator-selected delegation. */
#include "state.h"
#include "cgroup.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#ifdef __linux__
#include <sys/vfs.h>
#endif

void rh_cgroup_close(rh_cgroup *group,int remove_leaf) {
    if(group->kill>=0)close(group->kill);
    if(group->events>=0)close(group->events);
    if(group->procs>=0)close(group->procs);
    if(group->leaf>=0)close(group->leaf);
    if(group->parent>=0) {
        if(remove_leaf && group->name[0])(void)unlinkat(group->parent,group->name,AT_REMOVEDIR);
        close(group->parent);
    }
    *group=(rh_cgroup)RH_CGROUP_INIT;
}
int rh_cgroup_parent(rh_cgroup *group) {
#ifdef __linux__
    group->parent=rh_directory(getenv("RHOST_CGROUP_ROOT"),0);
    struct statfs info;
    return group->parent>=0 && fstatfs(group->parent,&info)==0 && info.f_type==0x63677270
        && faccessat(group->parent,".",W_OK,0)==0;
#else
    (void)group;
    return 0;
#endif
}
int rh_cgroup_prepare(rh_cgroup *group) {
#ifdef __linux__
    char name[40]="rhost-";
    if(!rh_random(name+6,16) || mkdirat(group->parent,name,0700)<0)goto fail;
    strcpy(group->name,name);
    group->leaf=openat(group->parent,name,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(group->leaf<0)goto fail;
    group->procs=openat(group->leaf,"cgroup.procs",O_WRONLY|O_NOFOLLOW|O_CLOEXEC);
    group->events=openat(group->leaf,"cgroup.events",O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
    group->kill=openat(group->leaf,"cgroup.kill",O_WRONLY|O_NOFOLLOW|O_CLOEXEC);
    if(group->procs<0 || group->events<0 || group->kill<0 || rh_cgroup_population(group)!=1)goto fail;
    return 1;
fail:
    rh_cgroup_close(group,1);
#else
    (void)group;
#endif
    return 0;
}
static int write_once(int fd,const char *value,size_t count) {
    ssize_t n;
    do {n=write(fd,value,count);}while(n<0 && errno==EINTR);
    return n==(ssize_t)count;
}
int rh_cgroup_attach(rh_cgroup *group,pid_t child) {
    char pid[32];int n=snprintf(pid,sizeof pid,"%ld",(long)child);
    return n>0 && (size_t)n<sizeof pid && write_once(group->procs,pid,(size_t)n)
        && rh_cgroup_population(group)==2;
}
int rh_cgroup_population(const rh_cgroup *group) {
    char text[256];ssize_t n;
    do {n=pread(group->events,text,sizeof text-1,0);}while(n<0 && errno==EINTR);
    if(n<=0)return 0;
    text[n]='\0';
    for(char *line=text;line;) {
        if(!strncmp(line,"populated 0\n",12))return 1;
        if(!strncmp(line,"populated 1\n",12))return 2;
        line=strchr(line,'\n');if(line)line++;
    }
    return 0;
}
int rh_cgroup_kill(const rh_cgroup *group) {
    return group->kill>=0 && write_once(group->kill,"1",1);
}
