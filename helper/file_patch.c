#include "files.h"
#include <stdlib.h>
#include <string.h>
static int base64(unsigned c){
    if(c>='A'&&c<='Z')return (int)(c-'A');if(c>='a'&&c<='z')return (int)(c-'a'+26);
    if(c>='0'&&c<='9')return (int)(c-'0'+52);if(c=='+')return 62;if(c=='/')return 63;return -1;
}
int f_decode(const j_value *content,j_buf *out,f_error *e){
    if(!content || content->type!=J_TEXT || content->size%4){f_fail(e,"CONFIG_INVALID","content must be valid base64 text");return 0;}
    out->limit=F_EDIT_LIMIT;
    for(size_t i=0;i<content->size;i+=4){
        const unsigned char *s=content->text+i;int a=base64(s[0]),b=base64(s[1]),c=base64(s[2]),d=base64(s[3]);
        if(a<0||b<0 || (s[2]=='=' && (s[3]!='=' || i+4!=content->size)) || (s[3]=='=' && i+4!=content->size) || (c<0 && s[2]!='=') || (d<0 && s[3]!='=')){
            f_fail(e,"CONFIG_INVALID","content must be valid base64 text");return 0;}
        unsigned char raw[3]={(unsigned char)((a<<2)|(b>>4)),(unsigned char)((b<<4)|((c<0?0:c)>>2)),(unsigned char)(((c<0?0:c)<<6)|(d<0?0:d))};
        j_append(out,raw,s[2]=='='?1:s[3]=='='?2:3);
        if(out->failed){f_fail(e,"FILE_TOO_LARGE","editing limit is 8388608 bytes");return 0;}
    }
    if(!rh_utf8(out->data,out->size,1)){f_fail(e,"INVALID_TEXT","content must be UTF-8");return 0;}return 1;
}
typedef struct {size_t start,end;const j_value *text;} span;
static int order(const void *left,const void *right){const span *a=left,*b=right;return a->start<b->start?-1:a->start>b->start;}
static size_t keys(const j_value *v){size_t n=0;for(v=v->first;v;v=v->next)n++;return n;}
int f_patch(const j_value *edits,const j_buf *old,j_buf *out,f_error *e){
    if(!edits || edits->type!=J_ARRAY || !edits->first){f_fail(e,"INVALID_PATCH","edits must be a nonempty array");return 0;}
    if(!rh_utf8(old->data,old->size,1)){f_fail(e,"INVALID_TEXT","file is not UTF-8 text");return 0;}
    size_t count=keys(edits);span *spans=calloc(count,sizeof *spans);if(!spans){f_fail(e,"INTERNAL","allocation failed");return 0;}
    int kind=0;size_t used=0;out->limit=F_EDIT_LIMIT;
    uint64_t total=0;for(size_t i=0;i<old->size;i++)if(old->data[i]=='\n')total++;
    if(old->size && old->data[old->size-1]!='\n')total++;
    for(const j_value *edit=edits->first;edit;edit=edit->next){
        if(edit->type!=J_OBJECT)goto invalid;
        j_value *range=j_get(edit,"start"),*target=j_get(edit,"old");span *s=&spans[used++];
        if(range && keys(edit)==3 && j_get(edit,"end") && j_get(edit,"text")){
            if(kind==2)goto invalid;kind=1;uint64_t start,end;
            if(!f_number(edit,"start",0,&start)||!f_number(edit,"end",0,&end)||!start||end<start||end>total)goto invalid;
            s->text=j_get(edit,"text");if(s->text->type!=J_TEXT)goto invalid;
            uint64_t row=1;size_t at=0;
            while(at<old->size && row<start)if(old->data[at++]=='\n')row++;
            if(row!=start || at==old->size)goto invalid;s->start=at;
            while(at<old->size && row<=end)if(old->data[at++]=='\n')row++;
            if(row<end || (row==end && at==s->start))goto invalid;
            s->end=at;
        }else if(target && keys(edit)==2 && j_get(edit,"new")){
            if(kind==1)goto invalid;kind=2;s->text=j_get(edit,"new");if(target->type!=J_TEXT || !target->size || s->text->type!=J_TEXT)goto invalid;
            size_t matches=0;
            if(target->size<=old->size)for(size_t i=0;i<=old->size-target->size;i++)if(!memcmp(old->data+i,target->text,target->size)){
                s->start=i;s->end=i+target->size;if(++matches>1)break;
            }
            if(matches!=1)goto invalid;
        }else goto invalid;
    }
    qsort(spans,count,sizeof *spans,order);size_t previous=0;
    for(size_t i=0;i<count;i++){
        span *s=&spans[i];if(s->start<previous)goto invalid;
        j_append(out,old->data+previous,s->start-previous);j_append(out,s->text->text,s->text->size);previous=s->end;
    }
    j_append(out,old->data+previous,old->size-previous);free(spans);
    if(out->failed){f_fail(e,"FILE_TOO_LARGE","editing limit is 8388608 bytes");return 0;}return 1;
invalid:free(spans);f_fail(e,"INVALID_PATCH","patch needs valid nonoverlapping original ranges or unique exact text");return 0;
}
