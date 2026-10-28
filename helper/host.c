#include "protocol.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int rh_boot_id(char out[37]) {
    out[0]=0;
#ifdef __linux__
    int fd=open("/proc/sys/kernel/random/boot_id",O_RDONLY|O_CLOEXEC);if(fd<0)return 0;
    char bytes[38];size_t used=0;int valid=1;
    while(used<sizeof bytes){ssize_t n=read(fd,bytes+used,sizeof bytes-used);if(n<0 && errno==EINTR)continue;if(n<0){valid=0;break;}if(!n)break;used+=(size_t)n;}
    close(fd);if(!valid || (used!=36 && !(used==37 && bytes[36]=='\n')))return 0;
    for(size_t i=0;i<36;i++){
        if(i==8||i==13||i==18||i==23){if(bytes[i]!='-')return 0;}
        else if(!((bytes[i]>='0'&&bytes[i]<='9')||(bytes[i]>='a'&&bytes[i]<='f')))return 0;
    }
    memcpy(out,bytes,36);out[36]=0;return 1;
#else
    return 0;
#endif
}
const char *rh_context_error(void){
    const char *expected=getenv("RHOST_EXPECT_BOOT_ID");if(!expected || !*expected)return NULL;
    char actual[37];if(!rh_boot_id(actual))return "CONTEXT_UNAVAILABLE";
    return strcmp(expected,actual)?"CONTEXT_CHANGED":NULL;
}
