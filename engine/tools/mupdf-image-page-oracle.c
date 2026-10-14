/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only full original PDF page renderer, PAGE DPI arguments.
 * Input16MiB, heap128MiB, 4096 per axis/16M pixels, CPU30s/wall45s.
 * AA0, nearest images, QUALITY (no decode subsampling), original native gridfit.
 * Fail closed on repair/warning/error before stdout. No production native backend.
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


static void diagnostic(void *user,const char *message){int *n=user;(*n)++;fprintf(stderr,"native diagnostic: %.500s\n",message);}
int main(int argc,char **argv) {
 unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length;int status=0,diagnostics=0,index,dpi;
 budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
 fz_context *ctx;fz_stream *raw=NULL;pdf_document *doc=NULL;pdf_page *page=NULL;fz_pixmap *pix=NULL;fz_device *dev=NULL;
 struct rlimit cpu={30,30};setrlimit(RLIMIT_CPU,&cpu);alarm(45);
 if(argc!=3||!bytes)return 1;
 index=0;dpi=0;
 for(const char *p=argv[1];*p;p++){if(*p<'0'||*p>'9'||index>100000)return 1;index=index*10+*p-'0';}
 for(const char *p=argv[2];*p;p++){if(*p<'0'||*p>'9'||dpi>288)return 1;dpi=dpi*10+*p-'0';}
 if(index<1||index>1000000||dpi<1||dpi>288)return 1;
 length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT)return 1;
 ctx=fz_new_context(&allocator,NULL,0);if(!ctx)return 1;fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);
 fz_var(raw);fz_var(doc);fz_var(page);fz_var(pix);fz_var(dev);fz_var(status);fz_var(diagnostics);
 fz_try(ctx) {
  fz_rect bounds;fz_irect bbox;fz_matrix scale=fz_scale(dpi/72.f,dpi/72.f);int w,h,stride;unsigned char *samples;
  raw=fz_open_memory(ctx,bytes,length);doc=pdf_open_document_with_stream(ctx,raw);pdf_disable_js(ctx,doc);
  if(pdf_needs_password(ctx,doc)||pdf_dict_get(ctx,pdf_trailer(ctx,doc),PDF_NAME(Encrypt)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"encrypted document");
  if(index>pdf_count_pages(ctx,doc))fz_throw(ctx,FZ_ERROR_ARGUMENT,"page range");
  page=pdf_load_page(ctx,doc,index-1);bounds=fz_transform_rect(pdf_bound_page(ctx,page,FZ_CROP_BOX),scale);
  if(!isfinite(bounds.x0)||!isfinite(bounds.y0)||!isfinite(bounds.x1)||!isfinite(bounds.y1)||fabsf(bounds.x0)>1e6f||fabsf(bounds.y0)>1e6f||fabsf(bounds.x1)>1e6f||fabsf(bounds.y1)>1e6f)fz_throw(ctx,FZ_ERROR_LIMIT,"page bounds");
  bbox=fz_round_rect(bounds);w=bbox.x1-bbox.x0;h=bbox.y1-bbox.y0;
  if(w<=0||h<=0||w>4096||h>4096||(uint64_t)w*h>16777216)fz_throw(ctx,FZ_ERROR_LIMIT,"raster dimensions");
  fz_set_aa_level(ctx,0);fz_set_graphics_min_line_width(ctx,0);
  /* QUALITY suppresses l2factor image subsampling in the pinned image.c. */
  fz_tune_image_rendering(ctx,FZ_IMAGE_RENDERING_QUALITY);
  pix=fz_new_pixmap_with_bbox(ctx,fz_device_rgb(ctx),bbox,NULL,0);fz_clear_pixmap_with_value(ctx,pix,255);
  dev=fz_new_draw_device(ctx,fz_identity,pix);fz_enable_device_hints(ctx,dev,FZ_DONT_INTERPOLATE_IMAGES);
  pdf_run_page_contents(ctx,page,dev,scale,NULL);fz_close_device(ctx,dev);fz_flush_warnings(ctx);
  if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair");
  if(fz_pixmap_components(ctx,pix)!=3||fz_pixmap_alpha(ctx,pix))fz_throw(ctx,FZ_ERROR_FORMAT,"pixel format");
  samples=fz_pixmap_samples(ctx,pix);stride=fz_pixmap_stride(ctx,pix);if(stride<w*3)fz_throw(ctx,FZ_ERROR_FORMAT,"stride");
  if(fprintf(stdout,"P6\n%d %d\n255\n",w,h)<0)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout header");
  for(int y=0;y<h;y++)if(fwrite(samples+(size_t)y*stride,3,w,stdout)!=(size_t)w)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout pixels");
  if(fflush(stdout))status=1;
 }fz_always(ctx){fz_drop_device(ctx,dev);fz_drop_pixmap(ctx,pix);fz_drop_page(ctx,(fz_page*)page);pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"image page oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
 fz_drop_context(ctx);free(bytes);return status;
}
