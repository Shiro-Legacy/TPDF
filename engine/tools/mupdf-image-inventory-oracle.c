/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned MuPDF image object inventory oracle. Input16MiB, heap128MiB, output64MiB;
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


static void diagnostic(void *user,const char *message){int *n=user;(*n)++;fprintf(stderr,"native diagnostic: %.500s\n",message);}
int main(int argc,char **argv) {
 unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length;int status=0,diagnostics=0;
 budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
 fz_context *ctx;fz_stream *raw=NULL;pdf_document *doc=NULL;pdf_obj *ref=NULL;
 FILE *out=tmpfile();struct rlimit cpu={10,10};setrlimit(RLIMIT_CPU,&cpu);alarm(15);
 (void)argv;if(argc!=1||!bytes||!out)return 1;
 length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT)return 1;
 ctx=fz_new_context(&allocator,NULL,0);if(!ctx)return 1;fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);
 fz_var(raw);fz_var(doc);fz_var(ref);fz_var(status);fz_var(diagnostics);
 fz_try(ctx) {
  int count;
  raw=fz_open_memory(ctx,bytes,length);doc=pdf_open_document_with_stream(ctx,raw);pdf_disable_js(ctx,doc);
  if(pdf_needs_password(ctx,doc)||pdf_dict_get(ctx,pdf_trailer(ctx,doc),PDF_NAME(Encrypt)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"encrypted document");
  count=pdf_xref_len(ctx,doc);if(count>100000)fz_throw(ctx,FZ_ERROR_LIMIT,"xref inventory bound");
  fprintf(out,"image-object-inventory-v1\n");
  for(int i=1;i<count;i++) {
   pdf_xref_entry *entry=pdf_get_xref_entry(ctx,doc,i);int components;
   if(entry->type!='n'&&entry->type!='o')continue;
   ref=pdf_new_indirect(ctx,doc,i,entry->type=='o'?0:entry->gen);
   if(pdf_is_stream(ctx,ref)&&pdf_name_eq(ctx,pdf_dict_get(ctx,ref,PDF_NAME(Subtype)),PDF_NAME(Image))) {
    pdf_obj *cs=pdf_dict_get(ctx,ref,PDF_NAME(ColorSpace));pdf_obj *filter=pdf_dict_get(ctx,ref,PDF_NAME(Filter));
    components=pdf_name_eq(ctx,cs,PDF_NAME(DeviceGray))?1:pdf_name_eq(ctx,cs,PDF_NAME(DeviceRGB))?3:0;
    fprintf(out,"%d %d %d %d %d %d %d %d %d %d %d\n",i,pdf_to_gen(ctx,ref),pdf_dict_get_int(ctx,ref,PDF_NAME(Width)),pdf_dict_get_int(ctx,ref,PDF_NAME(Height)),pdf_dict_get_int(ctx,ref,PDF_NAME(BitsPerComponent)),components,pdf_dict_get(ctx,ref,PDF_NAME(Mask))!=NULL,pdf_dict_get(ctx,ref,PDF_NAME(SMask))!=NULL,pdf_dict_get_bool(ctx,ref,PDF_NAME(Interpolate)),pdf_dict_get_bool(ctx,ref,PDF_NAME(ImageMask)),pdf_is_array(ctx,filter)?pdf_array_len(ctx,filter):filter!=NULL);
   }
   pdf_drop_obj(ctx,ref);ref=NULL;
  }
  fz_flush_warnings(ctx);if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair");
  if(fflush(out)||ftell(out)<0||ftell(out)>OUTPUT_LIMIT||fseek(out,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"output bound/seek");
  for(;;){unsigned char chunk[8192];size_t nread=fread(chunk,1,sizeof(chunk),out);if(nread&&fwrite(chunk,1,nread,stdout)!=nread)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout write");if(nread<sizeof(chunk)){if(ferror(out))fz_throw(ctx,FZ_ERROR_SYSTEM,"output read");break;}}
  if(fflush(stdout))status=1;
 }fz_always(ctx){pdf_drop_obj(ctx,ref);pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"image inventory oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
 fz_drop_context(ctx);fclose(out);free(bytes);return status;
}
