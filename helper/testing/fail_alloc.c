/* Local instrumentation only; never part of a runtime helper artifact. */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <stdlib.h>
#include <string.h>
static unsigned long counter;
static int fail(void){
    const char *text=getenv("RHOST_FAIL_ALLOC_AT");
    if(!text || !*text)return 0;
    unsigned long target=strtoul(text,NULL,10);
    if(++counter!=target)return 0;
    errno=ENOMEM;return 1;
}
void *rh_test_malloc(size_t n){return fail()?NULL:malloc(n);}
void *rh_test_calloc(size_t n,size_t size){return fail()?NULL:calloc(n,size);}
void *rh_test_realloc(void *p,size_t n){return fail()?NULL:realloc(p,n);}
char *rh_test_strdup(const char *s){return fail()?NULL:strdup(s);}
