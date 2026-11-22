#ifndef RHOST_CGROUP_H
#define RHOST_CGROUP_H
#include <sys/types.h>
typedef struct {
    int parent, leaf, procs, events, kill;
    char name[40];
} rh_cgroup;
#define RH_CGROUP_INIT {-1,-1,-1,-1,-1,{0}}
int rh_cgroup_parent(rh_cgroup *group);
int rh_cgroup_prepare(rh_cgroup *group);
void rh_cgroup_close(rh_cgroup *group,int remove_leaf);
int rh_cgroup_attach(rh_cgroup *group,pid_t child);
int rh_cgroup_population(const rh_cgroup *group);
int rh_cgroup_kill(const rh_cgroup *group);
#endif
