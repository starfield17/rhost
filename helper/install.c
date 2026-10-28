#define _POSIX_C_SOURCE 200809L
#include "state.h"
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <sys/wait.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>
static int handshake(int directory) {
    int input[2],output[2];
    if(pipe(input)<0)return 0;
    if(pipe(output)<0){close(input[0]);close(input[1]);return 0;}
    pid_t child=fork();
    if(child==0) {
        close(input[1]);close(output[0]);
        if(dup2(input[0],0)<0 || dup2(output[1],1)<0 || fchdir(directory)<0)_exit(255);
        close(input[0]);close(output[1]);
        execl("./candidate","./candidate",(char *)NULL);_exit(255);
    }
    close(input[0]);close(output[1]);
    if(child<0){close(input[1]);close(output[0]);return 0;}
    unsigned char request[16]={'R','H','0','1',0,0,0,1,0,0,0,0,0,0,0,0};
    int ok=write(input[1],request,16)==16;close(input[1]);
    unsigned char response[512];size_t size=0;
    while(size<sizeof response) {ssize_t n=read(output[0],response+size,sizeof response-size);if(n<0){ok=0;break;}if(!n)break;size+=(size_t)n;}
    close(output[0]);
    if(size==sizeof response){(void)kill(child,SIGKILL);ok=0;}
    int status=0;if(waitpid(child,&status,0)!=child || !WIFEXITED(status) || WEXITSTATUS(status)!=0)ok=0;
    if(!ok || size<17 || memcmp(response,"RH01",4) || rh_u32(response+4)!=UINT32_C(0x80000001) || rh_u64(response+8)!=size-16)return 0;
    rh_reader r={response,size,16,0};
    if(rh_read_byte(&r)!=0)return 0;
    char *digest=rh_read_text(&r),*os=rh_read_text(&r),*arch=rh_read_text(&r);
    (void)rh_read_int(&r);
    ok=rh_done(&r) && digest && !strcmp(digest,RHOST_SOURCE_DIGEST) && os && *os && arch && *arch;
    free(digest);free(os);free(arch);return ok;
}
int rh_install(const char *path,const char *executable) {
    int directory=rh_directory(path,1);struct stat info;int result=255,lock=-1,fd=-1;
    unsigned phase=1;
    unsigned char *bytes=NULL;
    if(directory<0 || fstat(directory,&info)<0 || info.st_uid!=geteuid() || (info.st_mode&0077))goto done;
    phase=2;lock=openat(directory,"install.lock",O_RDWR|O_CREAT|O_NOFOLLOW|O_CLOEXEC,0600);
    if(lock<0 || fstat(lock,&info)<0 || !S_ISREG(info.st_mode) || info.st_uid!=geteuid() || (info.st_mode&0077) || flock(lock,LOCK_EX)<0)goto done;
    phase=3;fd=open(executable,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
    if(fd<0 || fstat(fd,&info)<0 || !S_ISREG(info.st_mode) || info.st_size<=0 || info.st_size>RH_LIMIT)goto done;
    size_t size=(size_t)info.st_size,at=0;bytes=malloc(size);
    if(!bytes)goto done;
    while(at<size){ssize_t n=read(fd,bytes+at,size-at);if(n<=0)goto done;at+=(size_t)n;}
    close(fd);fd=-1;
    phase=4;if(!rh_atomic(directory,"candidate",bytes,size))goto done;
    fd=openat(directory,"candidate",O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
    if(fd<0 || fchmod(fd,0700)<0 || fsync(fd)<0)goto done;
    close(fd);fd=-1;
    phase=5;if(!handshake(directory))goto done;
    phase=6;
    if(renameat(directory,"candidate",directory,"rhost-helper")<0 || fsync(directory)<0)goto done;
    result=0;
done:
    if(result!=0)fprintf(stderr,"rhost helper installation failed at stage %u\n",phase);
    free(bytes);if(fd>=0)close(fd);if(lock>=0)close(lock);if(directory>=0)close(directory);return result;
}
