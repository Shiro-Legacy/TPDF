/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only PDF simple embedded TrueType code/GID inventory; object-number argv.
 * PDF16MiB/native heap128MiB/output2MiB; CPU10s/wall15s. No system fallback.
 * Strict original fixture metadata evidence, not a production rendering path.
 */
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_TRUETYPE_TABLES_H
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
#define OUTPUT_LIMIT (2u*1024u*1024u)
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
static fz_font *no_system_font(fz_context *ctx,const char *name,int bold,int italic,int exact){(void)ctx;(void)name;(void)bold;(void)italic;(void)exact;return NULL;}
static fz_font *no_cjk(fz_context *ctx,const char *name,int order,int serif){(void)name;(void)order;(void)serif;fz_throw(ctx,FZ_ERROR_ARGUMENT,"CJK fonts forbidden");}
static fz_font *no_fallback(fz_context *ctx,int script,int language,int serif,int bold,int italic){(void)script;(void)language;(void)serif;(void)bold;(void)italic;fz_throw(ctx,FZ_ERROR_ARGUMENT,"fallback fonts forbidden");}
int main(int argc,char **argv) {
 unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length;int status=0,diagnostics=0,object=0;budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
 fz_context *ctx;fz_stream *raw=NULL;pdf_document *doc=NULL;pdf_font_desc *font=NULL;pdf_obj *ref=NULL;FILE *out=tmpfile();struct rlimit cpu={10,10};size_t ai;
 setrlimit(RLIMIT_CPU,&cpu);alarm(15);if(argc!=2||!bytes||!out)return 1;
 for(ai=0;argv[1][ai];ai++){if(argv[1][ai]<'0'||argv[1][ai]>'9'||object>100000)return 1;object=object*10+argv[1][ai]-'0';}if(object<=0||object>1000000)return 1;
 length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT)return 1;
 ctx=fz_new_context(&allocator,NULL,0);if(!ctx)return 1;fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);fz_install_load_system_font_funcs(ctx,no_system_font,no_cjk,no_fallback);
 fz_var(raw);fz_var(doc);fz_var(font);fz_var(ref);fz_var(status);fz_var(diagnostics);
 fz_try(ctx) {
  FT_Face face;int i,selected=-1;raw=fz_open_memory(ctx,bytes,length);doc=pdf_open_document_with_stream(ctx,raw);pdf_disable_js(ctx,doc);
  if(pdf_needs_password(ctx,doc)||pdf_dict_get(ctx,pdf_trailer(ctx,doc),PDF_NAME(Encrypt)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"encrypted document");
  ref=pdf_new_indirect(ctx,doc,object,0);if(!pdf_name_eq(ctx,pdf_dict_get(ctx,ref,PDF_NAME(Subtype)),PDF_NAME(TrueType))||!pdf_is_stream(ctx,pdf_dict_get(ctx,pdf_dict_get(ctx,ref,PDF_NAME(FontDescriptor)),PDF_NAME(FontFile2))))fz_throw(ctx,FZ_ERROR_ARGUMENT,"embedded simple TrueType only");
  font=pdf_load_font(ctx,doc,NULL,ref);if(!font->is_embedded||!font->font->ft_face)fz_throw(ctx,FZ_ERROR_ARGUMENT,"embedded FreeType face required");face=font->font->ft_face;
  if(face->num_faces!=1||face->num_glyphs<=0||face->num_glyphs>65535||!face->charmap||face->num_charmaps>16)fz_throw(ctx,FZ_ERROR_ARGUMENT,"font/charmap bounds");
  for(i=0;i<face->num_charmaps;i++)if(face->charmaps[i]==face->charmap)selected=i;
  if(selected<0||font->cid_to_gid_len!=256||!font->cid_to_gid)fz_throw(ctx,FZ_ERROR_FORMAT,"simple identity code table required");
  fprintf(out,"{\"font_object\":%d,\"flags\":%d,\"glyph_count\":%ld,\"selected_cmap\":{\"index\":%d,\"platform\":%d,\"encoding\":%d,\"format\":%ld},\"code_to_gid\":[",object,font->flags,face->num_glyphs,selected,face->charmap->platform_id,face->charmap->encoding_id,FT_Get_CMap_Format(face->charmap));
  for(i=0;i<256;i++){int gid=pdf_font_cid_to_gid(ctx,font,i);if(gid!=font->cid_to_gid[i]||gid<0||gid>=face->num_glyphs)fz_throw(ctx,FZ_ERROR_FORMAT,"gid bounds");fprintf(out,"%s%d",i?",":"",gid);}fprintf(out,"],\"cmaps\":[");
  for(i=0;i<face->num_charmaps;i++) {FT_ULong code;FT_UInt gid;size_t count=0;FT_CharMap cmap=face->charmaps[i];if(FT_Set_Charmap(face,cmap))fz_throw(ctx,FZ_ERROR_FORMAT,"set cmap");fprintf(out,"%s{\"platform\":%d,\"encoding\":%d,\"format\":%ld,\"mappings\":[",i?",":"",cmap->platform_id,cmap->encoding_id,FT_Get_CMap_Format(cmap));code=FT_Get_First_Char(face,&gid);while(gid){if(++count>65536||code>0x10ffff)fz_throw(ctx,FZ_ERROR_LIMIT,"cmap mapping budget");if(ftell(out)<0||ftell(out)>OUTPUT_LIMIT-128)fz_throw(ctx,FZ_ERROR_LIMIT,"output budget");fprintf(out,"%s[%lu,%u]",count==1?"":",",code,gid);code=FT_Get_Next_Char(face,code,&gid);}fprintf(out,"]}");}
  fprintf(out,"]}\n");fz_flush_warnings(ctx);if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair");
  if(fflush(out)||ftell(out)<0||ftell(out)>OUTPUT_LIMIT||fseek(out,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"output bound/seek");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),out);if(n&&fwrite(chunk,1,n,stdout)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout write");if(n<sizeof(chunk))break;}
 }fz_always(ctx){pdf_drop_font(ctx,font);pdf_drop_obj(ctx,ref);pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"glyph inventory oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
 fz_drop_context(ctx);fclose(out);free(bytes);return status;
}
