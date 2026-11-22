/* RH02 owns session workers. Detached Run monitors own their own lifetimes. */
#include "mux.h"
#include "state.h"
#include "tunnel.h"
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define WORKERS 24
#define ORDINARY 22
#define TUNNEL WORKERS
#define OVERFLOW (WORKERS+1)
#define SLOTS (WORKERS+2)
typedef struct {
    unsigned char header[RH_MUX_HEADER], *body;
    size_t at, size;
    int complete;
} frame;
typedef struct {
    int fd;
    pid_t pid;
    uint64_t id;
    uint32_t kind;
    int answered;
    frame reply;
} worker;
static uint64_t reply_id;
static volatile sig_atomic_t cancelled;

static void cancel_start(int signal_number) { (void)signal_number;cancelled=1; }
int rh_mux_cancelled(void) { return cancelled!=0; }

int rh_mux_active(void) { return reply_id != 0; }
uint64_t rh_mux_reply_id(void) { return reply_id; }

void rh_mux_header(unsigned char h[RH_MUX_HEADER],uint64_t id,uint32_t kind,size_t size) {
    memcpy(h,"RH02",4);
    rh_put32(h+4,(uint32_t)(id>>32));rh_put32(h+8,(uint32_t)id);
    rh_put32(h+12,kind|RH_REPLY);rh_put32(h+16,(uint32_t)size);
}
static int unfinished(const worker *w) {
    return w->fd>=0 || w->reply.complete;
}

/* One read per ready descriptor. Return 0 on EOF, -1 on error, 1 otherwise. */
static int receive(int fd,frame *f) {
    int header=f->at<RH_MUX_HEADER;
    unsigned char *next=header?f->header+f->at:f->body+f->at-RH_MUX_HEADER;
    size_t left=header?RH_MUX_HEADER-f->at:RH_MUX_HEADER+f->size-f->at;
    ssize_t n=read(fd,next,left);
    if(n==0)return 0;
    if(n<0)return errno==EINTR || errno==EAGAIN?1:-1;
    f->at+=(size_t)n;
    if(header && f->at==RH_MUX_HEADER) {
        if(memcmp(f->header,"RH02",4)!=0)return -1;
        f->size=rh_u32(f->header+16);
        if(f->size>RH_LIMIT)return -1;
        if(f->size && !(f->body=malloc(f->size)))return -1;
    }
    f->complete=f->at>=RH_MUX_HEADER && f->at==RH_MUX_HEADER+f->size;
    return 1;
}
static void reset(frame *f) { free(f->body);memset(f,0,sizeof *f); }
static int nonblocking(int fd) {
    int flags=fcntl(fd,F_GETFL);
    return flags>=0 && fcntl(fd,F_SETFL,flags|O_NONBLOCK)>=0;
}
static void retire(worker *w,int terminate) {
    close(w->fd);reset(&w->reply);
    if(terminate)(void)kill(w->pid,SIGKILL);
    while(waitpid(w->pid,NULL,0)<0 && errno==EINTR){}
    w->fd=-1;
}
static int receipt(worker *w,unsigned tag,const char *code,const char *message) {
    rh_buf body={0};rh_byte(&body,tag);
    rh_text(&body,code);rh_text(&body,message);rh_byte(&body,0);
    if(body.failed){free(body.data);return 0;}
    rh_mux_header(w->reply.header,w->id,w->kind,body.size);
    w->reply.body=body.data;w->reply.size=body.size;w->reply.complete=1;
    return 1;
}
static int dispatch(frame *input,worker workers[SLOTS],unsigned slot) {
    uint64_t id=rh_u64(input->header+4);
    uint32_t kind=rh_u32(input->header+12);
    if(!id || kind<2 || (kind>RH_FOLLOW && kind!=RH_FILES))return 0;
    for(unsigned i=0;i<SLOTS;i++)if(unfinished(&workers[i]) && workers[i].id==id)return 0;
    workers[slot].id=id;workers[slot].kind=kind;workers[slot].answered=0;
    if(slot==OVERFLOW)
        return receipt(&workers[slot],2,"STATE_UNAVAILABLE","RH02 request capacity is full");
    int pipefd[2];
    if(pipe(pipefd)<0)return 0;
    pid_t pid=fork();
    if(pid==0) {
        close(pipefd[0]);
        for(unsigned i=0;i<WORKERS;i++)if(workers[i].fd>=0)close(workers[i].fd);
        if(dup2(pipefd[1],STDOUT_FILENO)<0)_exit(255);
        close(pipefd[1]);close(STDIN_FILENO);
        /* Keep descriptor zero reserved for the detached monitor's stdio setup. */
        if(open("/dev/null",O_RDONLY)!=STDIN_FILENO)_exit(255);
        reply_id=id;
        if(kind==2) {
            struct sigaction action={0};
            action.sa_handler=cancel_start;sigemptyset(&action.sa_mask);
            if(sigaction(SIGUSR1,&action,NULL)<0)_exit(255);
        }
        rh_reader request={input->body,input->size,0,0};
        int result;
        if(kind==RH_FOLLOW)result=rh_follow(&request);
        else if(kind==RH_FILES)result=rh_mux_files(&request);
        else result=rh_runs(kind,&request);
        _exit(result);
    }
    close(pipefd[1]);
    if(pid<0){close(pipefd[0]);return 0;}
    workers[slot].fd=pipefd[0];workers[slot].pid=pid;
    return 1;
}
/* Only the session parent writes SSH stdout. A partially written frame owns it
 * until complete, while stdin and other worker pipes remain observable. */
static int write_frame(int fd,const frame *f,size_t *sent) {
    const unsigned char *next=*sent<RH_MUX_HEADER?f->header+*sent:f->body+*sent-RH_MUX_HEADER;
    size_t left=(*sent<RH_MUX_HEADER?RH_MUX_HEADER:RH_MUX_HEADER+f->size)-*sent;
    ssize_t n=write(fd,next,left);
    if(n<0)return errno==EINTR || errno==EAGAIN?0:-1;
    if(!n)return -1;
    *sent+=(size_t)n;
    return *sent==RH_MUX_HEADER+f->size;
}
static int is_tunnel(uint32_t kind) {
    return kind>=RH_TCP_OPEN && kind<=RH_TCP_CLOSE;
}
static int spawn_tunnel(int *tun_in,worker workers[SLOTS]) {
    int to_tun[2],from_tun[2];
    if(pipe(to_tun)<0)return 0;
    if(pipe(from_tun)<0){close(to_tun[0]);close(to_tun[1]);return 0;}
    pid_t pid=fork();
    if(pid==0) {
        close(to_tun[1]);close(from_tun[0]);
        for(unsigned i=0;i<SLOTS;i++)if(workers[i].fd>=0)close(workers[i].fd);
        int devnull=open("/dev/null",O_RDWR);
        if(devnull>=0){
            (void)dup2(devnull,STDIN_FILENO);
            (void)dup2(devnull,STDOUT_FILENO);
            if(devnull>STDERR_FILENO)close(devnull);
        }
        _exit(rh_tunnel_run(to_tun[0],from_tun[1]));
    }
    close(to_tun[0]);close(from_tun[1]);
    if(pid<0 || !nonblocking(to_tun[1])) {
        close(to_tun[1]);close(from_tun[0]);
        if(pid>0){(void)kill(pid,SIGKILL);while(waitpid(pid,NULL,0)<0 && errno==EINTR){}}
        return 0;
    }
    *tun_in=to_tun[1];
    workers[TUNNEL].fd=from_tun[0];workers[TUNNEL].pid=pid;workers[TUNNEL].id=0;workers[TUNNEL].answered=1;
    return 1;
}
int rh_mux(void) {
    frame input={0},tun_tx={0};worker workers[SLOTS]={0};
    for(unsigned i=0;i<SLOTS;i++)workers[i].fd=-1;
    if(!nonblocking(STDOUT_FILENO))return 255;
    signal(SIGPIPE,SIG_IGN);
    int code=0,tun_in=-1;
    unsigned sending=SLOTS,after=0;
    size_t sent=0,tun_sent=0;
    for(;;) {
        if(input.complete && rh_u32(input.header+12)==RH_CANCEL) {
            if(input.size || !rh_u64(input.header+4)){code=255;break;}
            for(unsigned i=0;i<WORKERS;i++)if(workers[i].fd>=0 && workers[i].id==rh_u64(input.header+4)) {
                uint32_t kind=workers[i].kind;
                if(kind==2) {
                    workers[i].answered=1;
                    (void)kill(workers[i].pid,SIGUSR1);
                }else if(kind==3 || kind==4 || kind==8 || kind==9 || kind==10 || kind==RH_FOLLOW) {
                    /* Finish an in-flight write before discarding the worker. */
                    if(i==sending && sent){workers[i].answered=1;(void)kill(workers[i].pid,SIGKILL);}
                    else {retire(&workers[i],1);if(i==sending)sending=SLOTS;}
                }
                break;
            }
            reset(&input);
        }
        if(input.complete && is_tunnel(rh_u32(input.header+12))) {
            if(!tun_tx.complete) {
                uint32_t kind=rh_u32(input.header+12);
                if(!rh_u64(input.header+4) || (kind==RH_TCP_DATA && input.size>16384)) {
                    code=255;break;
                }
                if(tun_in<0 && !spawn_tunnel(&tun_in,workers)){code=255;break;}
                tun_tx=input;tun_sent=0;memset(&input,0,sizeof input);
            }
        }
        if(input.complete && !is_tunnel(rh_u32(input.header+12))) {
            int control=rh_u32(input.header+12)==5;
            unsigned slot=control?ORDINARY:0,limit=control?WORKERS:ORDINARY;
            while(slot<limit && unfinished(&workers[slot]))slot++;
            if(slot==limit)slot=OVERFLOW;
            if(!unfinished(&workers[slot])) {
                if(!dispatch(&input,workers,slot)){code=255;break;}
                reset(&input);
            }
        }
        if(sending==SLOTS) {
            for(unsigned i=0;i<SLOTS;i++) {
                unsigned index=(after+i)%SLOTS;
                if(workers[index].reply.complete){sending=index;break;}
            }
        }
        struct pollfd fds[WORKERS+4]={
            {STDIN_FILENO,input.complete?0:POLLIN,0},
            {STDOUT_FILENO,sending==SLOTS?0:POLLOUT,0}
        };
        for(unsigned i=0;i<WORKERS;i++)
            fds[i+2]=(struct pollfd){workers[i].reply.complete?-1:workers[i].fd,POLLIN,0};
        fds[WORKERS+2]=(struct pollfd){workers[TUNNEL].reply.complete?-1:workers[TUNNEL].fd,POLLIN,0};
        fds[WORKERS+3]=(struct pollfd){(tun_in>=0 && tun_tx.complete)?tun_in:-1,POLLOUT,0};
        if(poll(fds,WORKERS+4,-1)<0){if(errno==EINTR)continue;code=255;break;}
        if(fds[0].revents&POLLIN && !input.complete) {
            int result=receive(STDIN_FILENO,&input);
            if(result<=0){code=result<0?255:0;break;}
        }
        if(fds[0].revents&(POLLHUP|POLLERR|POLLNVAL))break;
        if(fds[1].revents&(POLLHUP|POLLERR|POLLNVAL))break;
        if(sending!=SLOTS && (fds[1].revents&POLLOUT)) {
            int result=write_frame(STDOUT_FILENO,&workers[sending].reply,&sent);
            if(result<0)break;
            if(result) {
                if(sending<WORKERS && rh_u32(workers[sending].reply.header+12)==(workers[sending].kind|RH_REPLY))
                    workers[sending].answered=1;
                reset(&workers[sending].reply);
                after=(sending+1)%SLOTS;sending=SLOTS;sent=0;
            }
        }
        if(fds[WORKERS+3].revents) {
            int result=(fds[WORKERS+3].revents&(POLLHUP|POLLERR|POLLNVAL))?-1:write_frame(tun_in,&tun_tx,&tun_sent);
            if(result<0) {
                close(tun_in);tun_in=-1;reset(&tun_tx);tun_sent=0;
                if(workers[TUNNEL].fd>=0){retire(&workers[TUNNEL],1);if(sending==TUNNEL)sending=SLOTS;}
            }else if(result) {
                reset(&tun_tx);tun_sent=0;
            }
        }
        if(fds[WORKERS+2].revents) {
            int result=receive(workers[TUNNEL].fd,&workers[TUNNEL].reply);
            if(result<=0) {
                retire(&workers[TUNNEL],result<0);
                if(sending==TUNNEL)sending=SLOTS;
                if(tun_in>=0){close(tun_in);tun_in=-1;}
                reset(&tun_tx);tun_sent=0;
            }
        }
        for(unsigned i=0;i<WORKERS;i++)if(fds[i+2].revents) {
            int result=receive(workers[i].fd,&workers[i].reply);
            if(result<=0) {
                int answered=workers[i].answered;
                retire(&workers[i],result<0);
                if(!answered && !receipt(&workers[i],1,"REMOTE_EXECUTION_UNKNOWN",
                     "RH02 worker ended without a terminal receipt"))code=255;
            }
        }
        if(code)break;
    }
    reset(&input);
    reset(&tun_tx);
    if(tun_in>=0)close(tun_in);
    for(unsigned i=0;i<SLOTS;i++) {
        if(workers[i].fd>=0)retire(&workers[i],1);
        else reset(&workers[i].reply);
    }
    return code;
}
