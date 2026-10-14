/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned MuPDF image decode oracle. Input16MiB, heap128MiB, output64MiB;
 * CPU10s/wall15s. No rendering or production native backend.
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


static void diagnostic(void *user,const char *message){int *n=user;(*n)++;(void)message;}
int main(int argc,char **argv) {
 unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length,ai;int status=0,diagnostics=0,object=0;
 budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
 fz_context *ctx;fz_stream *raw=NULL;pdf_document *doc=NULL;pdf_obj *ref=NULL;fz_image *image=NULL;fz_pixmap *pix=NULL;pdf_obj *mask=NULL;
 FILE *out=tmpfile();struct rlimit cpu={10,10};setrlimit(RLIMIT_CPU,&cpu);alarm(15);
 if(argc!=2||!bytes||!out)return 1;
 for(ai=0;argv[1][ai];ai++){if(argv[1][ai]<'0'||argv[1][ai]>'9'||object>100000)return 1;object=object*10+argv[1][ai]-'0';}if(object<=0||object>1000000)return 1;
 length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT)return 1;
 ctx=fz_new_context(&allocator,NULL,0);if(!ctx)return 1;fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);
 fz_var(raw);fz_var(doc);fz_var(ref);fz_var(image);fz_var(pix);fz_var(status);fz_var(diagnostics);
 fz_try(ctx) {
  int w,h,n,alpha,x,y;unsigned char *samples;
  raw=fz_open_memory(ctx,bytes,length);doc=pdf_open_document_with_stream(ctx,raw);pdf_disable_js(ctx,doc);
  if(pdf_needs_password(ctx,doc)||pdf_dict_get(ctx,pdf_trailer(ctx,doc),PDF_NAME(Encrypt)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"encrypted document");
  ref=pdf_new_indirect(ctx,doc,object,0);
  if(!pdf_is_stream(ctx,ref)||!pdf_name_eq(ctx,pdf_dict_get(ctx,ref,PDF_NAME(Subtype)),PDF_NAME(Image)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"image stream required");
  w=pdf_dict_get_int(ctx,ref,PDF_NAME(Width));h=pdf_dict_get_int(ctx,ref,PDF_NAME(Height));
  if(w<=0||h<=0||w>16384||h>16384||(uint64_t)w*h>4194304||pdf_dict_get_int(ctx,ref,PDF_NAME(BitsPerComponent))!=8)fz_throw(ctx,FZ_ERROR_LIMIT,"image dimension/depth bounds");
  if(!pdf_name_eq(ctx,pdf_dict_get(ctx,ref,PDF_NAME(ColorSpace)),PDF_NAME(DeviceGray))&&!pdf_name_eq(ctx,pdf_dict_get(ctx,ref,PDF_NAME(ColorSpace)),PDF_NAME(DeviceRGB)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"Gray/RGB required");
  mask=pdf_dict_get(ctx,ref,PDF_NAME(SMask));
  if(!pdf_is_indirect(ctx,mask)||!pdf_is_stream(ctx,mask))fz_throw(ctx,FZ_ERROR_ARGUMENT,"source SMask edge required");
  if(!pdf_name_eq(ctx,pdf_dict_get(ctx,mask,PDF_NAME(ColorSpace)),PDF_NAME(DeviceGray))||pdf_dict_get_int(ctx,mask,PDF_NAME(BitsPerComponent))!=8)fz_throw(ctx,FZ_ERROR_ARGUMENT,"source Gray8 required");
  image=pdf_load_image(ctx,doc,ref);if(!image->mask||image->imagemask)fz_throw(ctx,FZ_ERROR_ARGUMENT,"attached mask required");
  pix=fz_get_unscaled_pixmap_from_image(ctx,image->mask);
  fprintf(out,"{\"base_object\":%d,\"mask_object\":%d,\"width\":%d,\"height\":%d,\"alpha_hex\":\"",object,pdf_to_num(ctx,mask),w,h);
  if(fz_pixmap_width(ctx,pix)!=w||fz_pixmap_height(ctx,pix)!=h)fz_throw(ctx,FZ_ERROR_FORMAT,"decoded dimensions changed");
  n=fz_pixmap_components(ctx,pix);alpha=fz_pixmap_alpha(ctx,pix);
  if(n!=1||fz_pixmap_spots(ctx,pix))fz_throw(ctx,FZ_ERROR_FORMAT,"attached alpha components");
  (void)alpha;samples=fz_pixmap_samples(ctx,pix);
  for(y=0;y<h;y++)for(x=0;x<w;x++)fprintf(out,"%02x",samples[(size_t)y*fz_pixmap_stride(ctx,pix)+x]);
  fprintf(out,"\"}\n");
  fz_flush_warnings(ctx);if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair");
  if(fflush(out)||ftell(out)<0||ftell(out)>OUTPUT_LIMIT||fseek(out,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"output bound/seek");
  for(;;){unsigned char chunk[8192];size_t nread=fread(chunk,1,sizeof(chunk),out);if(nread&&fwrite(chunk,1,nread,stdout)!=nread)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout write");if(nread<sizeof(chunk)){if(ferror(out))fz_throw(ctx,FZ_ERROR_SYSTEM,"output read");break;}}
  if(fflush(stdout))status=1;
 }fz_always(ctx){fz_drop_pixmap(ctx,pix);fz_drop_image(ctx,image);pdf_drop_obj(ctx,ref);pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"image decode oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
 fz_drop_context(ctx);fclose(out);free(bytes);return status;
}
