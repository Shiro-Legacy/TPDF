/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned MuPDF path trace. Raw P0 content stdin, identity CTM.
 * Caps input4MiB, native heap128MiB, output64MiB, events300k, commands1m,
 * tokens1m, CPU10s/wall15s. No stdout on refusal. Implicit processor cleanup
 * pops are excluded; explicit Q pops are retained. No page/font/raster load.
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
#define INPUT_LIMIT (4u*1024u*1024u)
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
typedef struct {
    fz_device super;
    FILE *out;
    size_t bytes, events, commands;
    int depth, recording, path_first;
} trace_device;
static void emit(fz_context *ctx, trace_device *d, const char *format, ...)
{
    char buffer[1024]; int n; va_list args;
    va_start(args, format); n = vsnprintf(buffer, sizeof(buffer), format, args); va_end(args);
    if (n < 0 || (size_t)n >= sizeof(buffer) || (size_t)n > OUTPUT_LIMIT-d->bytes)
        fz_throw(ctx, FZ_ERROR_LIMIT, "trace output limit");
    if (fwrite(buffer, 1, (size_t)n, d->out) != (size_t)n) fz_throw(ctx,FZ_ERROR_SYSTEM,"trace output I/O");
    d->bytes += (size_t)n;
}
static void number(fz_context *ctx, trace_device *d, float v)
{ if (!isfinite(v)) fz_throw(ctx,FZ_ERROR_FORMAT,"nonfinite native trace"); emit(ctx,d,"%.9g",(double)v); }
static void command(fz_context *ctx, trace_device *d, const char *name, const float *values, int count)
{
    int i;
    if (++d->commands > 1000000) fz_throw(ctx,FZ_ERROR_LIMIT,"path command limit");
    emit(ctx,d,"%s[\"%s\"",d->path_first?"":",",name);d->path_first=0;
    for(i=0;i<count;i++){emit(ctx,d,",");number(ctx,d,values[i]);}emit(ctx,d,"]");
}
static void moveto(fz_context *ctx,void *p,float x,float y){float v[]={x,y};command(ctx,p,"m",v,2);}
static void lineto(fz_context *ctx,void *p,float x,float y){float v[]={x,y};command(ctx,p,"l",v,2);}
static void curveto(fz_context *ctx,void *p,float a,float b,float c,float d,float x,float y){float v[]={a,b,c,d,x,y};command(ctx,p,"c",v,6);}
static void closepath(fz_context *ctx,void *p){command(ctx,p,"h",NULL,0);}
static void event_start(fz_context *ctx,trace_device *d,const char *kind)
{
    if(++d->events>300000)fz_throw(ctx,FZ_ERROR_LIMIT,"trace event limit");
    emit(ctx,d,"%s{\"kind\":\"%s\",\"clip_depth\":%d",d->events==1?"":",",kind,d->depth);
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
    path_event(ctx,d,"clip",p,m);emit(ctx,d,",\"rule\":\"%s\"}",even?"evenodd":"nonzero");d->depth++;
}
static void pop(fz_context *ctx,fz_device *dev)
{
    trace_device *d=(trace_device*)dev;if(!d->recording)return;
    if(d->depth<=0)fz_throw(ctx,FZ_ERROR_FORMAT,"native clip stack underflow");
    d->depth--;event_start(ctx,d,"pop");emit(ctx,d,"}");
}
static void diagnostic(void *user,const char *message)
{ int *count=user;(*count)++;fprintf(stderr,"native diagnostic: %.500s\n",message); }
static void preflight(fz_context *ctx,const unsigned char *bytes,size_t length)
{
    static const char *words[]={"q","Q","cm","w","J","j","M","d","m","l","c","v","y","h","re","S","s","f","F","f*","B","B*","b","b*","n","W","W*","G","g","RG","rg","K","k"};
    fz_stream *stream=NULL;pdf_lexbuf lex;size_t tokens=0;int depth=0;
    pdf_lexbuf_init(ctx,&lex,PDF_LEXBUF_SMALL);fz_var(stream);
    fz_try(ctx){stream=fz_open_memory(ctx,bytes,length);for(;;){pdf_token t;size_t i;int found=0;
        lex.len=0;t=pdf_lex(ctx,stream,&lex);if(t==PDF_TOK_EOF)break;
        if(++tokens>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"token limit");
        if(t==PDF_TOK_INT||t==PDF_TOK_REAL)continue;
        if(t==PDF_TOK_OPEN_ARRAY){if(depth++)fz_throw(ctx,FZ_ERROR_FORMAT,"nested dash array");continue;}
        if(t==PDF_TOK_CLOSE_ARRAY){if(--depth!=0)fz_throw(ctx,FZ_ERROR_FORMAT,"unmatched array");continue;}
        if(t!=PDF_TOK_KEYWORD||depth)fz_throw(ctx,FZ_ERROR_FORMAT,"outside P0 lexical subset");
        for(i=0;i<sizeof(words)/sizeof(words[0]);i++)if(!strcmp(lex.scratch,words[i])){found=1;break;}
        if(!found)fz_throw(ctx,FZ_ERROR_FORMAT,"operator outside P0");
    }if(depth)fz_throw(ctx,FZ_ERROR_FORMAT,"unclosed array");}
    fz_always(ctx){fz_drop_stream(ctx,stream);pdf_lexbuf_fin(ctx,&lex);}fz_catch(ctx){fz_rethrow(ctx);}
}
int main(void)
{
    unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length;int status=0,diagnostics=0;
    FILE *output=tmpfile();budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
    fz_context *ctx;pdf_document *doc=NULL;pdf_processor *proc=NULL;fz_buffer *buffer=NULL;pdf_obj *resources=NULL;trace_device *dev=NULL;
    struct rlimit cpu={10,10};setrlimit(RLIMIT_CPU,&cpu);alarm(15);
    if(!bytes||!output){free(bytes);if(output)fclose(output);return 1;}
    length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT){free(bytes);fclose(output);return 1;}
    ctx=fz_new_context(&allocator,NULL,0);if(!ctx){free(bytes);fclose(output);return 1;}
    fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);
    fz_var(doc);fz_var(proc);fz_var(buffer);fz_var(resources);fz_var(dev);fz_var(status);fz_var(diagnostics);
    fz_try(ctx){
        preflight(ctx,bytes,length);doc=pdf_create_document(ctx);resources=pdf_new_dict(ctx,doc,0);buffer=fz_new_buffer_from_copied_data(ctx,bytes,length);
        dev=fz_new_derived_device(ctx,trace_device);dev->out=output;dev->recording=1;
        dev->super.fill_path=fill;dev->super.stroke_path=stroke;dev->super.clip_path=clip;dev->super.pop_clip=pop;
        emit(ctx,dev,"{\"events\":[");proc=pdf_new_run_processor(ctx,doc,&dev->super,fz_identity,-1,"View",NULL,NULL,NULL,NULL,NULL);
        pdf_process_glyph(ctx,proc,doc,resources,buffer);dev->recording=0;pdf_close_processor(ctx,proc);fz_close_device(ctx,&dev->super);fz_flush_warnings(ctx);
        if(diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native recovery diagnostics");emit(ctx,dev,"]}\n");
        if(fflush(output)||fseek(output,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace seek I/O");
        for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),output);if(n&&fwrite(chunk,1,n,stdout)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout I/O");if(n<sizeof(chunk)){if(ferror(output))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace read I/O");break;}}
        if(fflush(stdout))status=1;
    }fz_always(ctx){pdf_drop_processor(ctx,proc);fz_drop_device(ctx,(fz_device*)dev);pdf_drop_obj(ctx,resources);fz_drop_buffer(ctx,buffer);pdf_drop_document(ctx,doc);}
    fz_catch(ctx){fprintf(stderr,"path oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
    fz_drop_context(ctx);fclose(output);free(bytes);return status;
}
