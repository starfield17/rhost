/* Run observation is a client of the monitor, never an owner of the Run. */
#include "mux.h"
#include "state.h"
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>

#define DATA_LIMIT 16384u

int rh_control_open(int directory) {
    int cwd=open(".",O_RDONLY|O_DIRECTORY|O_CLOEXEC),fd=-1;
    if(cwd<0)return -1;
    if(fchdir(directory)==0) {
        fd=socket(AF_UNIX,SOCK_STREAM,0);
        struct sockaddr_un address;memset(&address,0,sizeof address);address.sun_family=AF_UNIX;
        strcpy(address.sun_path,"control.sock");
        struct timeval timeout={1,0};
        if(fd>=0 && (setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof timeout)<0 ||
                     setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof timeout)<0 ||
                     connect(fd,(struct sockaddr *)&address,sizeof address)<0)) {
            close(fd);fd=-1;
        }
        if(fchdir(cwd)<0){if(fd>=0)close(fd);fd=-1;}
    }
    close(cwd);return fd;
}
static int subscribe(int directory) {
    int fd=rh_control_open(directory);
    if(fd<0)return -1;
    unsigned char byte=4;
    if(write(fd,&byte,1)==1 && read(fd,&byte,1)==1 && byte==1)return fd;
    close(fd);return -1;
}
static int await_update(int *fd,uint64_t until) {
    uint64_t now=rh_millis();
    if(now>=until)return 0;
    if(*fd<0) {
        /* Legacy monitors lack subscription support. Bound the fallback so
         * their completion can still end a long watch early. */
        uint64_t delay=until-now;
        (void)poll(NULL,0,(int)(delay>250?250:delay));
        return 1;
    }
    struct pollfd ready={*fd,POLLIN,0};
    int result=poll(&ready,1,(int)(until-now));
    if(result<0)return errno==EINTR;
    if(result==0)return rh_millis()<until;
    unsigned char byte;
    if(read(*fd,&byte,1)!=1 || byte!=1 || write(*fd,&byte,1)!=1) {
        close(*fd);*fd=-1;
    }
    return 1;
}
static int snapshot(int directory,int *fd,rh_status *status,uint64_t until) {
    /* A freshly accepted Run can precede its monitor's socket bind. Retry
     * registration during that startup transition, before the snapshot used. */
    uint64_t startup=rh_millis()+1000;
    for(;;) {
        *fd=subscribe(directory);
        if(!rh_current_status(directory,status))return 0;
        uint64_t now=rh_millis();
        if(*fd>=0 || status->state!=1 || now>=startup || now>=until)return 1;
        (void)poll(NULL,0,10);
    }
}
static void counters(rh_buf *b,const rh_status *s) {
    rh_long(b,s->out_bytes);rh_long(b,s->out_kept);rh_byte(b,s->out_open);
    rh_byte(b,s->incomplete || s->out_bytes>s->out_kept);
    rh_long(b,s->err_bytes);rh_long(b,s->err_kept);rh_byte(b,s->err_open);
    rh_byte(b,s->incomplete || s->err_bytes>s->err_kept);
}
static int emit_status(uint32_t kind,rh_state *state,const char *key,const rh_status *status) {
    rh_buf b={0};rh_byte(&b,0);rh_record(&b,state,key,status);counters(&b,status);
    int result=rh_emit(kind,&b);free(b.data);return result;
}
static int data(int directory,unsigned stream,uint64_t *cursor,uint64_t kept,size_t *budget) {
    if(*cursor>=kept || !*budget)return 0;
    const char *name=stream==1?"stdout":"stderr";
    int fd=openat(directory,name,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
    struct stat info;
    if(fd<0)return 255;
    int result=255;
    if(fstat(fd,&info)==0 && S_ISREG(info.st_mode) && (uint64_t)info.st_size>=kept) {
        unsigned char bytes[DATA_LIMIT];
        while(*cursor<kept && *budget) {
            size_t count=(size_t)(kept-*cursor);
            if(count>sizeof bytes)count=sizeof bytes;
            if(count>*budget)count=*budget;
            ssize_t n=pread(fd,bytes,count,(off_t)*cursor);
            if(n<0 && errno==EINTR)continue;
            if(n<=0)goto done;
            rh_buf b={0};rh_byte(&b,stream);rh_long(&b,*cursor);rh_bytes(&b,bytes,(size_t)n);
            int sent=rh_emit(RH_DATA,&b);free(b.data);
            if(sent)goto done;
            *cursor+=(uint64_t)n;*budget-=(size_t)n;
        }
        result=0;
    }
done:
    close(fd);return result;
}
static int observe(uint32_t kind,rh_state *state,const char *key,int directory,uint32_t wait,
                   uint64_t out,uint64_t err,size_t cap) {
    uint64_t until=wait?rh_millis()+wait+1:rh_millis();
    rh_status status;int fd=-1,completed=0,result=255;
    size_t out_left=cap,err_left=cap;
    if(!snapshot(directory,&fd,&status,until))goto unavailable;
    if(cap && (out>status.out_kept || err>status.err_kept)) {
        result=rh_refuse(kind,"OUTPUT_RANGE_UNAVAILABLE","requested output is not retained");
        goto done;
    }
    for(;;) {
        if(cap && status.state==4 && !completed) {
            if(emit_status(RH_COMPLETION,state,key,&status))goto done;
            completed=1;
        }
        if(cap && (data(directory,1,&out,status.out_kept,&out_left) ||
                   data(directory,2,&err,status.err_kept,&err_left)))goto unavailable;
        if((!cap && status.state>=4) || (cap && ((!status.out_open && !status.err_open && status.state>=4) ||
             !out_left || !err_left)))break;
        if(rh_millis()>=until)break;
        int wake=await_update(&fd,until);
        if(!rh_current_status(directory,&status))goto unavailable;
        if(!wake) {
            /* Use a final authoritative snapshot even when the monitor exited
             * just before registration or the observation budget expired. */
            until=0;
        }
    }
    result=emit_status(kind,state,key,&status);
    goto done;
unavailable:
    result=rh_refuse(kind,"STATE_UNAVAILABLE","cannot establish retained Run evidence");
done:
    if(fd>=0)close(fd);
    return result;
}
int rh_watch(uint32_t kind,rh_state *state,const char *key,int directory,uint32_t wait) {
    return observe(kind,state,key,directory,wait,0,0,0);
}
int rh_follow(rh_reader *r) {
    char *generation=rh_read_text(r),*key=rh_read_text(r);
    uint64_t out=rh_read_long(r),err=rh_read_long(r),expiry;
    uint32_t wait=rh_read_int(r),cap=rh_read_int(r);
    rh_state state;int directory=-1,result=255;
    if(!rh_done(r) || !rh_generation(generation) || !rh_key(key,&expiry) || wait>300000 ||
       cap>1048576 || (!cap && (out || err))) {
        result=rh_refuse(RH_FOLLOW,"PROTOCOL_ERROR","invalid FOLLOW request");goto done;
    }
    const char *context=rh_context_error();
    if(context){result=rh_refuse(RH_FOLLOW,context,"boot guard refused observation");goto done;}
    if(!rh_state_open(&state,0)) {
        result=rh_refuse(RH_FOLLOW,"STATE_UNAVAILABLE","remote namespace is unavailable");goto done;
    }
    if(strcmp(generation,state.generation)) {
        result=rh_refuse(RH_FOLLOW,"NAMESPACE_MISMATCH","remote namespace changed");goto state_done;
    }
    directory=openat(state.runs,key,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(!rh_valid_run_directory(directory)) {
        result=rh_refuse(RH_FOLLOW,"RUN_NOT_FOUND","Run is not available");goto state_done;
    }
    (void)flock(state.registry,LOCK_UN);
    result=observe(RH_FOLLOW,&state,key,directory,wait,out,err,cap);
state_done:
    if(directory>=0)close(directory);
    rh_state_close(&state);
done:
    free(generation);free(key);return result;
}
