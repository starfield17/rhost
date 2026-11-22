#define _POSIX_C_SOURCE 200809L
#include "state.h"
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;
#define CLIENTS 32
#define QUOTA (64u*1024u*1024u)
static void private_fds(void) {
    int fd=open("/dev/null",O_RDWR);
    if(fd<0)_exit(255);
    for(int i=0;i<3;i++)if(dup2(fd,i)<0)_exit(255);
    if(fd>2)close(fd);
}
static int nonblock(int fd) {
    int flags=fcntl(fd,F_GETFL);return flags>=0 && fcntl(fd,F_SETFL,flags|O_NONBLOCK)>=0;
}
static int logs(int directory,const char *name) {
    return openat(directory,name,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);
}
static int drain(int fd,int log,uint64_t *total,uint64_t *kept,unsigned *broken,unsigned *incomplete) {
    unsigned char bytes[16384];size_t budget=262144;
    while(budget) {
        ssize_t n=read(fd,bytes,sizeof bytes);
        if(n==0)return 0;
        if(n<0) {if(errno==EAGAIN || errno==EWOULDBLOCK || errno==EINTR)return 1;*incomplete=1;return 0;}
        budget-=(size_t)n;
        if(UINT64_MAX-*total<(uint64_t)n){*incomplete=1;return 0;}
        *total+=(uint64_t)n;
        if(*kept<QUOTA && !*broken) {
            size_t size=(size_t)n;if(size>QUOTA-*kept)size=(size_t)(QUOTA-*kept);
            size_t at=0;
            while(at<size) {
                ssize_t wrote=write(log,bytes+at,size-at);
                if(wrote<0 && errno==EINTR)continue;
                if(wrote<=0){*incomplete=1;*broken=1;break;}at+=(size_t)wrote;*kept+=(uint64_t)wrote;
            }
        }
        if(*kept<*total)*incomplete=1;
    }
    return 1;
}
static void command_child(int directory,int lock,rh_start *q,int out[2],int err[2],int ack[2],int socket_fd,int gate[2],rh_cgroup *group,int startup) {
    close(out[0]);close(err[0]);close(ack[0]);close(socket_fd);close(lock);
    if(startup>=0)close(startup);
    rh_cgroup_close(group,0);
    if(gate[0]>=0) {
        close(gate[1]);
        unsigned char permit;
        ssize_t received;
        do {received=read(gate[0],&permit,1);}while(received<0 && errno==EINTR);
        close(gate[0]);
        if(received!=1 || permit!=1)_exit(126);
    }
    signal(SIGPIPE,SIG_DFL);signal(SIGHUP,SIG_DFL);signal(SIGINT,SIG_DFL);
    signal(SIGTERM,SIG_DFL);signal(SIGCHLD,SIG_DFL);signal(SIGUSR1,SIG_DFL);
    sigset_t unblocked;sigemptyset(&unblocked);(void)sigprocmask(SIG_SETMASK,&unblocked,NULL);
    unsigned char ok=0;
    int input=openat(directory,"stdin",O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
    if(setsid()>=0 && input>=0 && chdir(q->cwd)==0 && dup2(input,0)>=0 && dup2(out[1],1)>=0 && dup2(err[1],2)>=0)ok=1;
    if(input>=0)close(input);
    close(out[1]);close(err[1]);close(directory);
    if(write(ack[1],&ok,1)!=1 || !ok)_exit(126);
    close(ack[1]);
    if(!q->inherit)environ=NULL;
    for(size_t i=0;i<q->envc;i++) {
        char *equals=strchr(q->env[i],'=');*equals=0;
        if(setenv(q->env[i],equals+1,1)<0)_exit(126);
    }
    execvp(q->argv[0],q->argv);_exit(errno==ENOENT?127:126);
}
void rh_monitor(int directory,int lock,rh_start *q,rh_cgroup *group,int startup) {
    private_fds();signal(SIGCHLD,SIG_DFL);signal(SIGHUP,SIG_IGN);signal(SIGPIPE,SIG_IGN);
    rh_status status={0};status.state=2;status.cgroup=q->cgroup;
    int out[2]={-1,-1},err[2]={-1,-1},ack[2]={-1,-1},gate[2]={-1,-1},socket_fd=-1;
    int out_log=-1,err_log=-1;pid_t child=-1;
    if(q->cgroup && !rh_cgroup_prepare(group))goto lost;
    if(fchdir(directory)<0)goto lost;
    socket_fd=socket(AF_UNIX,SOCK_STREAM,0);
    struct sockaddr_un address;memset(&address,0,sizeof address);address.sun_family=AF_UNIX;
    strcpy(address.sun_path,"control.sock");
    if(socket_fd<0 || bind(socket_fd,(struct sockaddr *)&address,sizeof address)<0 || listen(socket_fd,CLIENTS)<0 || !nonblock(socket_fd))goto lost;
    if(pipe(out)<0 || pipe(err)<0 || pipe(ack)<0 || (q->cgroup && pipe(gate)<0))goto lost;
    out_log=logs(directory,"stdout");err_log=logs(directory,"stderr");
    if(out_log<0 || err_log<0)goto lost;
    if(!rh_status_write(directory,&status))goto lost;
    child=fork();
    if(child<0)goto lost;
    if(child==0)command_child(directory,lock,q,out,err,ack,socket_fd,gate,group,startup);
    if(q->cgroup) {
        close(gate[0]);gate[0]=-1;
        unsigned char permit=1;
        if(!rh_cgroup_attach(group,child) || write(gate[1],&permit,1)!=1) {
            close(gate[1]);gate[1]=-1;
            while(waitpid(child,NULL,0)<0 && errno==EINTR){}
            status.containment_refused=1;
            goto lost;
        }
        close(gate[1]);gate[1]=-1;
        status.population=2;
    }
    close(out[1]);out[1]=-1;close(err[1]);err[1]=-1;close(ack[1]);ack[1]=-1;
    unsigned char ready=0;
    ssize_t acknowledged;
    do {acknowledged=read(ack[0],&ready,1);}while(acknowledged<0 && errno==EINTR);
    close(ack[0]);ack[0]=-1;
    if(acknowledged!=1 || ready!=1)ready=0;
    if(!nonblock(out[0]) || !nonblock(err[0])) {
        if(q->cgroup)(void)rh_cgroup_kill(group);
        if(ready)(void)kill(-child,SIGKILL);
        else (void)kill(child,SIGKILL);
        int ignored;while(waitpid(child,&ignored,0)<0 && errno==EINTR){}
        goto lost;
    }
    status.state=3;status.out_open=status.err_open=1;
    uint64_t started=rh_millis(),exited=0,empty_at=0;int controlling=ready,deadline_sent=0;
    unsigned out_broken=0,err_broken=0;
    if(!ready)status.incomplete=1;
    (void)rh_status_write(directory,&status);
    if(startup>=0) {
        unsigned char ready_reply=1;
        ssize_t replied=write(startup,&ready_reply,1);(void)replied;
        close(startup);startup=-1;
    }
    rh_status published=status;
    int clients[CLIENTS];uint64_t client_deadlines[CLIENTS];
    unsigned subscriptions[CLIENTS]={0};
    uint64_t revision=1,delivered[CLIENTS]={0};
    for(unsigned i=0;i<CLIENTS;i++){clients[i]=-1;client_deadlines[i]=0;}
    for(;;) {
        siginfo_t info;memset(&info,0,sizeof info);
        if(!exited) {
            int observed=waitid(P_PID,(id_t)child,&info,WEXITED|WNOHANG|WNOWAIT);
            if(observed<0 && errno!=EINTR)goto lost;
            if(observed==0 && info.si_pid==child) {
                controlling=0;
                int code;pid_t reaped;
                do {reaped=waitpid(child,&code,0);}while(reaped<0 && errno==EINTR);
                if(reaped!=child)goto lost;
                status.state=4;
                if(WIFEXITED(code)){status.completion=1;status.value=(unsigned)WEXITSTATUS(code);}
                else if(WIFSIGNALED(code)){status.completion=2;status.value=(unsigned)WTERMSIG(code);}
                else goto lost;
                exited=rh_millis();
            }
        }
        uint64_t now=rh_millis();
        if(q->cgroup) {
            status.population=(unsigned)rh_cgroup_population(group);
            if(status.population!=1 || !exited)empty_at=0;
            else if(!empty_at)empty_at=now;
        }
        if((controlling || (q->cgroup && status.population!=1)) && q->deadline && now-started>=q->deadline) {
            if(!deadline_sent){if(controlling)(void)kill(-child,SIGTERM);deadline_sent=1;}
            if(now-started>=q->deadline+1000) {
                if(q->cgroup)(void)rh_cgroup_kill(group);
                else (void)kill(-child,SIGKILL);
            }
        }
        struct pollfd fds[CLIENTS+3]={{socket_fd,POLLIN,0},{status.out_open?out[0]:-1,POLLIN,0},{status.err_open?err[0]:-1,POLLIN,0}};
        for(unsigned i=0;i<CLIENTS;i++){fds[i+3].fd=clients[i];fds[i+3].events=POLLIN;fds[i+3].revents=0;}
        (void)poll(fds,CLIENTS+3,25);
        if(status.out_open)status.out_open=(unsigned)drain(out[0],out_log,&status.out_bytes,&status.out_kept,&out_broken,&status.incomplete);
        if(status.err_open)status.err_open=(unsigned)drain(err[0],err_log,&status.err_bytes,&status.err_kept,&err_broken,&status.incomplete);
        if(fds[0].revents&POLLIN) {
            int client=accept(socket_fd,NULL,NULL);
            if(client>=0) {
                unsigned slot=0;while(slot<CLIENTS && clients[slot]>=0)slot++;
                if(slot==CLIENTS || !nonblock(client))close(client);
                else {subscriptions[slot]=0;clients[slot]=client;client_deadlines[slot]=rh_millis()+1000;}
            }
        }
        for(unsigned i=0;i<CLIENTS;i++) {
            if(clients[i]<0)continue;
            if(subscriptions[i]) {
                if(fds[i+3].revents) {
                    unsigned char tag=0;
                    if(read(clients[i],&tag,1)==1 && tag==1)subscriptions[i]=1;
                    else {close(clients[i]);clients[i]=-1;subscriptions[i]=0;}
                }
                continue;
            }
            if(fds[i+3].revents || rh_millis()>=client_deadlines[i]) {
                unsigned char tag=0,answer=0;
                ssize_t received=read(clients[i],&tag,1);
                if(received<0 && (errno==EAGAIN || errno==EWOULDBLOCK) && rh_millis()<client_deadlines[i])continue;
                if(received==1 && tag==4) {
                    answer=1;
                    if(write(clients[i],&answer,1)==1){subscriptions[i]=1;delivered[i]=revision;continue;}
                }
                if(received==1 && tag==3 && q->cgroup) {
                    if(status.population!=1)answer=(unsigned char)rh_cgroup_kill(group);
                }else if(received==1 && tag>=1 && tag<=3 && controlling) {
                    int signal_number=tag==1?SIGINT:(tag==2?SIGTERM:SIGKILL);
                    if(kill(-child,signal_number)==0)answer=1;
                }
                ssize_t replied=write(clients[i],&answer,1);
                (void)replied;close(clients[i]);clients[i]=-1;
            }
        }
        uint64_t drain_start=q->cgroup?empty_at:exited;
        if(drain_start && rh_millis()-drain_start>=2000 && (status.out_open || status.err_open)) {
            status.incomplete=1;status.out_open=status.err_open=0;
            close(out[0]);out[0]=-1;close(err[0]);err[0]=-1;
        }
        if(memcmp(&status,&published,sizeof status)!=0 && rh_status_write(directory,&status)) {
            published=status;revision++;
        }
        for(unsigned i=0;i<CLIENTS;i++)if(subscriptions[i]==1 && delivered[i]!=revision) {
            unsigned char wake=1;
            if(write(clients[i],&wake,1)==1){subscriptions[i]=2;delivered[i]=revision;}
        }
        if(exited && !status.out_open && !status.err_open && (!q->cgroup || status.population==1))break;
    }
    if(out_log>=0 && fsync(out_log)<0)status.incomplete=1;
    if(err_log>=0 && fsync(err_log)<0)status.incomplete=1;
    (void)rh_status_write(directory,&status);
    for(unsigned i=0;i<CLIENTS;i++)if(clients[i]>=0)close(clients[i]);
    goto finish;
lost:
    if(status.state!=4){status.state=5;status.completion=status.value=0;}
    status.out_open=status.err_open=0;status.incomplete=1;
    if(q->cgroup) {
        if(child<0)status.containment_refused=1;
        status.population=(unsigned)rh_cgroup_population(group);
    }
    (void)rh_status_write(directory,&status);
finish:
    if(startup>=0)close(startup);
    rh_cgroup_close(group,1);
    unlinkat(directory,"control.sock",0);
    if(socket_fd>=0)close(socket_fd);
    if(out_log>=0)close(out_log);
    if(err_log>=0)close(err_log);
    for(unsigned i=0;i<2;i++){if(out[i]>=0)close(out[i]);if(err[i]>=0)close(err[i]);if(ack[i]>=0)close(ack[i]);if(gate[i]>=0)close(gate[i]);}
    close(lock);close(directory);_exit(0);
}
