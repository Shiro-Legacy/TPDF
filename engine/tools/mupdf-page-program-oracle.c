/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only MuPDF1.28.1 original-page compact mixed trace; PDF stdin.
 * Native page CTM and initial page-bounds clip metadata; ordered callbacks.
 * Input16MiB, native heap128MiB, decoded page4MiB/document128MiB, output64MiB.
 * Page/document work caps, clips1024, CPU30s/wall45s.
 * Atomic successful output; implicit cleanup clip pops separately counted.
 * Uses embedded or bundled Base14 fonts; system-loader callbacks cannot load files.
 */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <sys/resource.h>
#include <unistd.h>
#include "mupdf/fitz.h"
#include "mupdf/pdf.h"
#define INPUT_LIMIT (16u*1024u*1024u)
#define PAGE_CONTENT_LIMIT (4u*1024u*1024u)
#define TOTAL_DECODED_LIMIT (128u*1024u*1024u)
#define HEAP_LIMIT (128u*1024u*1024u)
#define OUTPUT_LIMIT (64u*1024u*1024u)
typedef union { size_t size; max_align_t alignment; } allocation_header;
typedef struct { size_t used; } budget;
static void *limited_malloc(void *user, size_t size)
{
    budget *b = user;
    allocation_header *h;
    if (size > HEAP_LIMIT - b->used || size > SIZE_MAX - sizeof(*h)) return NULL;
    h = malloc(sizeof(*h) + size);
    if (!h) return NULL;
    h->size = size; b->used += size; return h + 1;
}
static void limited_free(void *user, void *ptr)
{
    if (ptr) { budget *b = user; allocation_header *h = (allocation_header *)ptr - 1;
        b->used -= h->size; free(h); }
}
static void *limited_realloc(void *user, void *ptr, size_t size)
{
    budget *b = user;
    allocation_header *h, *next;
    size_t old;
    if (!ptr) return limited_malloc(user, size);
    if (!size) { limited_free(user, ptr); return NULL; }
    h = (allocation_header *)ptr - 1; old = h->size;
    if (size > HEAP_LIMIT - (b->used - old) || size > SIZE_MAX - sizeof(*h)) return NULL;
    next = realloc(h, sizeof(*h) + size);
    if (!next) return NULL;
    next->size = size; b->used = b->used - old + size; return next + 1;
}

typedef struct {size_t tokens,decoded,font_preparations,events,commands,items,callbacks,contexts,clip_ids;} work_budget;
typedef struct {int color_kind,depth;float values[4];size_t offset;} text_context;
typedef struct {fz_device super;FILE *out;size_t bytes,items,callbacks,events,commands,cleanup_pops;int depth,recording,path_first;size_t clips[1024];work_budget *work;text_context *contexts;size_t context_count,context_capacity;size_t *context_clips;size_t context_clip_count,context_clip_capacity;} trace_device;
static void emit(fz_context *ctx,trace_device *d,const char *format,...)
{
 char buffer[1024];int n;va_list args;va_start(args,format);n=vsnprintf(buffer,sizeof(buffer),format,args);va_end(args);
 if(n<0||(size_t)n>=sizeof(buffer)||(size_t)n>OUTPUT_LIMIT-d->bytes)fz_throw(ctx,FZ_ERROR_LIMIT,"trace output limit");
 if(fwrite(buffer,1,(size_t)n,d->out)!=(size_t)n)fz_throw(ctx,FZ_ERROR_SYSTEM,"trace I/O");d->bytes+=(size_t)n;
}
static void number(fz_context *ctx,trace_device *d,float v)
{if(!isfinite(v))fz_throw(ctx,FZ_ERROR_FORMAT,"nonfinite native placement");emit(ctx,d,"%.9g",(double)v);}
static void matrix(fz_context *ctx,trace_device *d,fz_matrix m)
{float v[]={m.a,m.b,m.c,m.d,m.e,m.f};int i;emit(ctx,d,"[");for(i=0;i<6;i++){if(i)emit(ctx,d,",");number(ctx,d,v[i]);}emit(ctx,d,"]");}
static void command(fz_context *ctx, trace_device *d, const char *name, const float *values, int count)
{
    int i;
    if (++d->commands > 1000000 || ++d->work->commands > 4000000) fz_throw(ctx,FZ_ERROR_LIMIT,"path command limit");
    emit(ctx,d,"%s[\"%s\"",d->path_first?"":",",name);d->path_first=0;
    for(i=0;i<count;i++){emit(ctx,d,",");number(ctx,d,values[i]);}emit(ctx,d,"]");
}
static void moveto(fz_context *ctx,void *p,float x,float y){float v[]={x,y};command(ctx,p,"m",v,2);}
static void lineto(fz_context *ctx,void *p,float x,float y){float v[]={x,y};command(ctx,p,"l",v,2);}
static void curveto(fz_context *ctx,void *p,float a,float b,float c,float d,float x,float y){float v[]={a,b,c,d,x,y};command(ctx,p,"c",v,6);}
static void closepath(fz_context *ctx,void *p){command(ctx,p,"h",NULL,0);}
static void event_start(fz_context *ctx,trace_device *d,const char *kind)
{
    if(++d->events>300000 || ++d->work->events>4000000)fz_throw(ctx,FZ_ERROR_LIMIT,"trace event limit");
    emit(ctx,d,"%s{\"kind\":\"%s\",\"clip_depth\":%d,\"clip_ids\":[",d->events==1?"":",",kind,d->depth);
    {int i;for(i=0;i<d->depth;i++){if(i)emit(ctx,d,",");emit(ctx,d,"%zu",d->clips[i]);}}emit(ctx,d,"]");
}
static void path_event(fz_context *ctx,trace_device *d,const char *kind,const fz_path *path,fz_matrix m)
{
    static const fz_path_walker walker={moveto,lineto,curveto,closepath,NULL,NULL,NULL,NULL};
    float values[]={m.a,m.b,m.c,m.d,m.e,m.f};int i;
    event_start(ctx,d,kind);emit(ctx,d,",\"path\":[");d->path_first=1;fz_walk_path(ctx,path,&walker,d);emit(ctx,d,"],\"matrix\":[");
    for(i=0;i<6;i++){if(i)emit(ctx,d,",");number(ctx,d,values[i]);}emit(ctx,d,"]");
}
static void color(fz_context *ctx,trace_device *d,fz_colorspace *cs,const float *values,float alpha)
{
    const char *name;int i,n;
    if(alpha!=1)fz_throw(ctx,FZ_ERROR_FORMAT,"unexpected alpha");
    if(cs==fz_device_gray(ctx))name="gray";else if(cs==fz_device_rgb(ctx))name="rgb";else if(cs==fz_device_cmyk(ctx))name="cmyk";else fz_throw(ctx,FZ_ERROR_FORMAT,"unexpected colorspace");
    n=fz_colorspace_n(ctx,cs);emit(ctx,d,",\"color\":{\"space\":\"%s\",\"values\":[",name);
    for(i=0;i<n;i++){if(i)emit(ctx,d,",");number(ctx,d,values[i]);}emit(ctx,d,"]}");
}
static void fill(fz_context *ctx,fz_device *dev,const fz_path *p,int even,fz_matrix m,fz_colorspace *cs,const float *v,float alpha,fz_color_params params)
{
    trace_device *d=(trace_device*)dev;(void)params;
    path_event(ctx,d,"fill",p,m);emit(ctx,d,",\"rule\":\"%s\"",even?"evenodd":"nonzero");color(ctx,d,cs,v,alpha);emit(ctx,d,"}");
}
static void stroke(fz_context *ctx,fz_device *dev,const fz_path *p,const fz_stroke_state *s,fz_matrix m,fz_colorspace *cs,const float *v,float alpha,fz_color_params params)
{
    trace_device *d=(trace_device*)dev;int i;(void)params;
    if(s->dash_len<0||s->dash_len>256)fz_throw(ctx,FZ_ERROR_LIMIT,"dash limit");
    path_event(ctx,d,"stroke",p,m);color(ctx,d,cs,v,alpha);emit(ctx,d,",\"style\":{\"width\":");number(ctx,d,s->linewidth);
    emit(ctx,d,",\"start_cap\":%d,\"dash_cap\":%d,\"end_cap\":%d,\"join\":%d,\"miter\":",s->start_cap,s->dash_cap,s->end_cap,s->linejoin);number(ctx,d,s->miterlimit);
    emit(ctx,d,",\"phase\":");number(ctx,d,s->dash_phase);emit(ctx,d,",\"dash\":[");for(i=0;i<s->dash_len;i++){if(i)emit(ctx,d,",");number(ctx,d,s->dash_list[i]);}emit(ctx,d,"]}}");
}
static void clip(fz_context *ctx,fz_device *dev,const fz_path *p,int even,fz_matrix m,fz_rect scissor)
{
    trace_device *d=(trace_device*)dev;(void)scissor;
    if(d->depth>=1024)fz_throw(ctx,FZ_ERROR_LIMIT,"clip depth limit");path_event(ctx,d,"clip",p,m);emit(ctx,d,",\"rule\":\"%s\"}",even?"evenodd":"nonzero");d->clips[d->depth++]=d->events-1;
}
static void pop(fz_context *ctx,fz_device *dev)
{
    trace_device *d=(trace_device*)dev;if(!d->recording){if(d->depth<=0)fz_throw(ctx,FZ_ERROR_FORMAT,"cleanup clip underflow");d->depth--;d->cleanup_pops++;return;}
    if(d->depth<=0)fz_throw(ctx,FZ_ERROR_FORMAT,"native clip stack underflow");
    d->depth--;event_start(ctx,d,"pop");emit(ctx,d,"}");
}
static size_t context(fz_context *ctx,trace_device *d,fz_colorspace *cs,const float *values,float alpha)
{
 int kind=0,n=0;size_t i; text_context item;
 if(cs){if(alpha!=1)fz_throw(ctx,FZ_ERROR_FORMAT,"text alpha");if(cs==fz_device_gray(ctx)){kind=1;n=1;}else if(cs==fz_device_rgb(ctx)){kind=2;n=3;}else if(cs==fz_device_cmyk(ctx)){kind=3;n=4;}else fz_throw(ctx,FZ_ERROR_FORMAT,"text colorspace");}
 memset(&item,0,sizeof(item));item.color_kind=kind;item.depth=d->depth;for(i=0;i<(size_t)n;i++){if(!isfinite(values[i]))fz_throw(ctx,FZ_ERROR_FORMAT,"text color finite");item.values[i]=values[i];}
 if(d->context_count){text_context *old=&d->contexts[d->context_count-1];if(old->color_kind==kind&&old->depth==d->depth&&!memcmp(old->values,item.values,sizeof(item.values))&&(!d->depth||!memcmp(d->context_clips+old->offset,d->clips,(size_t)d->depth*sizeof(size_t))))return d->context_count-1;}
 if(++d->work->contexts>1000000||(size_t)d->depth>4000000-d->work->clip_ids)fz_throw(ctx,FZ_ERROR_LIMIT,"context work limit");d->work->clip_ids+=(size_t)d->depth;
 if(d->context_count==d->context_capacity){size_t cap=d->context_capacity?d->context_capacity*2:16;if(cap>131072)fz_throw(ctx,FZ_ERROR_LIMIT,"page context count");d->contexts=fz_realloc_array(ctx,d->contexts,cap,text_context);d->context_capacity=cap;}
 if((size_t)d->depth>d->context_clip_capacity-d->context_clip_count){size_t need=d->context_clip_count+(size_t)d->depth,cap=d->context_clip_capacity?d->context_clip_capacity:16;while(cap<need){if(cap>2097152)fz_throw(ctx,FZ_ERROR_LIMIT,"page context clips");cap*=2;}d->context_clips=fz_realloc_array(ctx,d->context_clips,cap,size_t);d->context_clip_capacity=cap;}
 item.offset=d->context_clip_count;if(d->depth)memcpy(d->context_clips+d->context_clip_count,d->clips,(size_t)d->depth*sizeof(size_t));d->context_clip_count+=(size_t)d->depth;d->contexts[d->context_count]=item;return d->context_count++;
}
static void compact_code(fz_context *ctx,trace_device *d,int cid,const int *unicode,size_t count,fz_matrix placement,float advance,int mode,size_t context_id,size_t callback)
{
 size_t i;if(++d->events>300000||++d->work->events>4000000)fz_throw(ctx,FZ_ERROR_LIMIT,"event count limit");
 emit(ctx,d,"%s[\"code\",%d,[",d->events==1?"":",",cid);for(i=0;i<count;i++){if(i)emit(ctx,d,",");emit(ctx,d,"%d",unicode[i]);}emit(ctx,d,"],");matrix(ctx,d,placement);emit(ctx,d,",");number(ctx,d,advance);emit(ctx,d,",%d,%zu,%zu]",mode,context_id,callback);
}
static void text_event(fz_context *ctx,trace_device *d,const fz_text *text,fz_matrix ctm,int mode,fz_colorspace *cs,const float *values,float alpha)
{
 const fz_text_span *span;size_t callback=d->callbacks++,context_id=context(ctx,d,cs,values,alpha),count=0;int cid=-1,unicode[256];fz_matrix placement=fz_identity;float advance=0;
 if(d->callbacks>100000||++d->work->callbacks>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"callback limit");
 for(span=text->head;span;span=span->next){int i;if(span->wmode)fz_throw(ctx,FZ_ERROR_ARGUMENT,"vertical native text");if(span->len<0||span->len>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"span limit");
  for(i=0;i<span->len;i++){const fz_text_item *item=&span->items[i];int primary=item->gid>=0&&item->cid>=0;if(++d->items>1000000||++d->work->items>4000000)fz_throw(ctx,FZ_ERROR_LIMIT,"item limit");
   if(primary){fz_matrix m=span->trm;if(cid>=0)compact_code(ctx,d,cid,unicode,count,placement,advance,mode,context_id,callback);if(item->cid>255)fz_throw(ctx,FZ_ERROR_ARGUMENT,"non-byte source code");cid=item->cid;count=0;m.e=item->x;m.f=item->y;placement=fz_concat(m,ctm);advance=item->adv;}
   else if(cid<0)fz_throw(ctx,FZ_ERROR_FORMAT,"orphan Unicode continuation");
   if(count>=256)fz_throw(ctx,FZ_ERROR_LIMIT,"Unicode continuation limit");if(item->ucs<0||item->ucs>0x10ffff||(item->ucs>=0xd800&&item->ucs<=0xdfff))fz_throw(ctx,FZ_ERROR_ARGUMENT,"native undefined Unicode");unicode[count++]=item->ucs;
  }
 }
 if(cid>=0)compact_code(ctx,d,cid,unicode,count,placement,advance,mode,context_id,callback);
}
static void emit_contexts(fz_context *ctx,trace_device *d)
{
 size_t i,j;emit(ctx,d,"],\"contexts\":[");for(i=0;i<d->context_count;i++){text_context *c=&d->contexts[i];if(i)emit(ctx,d,",");emit(ctx,d,"{\"clip_ids\":[");for(j=0;j<(size_t)c->depth;j++){if(j)emit(ctx,d,",");emit(ctx,d,"%zu",d->context_clips[c->offset+j]);}emit(ctx,d,"],\"color_observable\":%s",c->color_kind?"true":"false");if(c->color_kind)color(ctx,d,c->color_kind==1?fz_device_gray(ctx):c->color_kind==2?fz_device_rgb(ctx):fz_device_cmyk(ctx),c->values,1);else emit(ctx,d,",\"color\":null");emit(ctx,d,"}");}emit(ctx,d,"]");
}
static void drop_trace(fz_context *ctx,fz_device *dev){trace_device *d=(trace_device*)dev;fz_free(ctx,d->contexts);fz_free(ctx,d->context_clips);}
/* A native ignore_text callback carries no paint color. */
static void fill_text(fz_context *ctx,fz_device *dev,const fz_text *text,fz_matrix ctm,fz_colorspace *cs,const float *color,float alpha,fz_color_params params)
{(void)params;text_event(ctx,(trace_device*)dev,text,ctm,0,cs,color,alpha);}
static void ignore_text(fz_context *ctx,fz_device *dev,const fz_text *text,fz_matrix ctm)
{text_event(ctx,(trace_device*)dev,text,ctm,3,NULL,NULL,1);}
static void expected_error(void *user,const char *message){(void)user;(void)message;}
static void diagnostic(void *user,const char *message){int *n=user;(*n)++;fprintf(stderr,"native diagnostic: %.500s\n",message);}
/* Returning NULL deliberately bypasses OS fonts and selects MuPDF's bundled
 * Base14 bytes. No filesystem or platform font lookup occurs in this callback. */
static fz_font *no_system_font(fz_context *ctx,const char *name,int bold,int italic,int exact)
{(void)ctx;(void)name;(void)bold;(void)italic;(void)exact;return NULL;}
static fz_font *no_cjk_font(fz_context *ctx,const char *name,int ordering,int serif)
{(void)name;(void)ordering;(void)serif;fz_throw(ctx,FZ_ERROR_FORMAT,"CJK/system fallback forbidden");}
static fz_font *no_fallback_font(fz_context *ctx,int script,int language,int serif,int bold,int italic)
{(void)script;(void)language;(void)serif;(void)bold;(void)italic;fz_throw(ctx,FZ_ERROR_FORMAT,"system fallback forbidden");}
static int named(fz_context *ctx,pdf_obj *obj,const char *name){return pdf_is_name(ctx,obj)&&!strcmp(pdf_to_name(ctx,obj),name);}
static int absent(fz_context *ctx,pdf_obj *obj){return !obj||pdf_is_null(ctx,obj);}
static void keys(fz_context *ctx,pdf_obj *obj,const char *const *allowed,size_t count)
{int i,n=pdf_dict_len(ctx,obj);if(!pdf_is_dict(ctx,obj)||n>32)fz_throw(ctx,FZ_ERROR_ARGUMENT,"dictionary shape limit");for(i=0;i<n;i++){const char *key=pdf_to_name(ctx,pdf_dict_get_key(ctx,obj,i));size_t j;for(j=0;j<count;j++)if(!strcmp(key,allowed[j]))break;if(j==count)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unsupported dictionary key %s",key);}}
#define KEYS(ctx,obj,list) keys(ctx,obj,list,sizeof(list)/sizeof(list[0]))
static void lexical(fz_context *ctx,const unsigned char *bytes,size_t length,int cmap,pdf_obj *fonts,pdf_obj *used,work_budget *work)
{
 static const char *const words[]={"q","Q","cm","BT","ET","Tc","Tw","Tz","TL","Tf","Tr","Ts","Td","TD","Tm","T*","Tj","TJ","'","\"","w","J","j","M","d","m","l","c","v","y","h","re","S","s","f","F","f*","B","B*","b","b*","n","W","W*","G","g","RG","rg","K","k"};
 fz_stream *stream=NULL;pdf_lexbuf lex;size_t tokens=0;int array=0,inside=0,saves=0,active_path=0,pending_clip=0;pdf_token previous=PDF_TOK_EOF;int64_t integer=0;char font_name[128]={0};int has_name=0;
 pdf_lexbuf_init(ctx,&lex,PDF_LEXBUF_SMALL);fz_var(stream);
 fz_try(ctx){stream=fz_open_memory(ctx,bytes,length);for(;;){pdf_token t;size_t j;lex.len=0;t=pdf_lex(ctx,stream,&lex);if(t==PDF_TOK_EOF)break;if(++tokens>1000000||++work->tokens>8000000)fz_throw(ctx,FZ_ERROR_LIMIT,"token limit");
  if(cmap){if(t==PDF_TOK_KEYWORD&&!strcmp(lex.scratch,"usecmap"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"usecmap forbidden");continue;}
  if(t==PDF_TOK_INT){integer=lex.i;previous=t;continue;}
  if(t==PDF_TOK_REAL){if(!isfinite(lex.f))fz_throw(ctx,FZ_ERROR_ARGUMENT,"nonfinite operand");previous=t;continue;}
  if(t==PDF_TOK_NAME&&!array){size_t n=strlen(lex.scratch);if(n>=sizeof(font_name))fz_throw(ctx,FZ_ERROR_LIMIT,"font resource name");memcpy(font_name,lex.scratch,n+1);has_name=1;}
  if(t==PDF_TOK_NAME||t==PDF_TOK_STRING){if(lex.len>1024*1024)fz_throw(ctx,FZ_ERROR_LIMIT,"operand length");previous=t;continue;}
  if(t==PDF_TOK_OPEN_ARRAY){if(array++)fz_throw(ctx,FZ_ERROR_ARGUMENT,"nested TJ array");previous=t;continue;}
  if(t==PDF_TOK_CLOSE_ARRAY){if(--array!=0)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unmatched array");previous=t;continue;}
  if(t!=PDF_TOK_KEYWORD||array)fz_throw(ctx,FZ_ERROR_ARGUMENT,"outside C0 lexical subset");
  for(j=0;j<sizeof(words)/sizeof(words[0]);j++)if(!strcmp(lex.scratch,words[j]))break;if(j==sizeof(words)/sizeof(words[0]))fz_throw(ctx,FZ_ERROR_ARGUMENT,"operator outside C0");
  if(!strcmp(lex.scratch,"BT")){if(inside||active_path||pending_clip)fz_throw(ctx,FZ_ERROR_ARGUMENT,"BT with active text/path/clip");inside=1;}
  else if(!strcmp(lex.scratch,"ET")){if(!inside)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unmatched ET");inside=0;}
  else if(!strcmp(lex.scratch,"q")||!strcmp(lex.scratch,"Q")||!strcmp(lex.scratch,"cm")){
   if(inside)fz_throw(ctx,FZ_ERROR_ARGUMENT,"special graphics state inside BT");
   if(!strcmp(lex.scratch,"q")){if(++saves>128)fz_throw(ctx,FZ_ERROR_LIMIT,"save depth");}else if(!strcmp(lex.scratch,"Q")&&--saves<0)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unmatched Q");
  }else if(!strcmp(lex.scratch,"Tr")){if(previous!=PDF_TOK_INT||(integer!=0&&integer!=3))fz_throw(ctx,FZ_ERROR_ARGUMENT,"text rendering mode outside C0");}
  else if((!strcmp(lex.scratch,"Tm")||!strcmp(lex.scratch,"Td")||!strcmp(lex.scratch,"TD")||!strcmp(lex.scratch,"T*")||!strcmp(lex.scratch,"Tj")||!strcmp(lex.scratch,"TJ")||!strcmp(lex.scratch,"'")||!strcmp(lex.scratch,"\""))&&!inside)fz_throw(ctx,FZ_ERROR_ARGUMENT,"text operation outside BT");
  if(!strcmp(lex.scratch,"m")||!strcmp(lex.scratch,"l")||!strcmp(lex.scratch,"c")||!strcmp(lex.scratch,"v")||!strcmp(lex.scratch,"y")||!strcmp(lex.scratch,"h")||!strcmp(lex.scratch,"re")) {if(inside)fz_throw(ctx,FZ_ERROR_ARGUMENT,"path inside BT");active_path=1;}
  else if(!strcmp(lex.scratch,"W")||!strcmp(lex.scratch,"W*")){if(inside)fz_throw(ctx,FZ_ERROR_ARGUMENT,"clip inside BT");pending_clip=1;}
  else if(!strcmp(lex.scratch,"S")||!strcmp(lex.scratch,"s")||!strcmp(lex.scratch,"f")||!strcmp(lex.scratch,"F")||!strcmp(lex.scratch,"f*")||!strcmp(lex.scratch,"B")||!strcmp(lex.scratch,"B*")||!strcmp(lex.scratch,"b")||!strcmp(lex.scratch,"b*")||!strcmp(lex.scratch,"n")){if(inside)fz_throw(ctx,FZ_ERROR_ARGUMENT,"paint inside BT");active_path=0;pending_clip=0;}
  if(!strcmp(lex.scratch,"cm")&&active_path)fz_throw(ctx,FZ_ERROR_ARGUMENT,"matrix change with active path");
  if(!strcmp(lex.scratch,"Tf")&&(!has_name||absent(ctx,pdf_dict_gets(ctx,fonts,font_name))))fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing explicit font binding");
  if(!strcmp(lex.scratch,"Tf")){if(pdf_dict_len(ctx,used)>=256&&!pdf_dict_gets(ctx,used,font_name))fz_throw(ctx,FZ_ERROR_LIMIT,"used font bindings");pdf_dict_puts(ctx,used,font_name,pdf_dict_gets(ctx,fonts,font_name));}
  has_name=0;previous=t;
 }if(!cmap&&(array||inside||saves||active_path||pending_clip))fz_throw(ctx,FZ_ERROR_ARGUMENT,"unclosed text/graphics state");}
 fz_always(ctx){fz_drop_stream(ctx,stream);pdf_lexbuf_fin(ctx,&lex);}fz_catch(ctx){fz_rethrow(ctx);}
}
static void stream_shape(fz_context *ctx,pdf_obj *obj)
{
 pdf_obj *filter;int i,n;if(!pdf_is_stream(ctx,obj))fz_throw(ctx,FZ_ERROR_ARGUMENT,"expected stream");if(!absent(ctx,pdf_dict_gets(ctx,obj,"F")))fz_throw(ctx,FZ_ERROR_ARGUMENT,"external stream forbidden");
 filter=pdf_dict_get(ctx,obj,PDF_NAME(Filter));n=pdf_is_array(ctx,filter)?pdf_array_len(ctx,filter):absent(ctx,filter)?0:1;if(n>16)fz_throw(ctx,FZ_ERROR_LIMIT,"filter count");for(i=0;i<n;i++){pdf_obj *f=pdf_is_array(ctx,filter)?pdf_array_get(ctx,filter,i):filter;if(!named(ctx,f,"FlateDecode")&&!named(ctx,f,"Fl")&&!named(ctx,f,"LZWDecode")&&!named(ctx,f,"LZW")&&!named(ctx,f,"ASCIIHexDecode")&&!named(ctx,f,"AHx")&&!named(ctx,f,"ASCII85Decode")&&!named(ctx,f,"A85")&&!named(ctx,f,"RunLengthDecode")&&!named(ctx,f,"RL"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"stream filter outside PP1");}
}
static fz_buffer *read_bounded(fz_context *ctx,fz_stream *stream,work_budget *work)
{
 fz_buffer *buffer=NULL;unsigned char chunk[8192];size_t length=0;fz_var(buffer);fz_try(ctx){buffer=fz_new_buffer(ctx,4096);for(;;){size_t n=fz_read(ctx,stream,chunk,sizeof(chunk));if(n>PAGE_CONTENT_LIMIT-length||n>TOTAL_DECODED_LIMIT-work->decoded)fz_throw(ctx,FZ_ERROR_LIMIT,"decoded stream budget");length+=n;work->decoded+=n;fz_append_data(ctx,buffer,chunk,n);if(n<sizeof(chunk))break;}if(stream->error)fz_throw(ctx,FZ_ERROR_FORMAT,"stream decode error");}fz_catch(ctx){fz_drop_buffer(ctx,buffer);fz_rethrow(ctx);}return buffer;
}
static void encoding(fz_context *ctx,pdf_obj *obj)
{
 static const char *const allowed[]={"Type","BaseEncoding","Differences"};pdf_obj *base,*diff;int i,index=-1,n;
 if(pdf_is_name(ctx,obj)){if(!named(ctx,obj,"WinAnsiEncoding")&&!named(ctx,obj,"MacRomanEncoding")&&!named(ctx,obj,"StandardEncoding")&&!named(ctx,obj,"MacExpertEncoding"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"unsupported encoding");return;}
 KEYS(ctx,obj,allowed);base=pdf_dict_get(ctx,obj,PDF_NAME(BaseEncoding));if(!pdf_is_name(ctx,base))fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing base encoding");encoding(ctx,base);diff=pdf_dict_get(ctx,obj,PDF_NAME(Differences));if(absent(ctx,diff))return;
 if(!pdf_is_array(ctx,diff)||(n=pdf_array_len(ctx,diff))>4096)fz_throw(ctx,FZ_ERROR_LIMIT,"Differences limit");for(i=0;i<n;i++){pdf_obj *v=pdf_array_get(ctx,diff,i);if(pdf_is_int(ctx,v)){int64_t x=pdf_to_int64(ctx,v);if(x<0||x>255)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Differences index");index=(int)x;}else if(pdf_is_name(ctx,v)){if(index<0||index>255||strlen(pdf_to_name(ctx,v))>127)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Differences name/index");index++;}else fz_throw(ctx,FZ_ERROR_ARGUMENT,"Differences type");}
}
static void resources(fz_context *ctx,pdf_obj *fonts,work_budget *work)
{
 int i,n=pdf_dict_len(ctx,fonts);if(!pdf_is_dict(ctx,fonts)||n>256)fz_throw(ctx,FZ_ERROR_LIMIT,"font bindings");
 for(i=0;i<n;i++){pdf_obj *font=pdf_dict_get_val(ctx,fonts,i),*widths,*first,*last,*desc,*tu,*enc;int j,count,embedded=0;int64_t low,high;
  if(++work->font_preparations>100000)fz_throw(ctx,FZ_ERROR_LIMIT,"font preparation work");if(!pdf_is_dict(ctx,font)||pdf_dict_len(ctx,font)>64)fz_throw(ctx,FZ_ERROR_ARGUMENT,"font dictionary shape");
  if(!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(Subtype)),"Type1")&&!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(Subtype)),"TrueType"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"font subtype outside PP1");
  desc=pdf_dict_get(ctx,font,PDF_NAME(FontDescriptor));if(!absent(ctx,desc)){if(!pdf_is_dict(ctx,desc)||pdf_dict_len(ctx,desc)>64)fz_throw(ctx,FZ_ERROR_ARGUMENT,"descriptor shape");for(j=0;j<3;j++){const char *key=j==0?"FontFile":j==1?"FontFile2":"FontFile3";pdf_obj *program=pdf_dict_gets(ctx,desc,key);if(!absent(ctx,program)){fz_stream *stream=NULL;fz_buffer *buffer=NULL;embedded++;stream_shape(ctx,program);fz_var(stream);fz_var(buffer);fz_try(ctx){stream=pdf_open_stream(ctx,program);buffer=read_bounded(ctx,stream,work);}fz_always(ctx){fz_drop_stream(ctx,stream);fz_drop_buffer(ctx,buffer);}fz_catch(ctx){fz_rethrow(ctx);}}}if(embedded>1)fz_throw(ctx,FZ_ERROR_ARGUMENT,"multiple embedded font programs");}
  widths=pdf_dict_get(ctx,font,PDF_NAME(Widths));first=pdf_dict_get(ctx,font,PDF_NAME(FirstChar));last=pdf_dict_get(ctx,font,PDF_NAME(LastChar));
  if(!absent(ctx,widths)){low=pdf_to_int64(ctx,first);high=pdf_to_int64(ctx,last);if(!pdf_is_int(ctx,first)||!pdf_is_int(ctx,last)||low<0||high>255||low>high||!pdf_is_array(ctx,widths)||(count=pdf_array_len(ctx,widths))!=high-low+1)fz_throw(ctx,FZ_ERROR_ARGUMENT,"width declaration");for(j=0;j<count;j++){pdf_obj *v=pdf_array_get(ctx,widths,j);if(!pdf_is_number(ctx,v)||!isfinite(pdf_to_real(ctx,v)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"width value");}}
  else {if(!absent(ctx,first)||!absent(ctx,last))fz_throw(ctx,FZ_ERROR_ARGUMENT,"incomplete width declaration");if(embedded||!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(Subtype)),"Type1")||(!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(BaseFont)),"Helvetica")&&!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(BaseFont)),"Times-Roman")))fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing explicit or pinned advance source");}
  if(!embedded&&(!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(Subtype)),"Type1")||(!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(BaseFont)),"Helvetica")&&!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(BaseFont)),"Times-Roman"))))fz_throw(ctx,FZ_ERROR_ARGUMENT,"native has no allowed embedded or pinned font");
  tu=pdf_dict_get(ctx,font,PDF_NAME(ToUnicode));enc=pdf_dict_get(ctx,font,PDF_NAME(Encoding));if(!absent(ctx,enc))encoding(ctx,enc);if(absent(ctx,tu)&&absent(ctx,enc))fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing explicit Unicode source");if(absent(ctx,widths)&&absent(ctx,enc))fz_throw(ctx,FZ_ERROR_ARGUMENT,"pinned metrics require explicit encoding");
  if(!absent(ctx,tu)){fz_stream *stream=NULL;fz_buffer *buffer=NULL;unsigned char *data;size_t length;stream_shape(ctx,tu);fz_var(stream);fz_var(buffer);fz_try(ctx){stream=pdf_open_stream(ctx,tu);buffer=read_bounded(ctx,stream,work);length=fz_buffer_storage(ctx,buffer,&data);lexical(ctx,data,length,1,NULL,NULL,work);}fz_always(ctx){fz_drop_stream(ctx,stream);fz_drop_buffer(ctx,buffer);}fz_catch(ctx){fz_rethrow(ctx);}}
 }
}
static void quote(fz_context *ctx,trace_device *d,const char *value)
{size_t i;emit(ctx,d,"\"");for(i=0;value[i]&&i<400;i++){unsigned char c=(unsigned char)value[i];if(c=='\"'||c=='\\')emit(ctx,d,"\\%c",c);else if(c<32||c>126)emit(ctx,d,"\\u%04x",c);else emit(ctx,d,"%c",c);}emit(ctx,d,"\"");}
static void page_trace(fz_context *ctx,pdf_document *doc,int index,trace_device *d,work_budget *work)
{
 pdf_page *page=NULL;pdf_obj *used=NULL;fz_buffer *content=NULL;fz_stream *stream=NULL;pdf_processor *proc=NULL;fz_var(page);fz_var(used);fz_var(content);fz_var(stream);fz_var(proc);
 fz_try(ctx){pdf_obj *res,*fonts,*contents,*cs;fz_rect crop,bounds;fz_matrix transform;unsigned char *bytes;size_t length;int i,n;
  page=pdf_load_page(ctx,doc,index);res=pdf_page_resources(ctx,page);fonts=pdf_dict_get(ctx,res,PDF_NAME(Font));contents=pdf_page_contents(ctx,page);
  if(!absent(ctx,pdf_dict_get(ctx,page->obj,PDF_NAME(Group))))fz_throw(ctx,FZ_ERROR_ARGUMENT,"page transparency Group outside PP1");cs=pdf_dict_get(ctx,res,PDF_NAME(ColorSpace));if(!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultGray"))||!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultRGB"))||!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultCMYK")))fz_throw(ctx,FZ_ERROR_ARGUMENT,"default device color override outside PP1");
  if(!absent(ctx,contents)){n=pdf_is_array(ctx,contents)?pdf_array_len(ctx,contents):1;if(n>10000)fz_throw(ctx,FZ_ERROR_LIMIT,"content stream occurrences");for(i=0;i<n;i++)stream_shape(ctx,pdf_is_array(ctx,contents)?pdf_array_get(ctx,contents,i):contents);stream=pdf_open_contents_stream(ctx,doc,contents);content=read_bounded(ctx,stream,work);}else content=fz_new_buffer(ctx,0);
  length=fz_buffer_storage(ctx,content,&bytes);used=pdf_new_dict(ctx,doc,4);lexical(ctx,bytes,length,0,fonts,used,work);resources(ctx,used,work);
  pdf_page_transform(ctx,page,&crop,&transform);bounds=fz_transform_rect(crop,transform);if(!isfinite(bounds.x0)||!isfinite(bounds.y0)||!isfinite(bounds.x1)||!isfinite(bounds.y1)||bounds.x0>=bounds.x1||bounds.y0>=bounds.y1)fz_throw(ctx,FZ_ERROR_ARGUMENT,"page geometry");
  emit(ctx,d,"{\"page\":%d,\"status\":\"accepted\",\"advance_policy\":\"allow_pinned_base14\",\"initial_ctm\":",index+1);matrix(ctx,d,transform);emit(ctx,d,",\"initial_clip\":[");number(ctx,d,bounds.x0);emit(ctx,d,",");number(ctx,d,bounds.y0);emit(ctx,d,",");number(ctx,d,bounds.x1);emit(ctx,d,",");number(ctx,d,bounds.y1);emit(ctx,d,"],\"events\":[");
  proc=pdf_new_run_processor(ctx,doc,&d->super,transform,-1,"View",NULL,NULL,NULL,NULL,NULL);pdf_process_glyph(ctx,proc,doc,res,content);d->recording=0;pdf_close_processor(ctx,proc);fz_close_device(ctx,&d->super);emit_contexts(ctx,d);emit(ctx,d,",\"text_callbacks\":%zu,\"cleanup_pops\":%zu}",d->callbacks,d->cleanup_pops);
 }fz_always(ctx){pdf_drop_processor(ctx,proc);fz_drop_stream(ctx,stream);fz_drop_buffer(ctx,content);pdf_drop_obj(ctx,used);fz_drop_page(ctx,(fz_page*)page);}fz_catch(ctx){fz_rethrow(ctx);}
}
int main(int argc,char **argv)
{
 unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length;int status=0,diagnostics=0,selection=0,all=0;FILE *output=tmpfile();budget heap={0};work_budget work={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
 fz_context *ctx;fz_stream *raw=NULL;pdf_document *doc=NULL;trace_device envelope={0};struct rlimit cpu={30,30};size_t arg;
 setrlimit(RLIMIT_CPU,&cpu);alarm(45);if(argc!=2||!bytes||!output){free(bytes);if(output)fclose(output);return 1;}all=!strcmp(argv[1],"all");if(!all){if(!*argv[1])return 1;for(arg=0;argv[1][arg];arg++){if(argv[1][arg]<'0'||argv[1][arg]>'9'||selection>10000)return 1;selection=selection*10+argv[1][arg]-'0';}if(selection<1||selection>10000)return 1;}
 length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT){free(bytes);fclose(output);return 1;}ctx=fz_new_context(&allocator,NULL,0);if(!ctx){free(bytes);fclose(output);return 1;}fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,expected_error,NULL);fz_install_load_system_font_funcs(ctx,no_system_font,no_cjk_font,no_fallback_font);envelope.out=output;
 fz_var(raw);fz_var(doc);fz_var(status);fz_var(diagnostics);
 fz_try(ctx){int count,index,first,last,pages=0;raw=fz_open_memory(ctx,bytes,length);doc=pdf_open_document_with_stream(ctx,raw);pdf_disable_js(ctx,doc);if(pdf_needs_password(ctx,doc)||!absent(ctx,pdf_dict_get(ctx,pdf_trailer(ctx,doc),PDF_NAME(Encrypt))))fz_throw(ctx,FZ_ERROR_FORMAT,"encryption forbidden");count=pdf_count_pages(ctx,doc);if(count>10000||selection>count)fz_throw(ctx,FZ_ERROR_LIMIT,"page count/index");first=all?0:selection-1;last=all?count:selection;emit(ctx,&envelope,"{\"pages\":[");
  for(index=first;index<last;index++){FILE *page_output=tmpfile();trace_device *d=NULL;int accepted=0;char refusal[512]={0};if(!page_output)fz_throw(ctx,FZ_ERROR_SYSTEM,"page temporary file");fz_var(d);fz_var(accepted);fz_var(refusal);
   fz_try(ctx){d=fz_new_derived_device(ctx,trace_device);d->out=page_output;d->work=&work;d->recording=1;d->super.fill_path=fill;d->super.stroke_path=stroke;d->super.clip_path=clip;d->super.pop_clip=pop;d->super.fill_text=fill_text;d->super.ignore_text=ignore_text;d->super.drop_device=drop_trace;
    fz_try(ctx){page_trace(ctx,doc,index,d,&work);accepted=1;}fz_catch(ctx){if(fz_caught(ctx)!=FZ_ERROR_ARGUMENT)fz_rethrow(ctx);snprintf(refusal,sizeof(refusal),"%s",fz_caught_message(ctx));fz_ignore_error(ctx);}
    fz_flush_warnings(ctx);if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair forbidden");if(pages++)emit(ctx,&envelope,",");
    if(accepted){if(d->bytes>OUTPUT_LIMIT-envelope.bytes)fz_throw(ctx,FZ_ERROR_LIMIT,"document output budget");if(fflush(page_output)||fseek(page_output,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"page trace seek");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),page_output);if(n&&fwrite(chunk,1,n,output)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"page trace copy");if(n<sizeof(chunk)){if(ferror(page_output))fz_throw(ctx,FZ_ERROR_SYSTEM,"page trace read");break;}}envelope.bytes+=d->bytes;}
    else {emit(ctx,&envelope,"{\"page\":%d,\"status\":\"refused\",\"error\":",index+1);quote(ctx,&envelope,refusal);emit(ctx,&envelope,"}");}
   }fz_always(ctx){fz_drop_device(ctx,(fz_device*)d);fclose(page_output);}fz_catch(ctx){fz_rethrow(ctx);}
  }
  emit(ctx,&envelope,"],\"repaired\":false,\"work\":{\"tokens\":%zu,\"decoded_bytes\":%zu,\"font_preparations\":%zu,\"items\":%zu,\"commands\":%zu,\"events\":%zu}}\n",work.tokens,work.decoded,work.font_preparations,work.items,work.commands,work.events);
  if(fflush(output)||fseek(output,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace seek I/O");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),output);if(n&&fwrite(chunk,1,n,stdout)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout I/O");if(n<sizeof(chunk)){if(ferror(output))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace read I/O");break;}}if(fflush(stdout))status=1;
 }fz_always(ctx){pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"page-program oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}fz_drop_context(ctx);fclose(output);free(bytes);return status;
}
