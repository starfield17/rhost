#define _XOPEN_SOURCE 700
#include "state.h"
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
static int valid_directory(int fd) {
    struct stat info;return fd>=0 && fstat(fd,&info)==0 && info.st_uid==geteuid() && !(info.st_mode&0077);
}
static int current_status(int directory,rh_status *status) {
    if(!rh_status_read(directory,status))return 0;
    int lock=openat(directory,"owner.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC);
    if(lock<0)return 0;
    if(flock(lock,LOCK_EX|LOCK_NB)==0) {
        /* Re-read after obtaining liveness ownership to retain a just-published exit. */
        if(!rh_status_read(directory,status)){close(lock);return 0;}
        if(status->state!=4 && status->state!=5) {
            status->state=5;status->completion=status->value=0;
            status->out_open=status->err_open=0;status->incomplete=1;
            if(!rh_status_write(directory,status)){close(lock);return 0;}
        }
    }else if(errno!=EWOULDBLOCK && errno!=EAGAIN){close(lock);return 0;}
    close(lock);return 1;
}
static int stream(rh_buf *b,int directory,const char *name,uint64_t from,uint32_t cap,uint64_t observed,uint64_t kept,unsigned open,unsigned incomplete) {
    if(from>kept)return 0;
    uint64_t available=kept-from;if(available>cap)available=cap;
    unsigned char *bytes=malloc((size_t)available+1);if(!bytes)return 0;
    size_t used=0;
    if(available) {
        int fd=openat(directory,name,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
        struct stat info;
        if(fd<0 || fstat(fd,&info)<0 || !S_ISREG(info.st_mode) || (uint64_t)info.st_size<kept) {if(fd>=0)close(fd);free(bytes);return 0;}
        while(used<(size_t)available) {
            ssize_t n=pread(fd,bytes+used,(size_t)available-used,(off_t)(from+used));
            if(n<0 && errno==EINTR)continue;
            if(n<=0){close(fd);free(bytes);return 0;}used+=(size_t)n;
        }
        close(fd);
    }
    rh_long(b,from);rh_long(b,from+used);rh_long(b,observed);rh_bytes(b,bytes,used);
    rh_byte(b,open);rh_byte(b,incomplete || observed>kept);free(bytes);return !b->failed;
}
static int observation(uint32_t kind,rh_state *s,const char *key,int directory,uint32_t wait,uint64_t out,uint64_t err,uint32_t cap,int start) {
    uint64_t until=rh_millis()+wait;rh_status status;
    for(;;) {
        if(!current_status(directory,&status))return rh_refuse(kind,"STATE_UNAVAILABLE","cannot establish Run evidence");
        if(!wait || status.state>=4 || (!start && (status.out_kept>out || status.err_kept>err)) || rh_millis()>=until)break;
        struct timespec pause={0,25000000};nanosleep(&pause,NULL);
    }
    if(out>status.out_kept || err>status.err_kept)return rh_refuse(kind,"OUTPUT_RANGE_UNAVAILABLE","requested output is not retained");
    rh_buf b={0};rh_byte(&b,0);rh_record(&b,s,key,&status);
    if(!stream(&b,directory,"stdout",out,cap,status.out_bytes,status.out_kept,status.out_open,status.incomplete) ||
       !stream(&b,directory,"stderr",err,cap,status.err_bytes,status.err_kept,status.err_open,status.incomplete)) {
        free(b.data);return rh_refuse(kind,"STATE_UNAVAILABLE","cannot read retained output");
    }
    int code=rh_emit(kind,&b);free(b.data);return code;
}
static int submit(uint32_t kind,rh_reader *r) {
    rh_start q;
    if(!rh_parse_start(r,&q)){rh_start_free(&q);return rh_refuse(kind,"PROTOCOL_ERROR","invalid start request");}
    const char *context=rh_context_error();if(context){rh_start_free(&q);return rh_refuse(kind,context,"boot guard refused Run submission");}
    rh_state s;
    if(!rh_state_open(&s,0)){rh_start_free(&q);return rh_refuse(kind,"STATE_UNAVAILABLE","prepare the remote namespace first");}
    const char *error=NULL;
    if(strcmp(q.generation,s.generation)){error="NAMESPACE_MISMATCH";goto refused;}
    uint64_t now;
    if(!rh_clock(&s,&now)){error="STATE_UNAVAILABLE";goto refused;}
    if(q.expiry<=now || q.expiry-now>604800){error="REQUEST_EXPIRED";goto refused;}
    /* The final fields are observation wait (4) and execution deadline (8). */
    rh_buf canonical={0};rh_raw(&canonical,r->data,r->size-12);rh_raw(&canonical,r->data+r->size-8,8);
    if(canonical.failed){free(canonical.data);error="STATE_UNAVAILABLE";goto refused;}
    unsigned char hash[32];rh_sha256(canonical.data,canonical.size,hash);free(canonical.data);
    int directory=openat(s.runs,q.key,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
    if(directory>=0) {
        unsigned char *previous=NULL;size_t size=0;
        int match=valid_directory(directory) && rh_file(directory,"fingerprint",&previous,&size,32) && size==32 && !memcmp(previous,hash,32);
        free(previous);
        if(!match){close(directory);error="IDEMPOTENCY_CONFLICT";goto refused;}
    }else {
        if(errno!=ENOENT){error="STATE_UNAVAILABLE";goto refused;}
        char *expanded=NULL;const char *context=q.cwd;
        if(q.cwd[0]=='~' && (q.cwd[1]=='/' || q.cwd[1]==0)) {
            const char *home=getenv("HOME");if(!home){error="CONFIG_INVALID";goto refused;}
            expanded=malloc(strlen(home)+strlen(q.cwd)+1);
            if(!expanded){error="STATE_UNAVAILABLE";goto refused;}
            strcpy(expanded,home);strcat(expanded,q.cwd+1);context=expanded;
        }
        char *resolved=realpath(context,NULL);free(expanded);
        if(!resolved){error="CONFIG_INVALID";goto refused;}
        free(q.cwd);q.cwd=resolved;
        char random[33],staging[64];
        if(!rh_random(random,16)){error="STATE_UNAVAILABLE";goto refused;}
        snprintf(staging,sizeof staging,".stage-%s",random);
        if(mkdirat(s.runs,staging,0700)<0){error="STATE_UNAVAILABLE";goto refused;}
        directory=openat(s.runs,staging,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        if(directory<0){error="STATE_UNAVAILABLE";goto refused;}
        int lock=openat(directory,"owner.lock",O_RDWR|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);
        rh_status initial={0};initial.state=1;
        if(lock<0 || flock(lock,LOCK_EX)<0 || !rh_atomic(directory,"request",r->data,r->size) ||
           !rh_atomic(directory,"fingerprint",hash,32) || !rh_atomic(directory,"stdin",q.input,q.input_size) ||
           !rh_status_write(directory,&initial) || renameat(s.runs,staging,s.runs,q.key)<0 || fsync(s.runs)<0) {
            if(lock>=0)close(lock);close(directory);error="STATE_UNAVAILABLE";goto refused;
        }
        pid_t child=fork();
        if(child==0) {
            /* No inherited registry lock or SSH pipe may hold a submission open. */
            rh_state_close(&s);
            if(setsid()<0)_exit(255);
            pid_t monitor=fork();if(monitor<0)_exit(255);if(monitor>0)_exit(0);
            rh_monitor(directory,lock,&q);
        }
        close(lock);
        /* A failed fork leaves accepted evidence; never submit the work again. */
        if(child>0) {int code;while(waitpid(child,&code,0)<0 && errno==EINTR){}}
    }
    if(flock(s.registry,LOCK_UN)<0){close(directory);error="STATE_UNAVAILABLE";goto refused;}
    int result=observation(kind,&s,q.key,directory,q.wait,0,0,4096,1);
    close(directory);rh_state_close(&s);rh_start_free(&q);return result;
refused:
    rh_state_close(&s);rh_start_free(&q);return rh_refuse(kind,error,"Run submission refused");
}
static int signal_run(uint32_t kind,int directory,unsigned tag,const char *key) {
    int cwd=open(".",O_RDONLY|O_DIRECTORY|O_CLOEXEC);int fd=-1;unsigned char delivered=0;
    if(cwd<0 || fchdir(directory)<0)goto done;
    fd=socket(AF_UNIX,SOCK_STREAM,0);
    struct sockaddr_un address;memset(&address,0,sizeof address);address.sun_family=AF_UNIX;strcpy(address.sun_path,"control.sock");
    struct timeval timeout={1,0};
    if(fd<0 || setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof timeout)<0 || setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof timeout)<0)goto done;
    if(connect(fd,(struct sockaddr *)&address,sizeof address)<0)goto done;
    unsigned char message=(unsigned char)tag;
    if(write(fd,&message,1)!=1 || read(fd,&delivered,1)!=1)delivered=0;
done:
    if(cwd>=0){(void)fchdir(cwd);close(cwd);}if(fd>=0)close(fd);
    if(!delivered)return rh_refuse(kind,"RUN_CONTROL_UNAVAILABLE","monitor did not confirm signal delivery");
    rh_buf b={0};rh_byte(&b,0);rh_text(&b,key);rh_byte(&b,1);int result=rh_emit(kind,&b);free(b.data);return result;
}
int rh_runs(uint32_t kind,rh_reader *r) {
    if(kind==2)return submit(kind,r);
    char *generation=NULL,*key=NULL;uint32_t wait=0,cap=4096;uint64_t out=0,err=0,expiry;unsigned signal_tag=0;
    if(kind!=7 && kind!=8)generation=rh_read_text(r);
    if(kind==3 || kind==5)key=rh_read_text(r);
    if(kind==3){wait=rh_read_int(r);out=rh_read_long(r);err=rh_read_long(r);cap=rh_read_int(r);}
    if(kind==5)signal_tag=rh_read_byte(r);
    if(!rh_done(r) || ((kind!=7 && kind!=8) && !rh_generation(generation)) ||
       ((kind==3 || kind==5) && !rh_key(key,&expiry)) || wait>300000 || !cap || cap>1048576 || (kind==5 && (signal_tag<1 || signal_tag>3))) {
        free(generation);free(key);return rh_refuse(kind,"PROTOCOL_ERROR","invalid Run request");
    }
    const char *context=rh_context_error();if(context){free(generation);free(key);return rh_refuse(kind,context,"boot guard refused operation");}
    rh_state s;
    if(!rh_state_open(&s,kind==7)){free(generation);free(key);return rh_refuse(kind,"STATE_UNAVAILABLE","remote namespace is unavailable");}
    int result=255;
    if(generation && strcmp(generation,s.generation)){result=rh_refuse(kind,"NAMESPACE_MISMATCH","remote namespace changed");goto done;}
    if(kind==7 || kind==8) {
        rh_buf b={0};rh_byte(&b,0);rh_text(&b,s.generation);
        uint64_t now=0;if(!(kind==7?rh_clock(&s,&now):rh_clock_read(&s,&now))){free(b.data);result=rh_refuse(kind,"STATE_UNAVAILABLE","cannot establish effective clock");goto done;}
        rh_long(&b,now);char boot[37];(void)rh_boot_id(boot);rh_text(&b,boot);result=rh_emit(kind,&b);free(b.data);goto done;
    }
    if(kind==3 || kind==5) {
        int directory=openat(s.runs,key,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        if(!valid_directory(directory)){if(directory>=0)close(directory);result=rh_refuse(kind,"RUN_NOT_FOUND","Run is not available");goto done;}
        (void)flock(s.registry,LOCK_UN);
        if(kind==3)result=observation(kind,&s,key,directory,wait,out,err,cap,0);
        else result=signal_run(kind,directory,signal_tag,key);
        close(directory);goto done;
    }
    /* Listing and pruning are bounded, serialized operations, never partial success. */
    int duplicate=dup(s.runs);DIR *entries=duplicate<0?NULL:fdopendir(duplicate);
    if(!entries){if(duplicate>=0)close(duplicate);result=rh_refuse(kind,"STATE_UNAVAILABLE","cannot list Runs");goto done;}
    rh_buf records={0};uint32_t count=0;struct dirent *entry;int failed=0;
    uint64_t now=0;if(kind==6 && !rh_clock(&s,&now))failed=1;
    while(!failed && (entry=readdir(entries))) {
        if(!rh_key(entry->d_name,&expiry))continue;
        int directory=openat(s.runs,entry->d_name,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
        rh_status status;
        if(!valid_directory(directory) || !current_status(directory,&status)){if(directory>=0)close(directory);failed=1;break;}
        if(kind==4) {
            if(count>=4096){failed=1;close(directory);break;}
            rh_record(&records,&s,entry->d_name,&status);count++;
        }else if(kind==6 && expiry<=now && status.state==4) {
            int lock=openat(directory,"owner.lock",O_RDWR|O_NOFOLLOW|O_CLOEXEC);
            if(lock>=0 && flock(lock,LOCK_EX|LOCK_NB)==0) {
                const char *names[]={"request","fingerprint","stdin","stdout","stderr","status","control.sock","owner.lock"};
                int removed=1;for(unsigned i=0;i<sizeof names/sizeof names[0];i++)if(unlinkat(directory,names[i],0)<0 && errno!=ENOENT)removed=0;
                if(!removed || unlinkat(s.runs,entry->d_name,AT_REMOVEDIR)<0 || fsync(s.runs)<0)failed=1;
                else {rh_text(&records,entry->d_name);count++;}
            }
            if(lock>=0)close(lock);
        }
        close(directory);
        if(records.failed || count>4096)failed=1;
    }
    closedir(entries);
    if(failed)result=rh_refuse(kind,"STATE_UNAVAILABLE","cannot complete bounded Run operation");
    else {rh_buf b={0};rh_byte(&b,0);rh_int(&b,count);rh_raw(&b,records.data,records.size);result=rh_emit(kind,&b);free(b.data);}
    free(records.data);
done:
    rh_state_close(&s);free(generation);free(key);return result;
}
