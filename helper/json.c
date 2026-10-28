#include "protocol.h"
#include "json.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
void j_append(j_buf *b,const void *p,size_t n) {
    if(b->failed || n>b->limit || b->size>b->limit-n){b->failed=1;return;}
    if(b->size+n>b->capacity){
        size_t capacity=b->capacity?b->capacity:64;
        while(capacity<b->size+n){if(capacity>b->limit/2){capacity=b->limit;break;}capacity*=2;}
        unsigned char *next=realloc(b->data,capacity);if(!next){b->failed=1;return;}b->data=next;b->capacity=capacity;
    }
    if(n)memcpy(b->data+b->size,p,n);b->size+=n;
}
j_value *j_new(unsigned type) {j_value *v=calloc(1,sizeof *v);if(v)v->type=type;return v;}
void j_free(j_value *v) {
    while(v){j_value *next=v->next;j_free(v->first);free(v->text);free(v->key);free(v);v=next;}
}
int j_add(j_value *parent,const char *name,j_value *child) {
    if(!parent || !child){j_free(child);return 0;}
    if(name){child->key=strdup(name);if(!child->key){j_free(child);return 0;}child->key_size=strlen(name);}
    j_value **tail=&parent->first;while(*tail)tail=&(*tail)->next;*tail=child;return 1;
}
int j_take(j_value *parent,const char *name,j_value **child){j_value *value=*child;*child=NULL;return j_add(parent,name,value);}
j_value *j_get(const j_value *parent,const char *name) {
    if(!parent || parent->type!=J_OBJECT)return NULL;
    size_t n=strlen(name);for(j_value *v=parent->first;v;v=v->next)if(v->key_size==n && !memcmp(v->key,name,n))return v;return NULL;
}
const char *j_string(const j_value *v) {return v && v->type==J_TEXT && !memchr(v->text,0,v->size)?(const char *)v->text:NULL;}
int j_text(j_value *parent,const char *name,const void *text,size_t size) {
    j_value *v=j_new(J_TEXT);if(!v)return 0;v->text=malloc(size+1);
    if(!v->text){free(v);return 0;}if(size)memcpy(v->text,text,size);v->text[size]=0;v->size=size;return j_add(parent,name,v);
}
int j_integer(j_value *p,const char *name,uint64_t n){j_value *v=j_new(J_INT);if(v)v->number=n;return j_add(p,name,v);}
int j_boolean(j_value *p,const char *name,int n){j_value *v=j_new(J_BOOL);if(v)v->number=n?1:0;return j_add(p,name,v);}
typedef struct {const unsigned char *p;size_t size,at,nodes;int failed;} parser;
static void space(parser *p){while(p->at<p->size && (p->p[p->at]==' '||p->p[p->at]=='\n'||p->p[p->at]=='\r'||p->p[p->at]=='\t'))p->at++;}
static int digit(unsigned c){return c>='0' && c<='9';}
static int hex(unsigned c){if(digit(c))return (int)(c-'0');if(c>='a' && c<='f')return (int)(c-'a'+10);if(c>='A' && c<='F')return (int)(c-'A'+10);return -1;}
static unsigned quad(parser *p){
    unsigned value=0;if(p->size-p->at<4){p->failed=1;return 0;}
    for(unsigned i=0;i<4;i++){int h=hex(p->p[p->at++]);if(h<0){p->failed=1;return 0;}value=(value<<4)|(unsigned)h;}return value;
}
static void scalar(j_buf *b,unsigned code){
    unsigned char v[4];size_t n;
    if(code<128){v[0]=(unsigned char)code;n=1;}
    else if(code<2048){v[0]=(unsigned char)(192|(code>>6));v[1]=(unsigned char)(128|(code&63));n=2;}
    else if(code<65536){v[0]=(unsigned char)(224|(code>>12));v[1]=(unsigned char)(128|((code>>6)&63));v[2]=(unsigned char)(128|(code&63));n=3;}
    else{v[0]=(unsigned char)(240|(code>>18));v[1]=(unsigned char)(128|((code>>12)&63));v[2]=(unsigned char)(128|((code>>6)&63));v[3]=(unsigned char)(128|(code&63));n=4;}
    j_append(b,v,n);
}
static unsigned char *string(parser *p,size_t *size){
    j_buf b={0};b.limit=J_INPUT_LIMIT;int closed=0;
    if(p->at==p->size || p->p[p->at++]!='"'){p->failed=1;return NULL;}
    while(p->at<p->size && !p->failed){
        unsigned char c=p->p[p->at++];if(c=='"'){closed=1;break;}
        if(c<32){p->failed=1;break;}
        if(c!='\\'){j_append(&b,&c,1);continue;}
        if(p->at==p->size){p->failed=1;break;}
        c=p->p[p->at++];
        switch(c){
        case '"':case '\\':case '/':j_append(&b,&c,1);break;
        case 'b':c=8;j_append(&b,&c,1);break;
        case 'f':c=12;j_append(&b,&c,1);break;
        case 'n':c=10;j_append(&b,&c,1);break;
        case 'r':c=13;j_append(&b,&c,1);break;
        case 't':c=9;j_append(&b,&c,1);break;
        case 'u':{
            unsigned code=quad(p);
            if(code>=0xd800 && code<=0xdbff){
                if(p->size-p->at<6 || p->p[p->at]!='\\' || p->p[p->at+1]!='u'){p->failed=1;break;}
                p->at+=2;unsigned low=quad(p);if(low<0xdc00 || low>0xdfff){p->failed=1;break;}
                code=0x10000+((code-0xd800)<<10)+(low-0xdc00);
            }else if(code>=0xdc00 && code<=0xdfff){p->failed=1;break;}
            scalar(&b,code);break;
        }
        default:p->failed=1;break;
        }
    }
    if(!closed || p->failed || b.failed || !rh_utf8(b.data,b.size,1)){free(b.data);p->failed=1;return NULL;}
    *size=b.size;unsigned char zero=0;j_append(&b,&zero,1);
    if(b.failed){free(b.data);p->failed=1;return NULL;}return b.data;
}
static j_value *value(parser *p,unsigned depth){
    if(depth>32 || ++p->nodes>65536){p->failed=1;return NULL;}space(p);
    if(p->at==p->size){p->failed=1;return NULL;}
    unsigned c=p->p[p->at];j_value *v=NULL;
    if(c=='"'){v=j_new(J_TEXT);if(v)v->text=string(p,&v->size);if(!v || !v->text)goto fail;return v;}
    if(c=='{' || c=='['){
        unsigned closing=c=='{'?'}':']';v=j_new(c=='{'?J_OBJECT:J_ARRAY);if(!v)goto fail;p->at++;space(p);
        if(p->at<p->size && p->p[p->at]==closing){p->at++;return v;}
        j_value **tail=&v->first;size_t members=0;
        for(;;){
            char *name=NULL;size_t name_size=0;
            if(c=='{'){
                if(++members>256)goto fail;
                name=(char *)string(p,&name_size);if(!name)goto fail;space(p);
                if(p->at==p->size || p->p[p->at++]!=':'){free(name);goto fail;}
                for(j_value *old=v->first;old;old=old->next)if(old->key_size==name_size && !memcmp(old->key,name,name_size)){free(name);goto fail;}
            }
            j_value *child=value(p,depth+1);if(!child){free(name);goto fail;}child->key=name;child->key_size=name_size;*tail=child;tail=&child->next;
            space(p);if(p->at==p->size)goto fail;
            unsigned token=p->p[p->at++];if(token==closing)return v;if(token!=',')goto fail;space(p);
        }
    }
    if(c=='t' || c=='f' || c=='n'){
        const char *literal=c=='t'?"true":c=='f'?"false":"null";size_t n=strlen(literal);
        if(p->size-p->at<n || memcmp(p->p+p->at,literal,n))goto fail;
        p->at+=n;v=j_new(c=='n'?J_NULL:J_BOOL);if(!v)goto fail;v->number=c=='t';return v;
    }
    if(c=='-' || digit(c)){
        v=j_new(J_INT);if(!v)goto fail;size_t beginning=p->at;
        if(c=='-'){v->negative=1;p->at++;}if(p->at==p->size || !digit(p->p[p->at]))goto fail;
        unsigned first=p->p[p->at];
        do {unsigned d=p->p[p->at++]-'0';if(v->number>(UINT64_MAX-d)/10)goto fail;v->number=v->number*10+d;}while(p->at<p->size && digit(p->p[p->at]));
        if(first=='0' && p->at-beginning>(v->negative?2u:1u))goto fail;
        if(p->at<p->size && p->p[p->at]=='.'){
            v->type=J_REAL;p->at++;if(p->at==p->size || !digit(p->p[p->at]))goto fail;while(p->at<p->size && digit(p->p[p->at]))p->at++;
        }
        if(p->at<p->size && (p->p[p->at]=='e'||p->p[p->at]=='E')){
            v->type=J_REAL;p->at++;if(p->at<p->size && (p->p[p->at]=='+'||p->p[p->at]=='-'))p->at++;
            if(p->at==p->size || !digit(p->p[p->at]))goto fail;while(p->at<p->size && digit(p->p[p->at]))p->at++;
        }
        if(v->type==J_REAL){v->size=p->at-beginning;v->text=malloc(v->size+1);if(!v->text)goto fail;memcpy(v->text,p->p+beginning,v->size);v->text[v->size]=0;}
        return v;
    }
fail:p->failed=1;j_free(v);return NULL;
}
j_value *j_parse(const unsigned char *bytes,size_t size){
    parser p={bytes,size,0,0,0};if(size>J_INPUT_LIMIT)return NULL;
    j_value *v=value(&p,0);space(&p);if(p.failed || p.at!=p.size){j_free(v);return NULL;}return v;
}
static void quote(j_buf *out,const unsigned char *s,size_t size){
    j_append(out,"\"",1);
    for(size_t i=0;i<size;i++){
        unsigned char c=s[i];
        if(c=='"'||c=='\\'){j_append(out,"\\",1);j_append(out,&c,1);}
        else if(c<32){char escaped[7];snprintf(escaped,sizeof escaped,"\\u%04x",(unsigned)c);j_append(out,escaped,6);}
        else j_append(out,&c,1);
    }
    j_append(out,"\"",1);
}
int j_dump(const j_value *v,j_buf *out){
    if(!v)return 0;
    switch(v->type){
    case J_NULL:j_append(out,"null",4);break;
    case J_BOOL:j_append(out,v->number?"true":"false",v->number?4:5);break;
    case J_INT:{char text[32];int n=snprintf(text,sizeof text,"%s%llu",v->negative?"-":"",(unsigned long long)v->number);if(n<0 || (size_t)n>=sizeof text)return 0;j_append(out,text,(size_t)n);break;}
    case J_REAL:j_append(out,v->text,v->size);break;
    case J_TEXT:quote(out,v->text,v->size);break;
    case J_ARRAY:case J_OBJECT:{
        j_append(out,v->type==J_ARRAY?"[":"{",1);int first=1;
        for(const j_value *child=v->first;child;child=child->next){
            if(!first)j_append(out,",",1);first=0;
            if(v->type==J_OBJECT){quote(out,(const unsigned char *)child->key,child->key_size);j_append(out,":",1);}
            if(!j_dump(child,out))return 0;
        }
        j_append(out,v->type==J_ARRAY?"]":"}",1);break;
    }
    default:return 0;
    }
    return !out->failed;
}

j_value *j_clone(const j_value *v){if(!v)return NULL;j_buf b={0};b.limit=J_INPUT_LIMIT;if(!j_dump(v,&b)){free(b.data);return NULL;}j_value *copy=j_parse(b.data,b.size);free(b.data);return copy;}
