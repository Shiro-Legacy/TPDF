/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned MuPDF font-free geometry raster oracle. WIDTH HEIGHT AA arguments;
 * raw standalone content stdin; identity CTM, white opaque RGB, AA0 or AA8, P6 PPM.
 * Input4MiB, dimensions1..4096, pixels<=16777216, native heap128MiB,
 * tokens500k, numeric input magnitude1m, CPU30s/wall45s. No partial stdout
 * on decode/render/diagnostic failures. No font or external resource loading.
 */
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
#define PIXEL_LIMIT 16777216u
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
static void preflight(fz_context *ctx,const unsigned char *bytes,size_t length)
{
    static const char *words[]={"q","Q","cm","m","l","c","h","re","f","F","f*","n","W","W*","g","rg"};
    static const int arities[]={0,0,6,2,2,6,0,4,0,0,0,0,0,0,1,3};
    fz_stream *stream=NULL;pdf_lexbuf lex;size_t tokens=0;int depth=0,path=0,pending=0,operands=0;double values[6];
    pdf_lexbuf_init(ctx,&lex,PDF_LEXBUF_SMALL);fz_var(stream);
    fz_try(ctx){stream=fz_open_memory(ctx,bytes,length);for(;;){pdf_token t;size_t i;int found=-1;
        lex.len=0;t=pdf_lex(ctx,stream,&lex);if(t==PDF_TOK_EOF)break;
        if(++tokens>500000)fz_throw(ctx,FZ_ERROR_LIMIT,"token limit");
        if(t==PDF_TOK_INT||t==PDF_TOK_REAL){double v=t==PDF_TOK_INT?(double)lex.i:(double)lex.f;if(!isfinite(v)||fabs(v)>1000000.||operands>=6)fz_throw(ctx,FZ_ERROR_LIMIT,"numeric operand bound");values[operands++]=v;continue;}
        if(t!=PDF_TOK_KEYWORD)fz_throw(ctx,FZ_ERROR_FORMAT,"outside geometry lexical subset");
        for(i=0;i<sizeof(words)/sizeof(words[0]);i++)if(!strcmp(lex.scratch,words[i])){found=(int)i;break;}
        if(found<0)fz_throw(ctx,FZ_ERROR_FORMAT,"operator outside fill geometry");
        if(operands!=arities[found])fz_throw(ctx,FZ_ERROR_FORMAT,"operator operand count");
        if(found==0){if(path||pending||++depth>128)fz_throw(ctx,FZ_ERROR_LIMIT,"graphics save/path state");}
        else if(found==1){if(path||pending||--depth<0)fz_throw(ctx,FZ_ERROR_FORMAT,"graphics restore/path state");}
        else if(found==2){if(path||pending)fz_throw(ctx,FZ_ERROR_FORMAT,"matrix during path");}
        else if(found==3||found==7)path=1;
        else if(found>=4&&found<=6){if(!path)fz_throw(ctx,FZ_ERROR_FORMAT,"path continuation without move");}
        else if(found>=8&&found<=11){path=0;pending=0;}
        else if(found==12||found==13){if(!path||pending)fz_throw(ctx,FZ_ERROR_FORMAT,"invalid pending clip");pending=1;}
        else for(i=0;i<(size_t)operands;i++)if(values[i]<0||values[i]>1)fz_throw(ctx,FZ_ERROR_FORMAT,"color component range");
        operands=0;
    }if(depth||operands||path||pending)fz_throw(ctx,FZ_ERROR_FORMAT,"unfinished geometry state");}
    fz_always(ctx){fz_drop_stream(ctx,stream);pdf_lexbuf_fin(ctx,&lex);}fz_catch(ctx){fz_rethrow(ctx);}
}
static void diagnostic(void *user,const char *message)
{int *count=user;(*count)++;fprintf(stderr,"native diagnostic: %.500s\n",message);}
static int dimension(const char *text)
{
    unsigned value=0;const unsigned char *p=(const unsigned char*)text;
    if(!*p)return 0;
    for(;*p;p++){if(*p<'0'||*p>'9')return 0;value=value*10u+(*p-'0');if(value>4096)return 0;}
    return (int)value;
}
int main(int argc,char **argv)
{
    int width,height,aa,status=0,diagnostics=0;unsigned char *bytes;size_t length;
    budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
    fz_context *ctx;pdf_document *doc=NULL;pdf_processor *proc=NULL;fz_buffer *buffer=NULL;pdf_obj *resources=NULL;fz_device *device=NULL;fz_pixmap *pixels=NULL;
    struct rlimit cpu={30,30};setrlimit(RLIMIT_CPU,&cpu);alarm(45);
    if(argc!=4||(width=dimension(argv[1]))==0||(height=dimension(argv[2]))==0||(size_t)width*(size_t)height>PIXEL_LIMIT){fputs("usage: mupdf-page-raster-oracle WIDTH HEIGHT AA (AA0or8,1..4096,<=16777216pixels)\n",stderr);return 1;}
    if(strcmp(argv[3],"0")&&strcmp(argv[3],"8"))return 1;aa=argv[3][0]-'0';
    bytes=malloc(INPUT_LIMIT+1);if(!bytes)return 1;
    length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT){free(bytes);return 1;}
    ctx=fz_new_context(&allocator,NULL,0);if(!ctx){free(bytes);return 1;}
    fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);
    fz_var(doc);fz_var(proc);fz_var(buffer);fz_var(resources);fz_var(device);fz_var(pixels);fz_var(status);fz_var(diagnostics);
    fz_try(ctx){
        int y,stride;unsigned char *samples;fz_irect bbox={0,0,width,height};
        preflight(ctx,bytes,length);fz_set_aa_level(ctx,aa);
        doc=pdf_create_document(ctx);resources=pdf_new_dict(ctx,doc,0);buffer=fz_new_buffer_from_copied_data(ctx,bytes,length);
        pixels=fz_new_pixmap_with_bbox(ctx,fz_device_rgb(ctx),bbox,NULL,0);fz_clear_pixmap_with_value(ctx,pixels,255);
        device=fz_new_draw_device(ctx,fz_identity,pixels);
        proc=pdf_new_run_processor(ctx,doc,device,fz_identity,-1,"View",NULL,NULL,NULL,NULL,NULL);
        pdf_process_glyph(ctx,proc,doc,resources,buffer);pdf_close_processor(ctx,proc);fz_close_device(ctx,device);fz_flush_warnings(ctx);
        if(diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native recovery diagnostics");
        if(fz_pixmap_components(ctx,pixels)!=3||fz_pixmap_alpha(ctx,pixels)!=0)fz_throw(ctx,FZ_ERROR_FORMAT,"unexpected pixel format");
        samples=fz_pixmap_samples(ctx,pixels);stride=fz_pixmap_stride(ctx,pixels);
        if(stride<width*3)fz_throw(ctx,FZ_ERROR_FORMAT,"unexpected pixel stride");
        /* No stdout is touched before all rendering and diagnostic checks end. */
        if(fprintf(stdout,"P6\n%d %d\n255\n",width,height)<0)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout header I/O");
        for(y=0;y<height;y++)if(fwrite(samples+(size_t)y*(size_t)stride,3,(size_t)width,stdout)!=(size_t)width)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout pixel I/O");
        if(fflush(stdout))status=1;
    }fz_always(ctx){pdf_drop_processor(ctx,proc);fz_drop_device(ctx,device);fz_drop_pixmap(ctx,pixels);pdf_drop_obj(ctx,resources);fz_drop_buffer(ctx,buffer);pdf_drop_document(ctx,doc);}
    fz_catch(ctx){fprintf(stderr,"page raster oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
    fz_drop_context(ctx);free(bytes);return status;
}
