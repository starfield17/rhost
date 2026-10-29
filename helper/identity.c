#include "files.h"
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
int rh_binary_hash(const char *executable,char out[65]){
    int fd=-1;
#ifdef __linux__
    /* This kernel-owned link names the running image, including an unlinked one. */
    fd=open("/proc/self/exe",O_RDONLY|O_CLOEXEC);
#endif
    if(fd<0)fd=open(executable,O_RDONLY|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC);
    if(fd<0)return 0;struct stat before,after;int ok=0;
    if(fstat(fd,&before)<0 || !S_ISREG(before.st_mode) || before.st_size<=0 || before.st_size>RH_LIMIT)goto done;
    rh_hash hash;rh_hash_init(&hash);unsigned char bytes[65536];size_t total=0;
    for(;;){size_t cap=RH_LIMIT-total+1;if(cap>sizeof bytes)cap=sizeof bytes;ssize_t n=read(fd,bytes,cap);
        if(n<0 && errno==EINTR)continue;if(n<0)goto done;if(!n)break;total+=(size_t)n;if(total>RH_LIMIT)goto done;rh_hash_update(&hash,bytes,(size_t)n);
    }
    if(fstat(fd,&after)<0 || !f_identity(&before,&after) || total!=(size_t)before.st_size)goto done;
    unsigned char digest[32];rh_hash_finish(&hash,digest);rh_hex(digest,32,out);ok=1;
done:close(fd);return ok;
}
