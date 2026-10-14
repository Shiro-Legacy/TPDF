/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only exact-name pinned CFF outline oracle. No PDF or system fonts.
 * Input4096 printable ASCII names<=127bytes; heap128MiB, output64MiB,
 * commands<=1million, CPU30s/wall45s; diagnostics fatal and stdout atomic.
 * FT NO_SCALE/NO_HINTING/NO_AUTOHINT/NO_BITMAP/IGNORE_TRANSFORM drawing origin.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <sys/resource.h>
#include <unistd.h>
#include "mupdf/fitz.h"
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_ADVANCES_H
#include FT_OUTLINE_H
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


typedef struct {fz_context *ctx;FILE *out;size_t bytes,commands,glyph_commands;int open,first;} capture;
static void emit(capture *c,const char *format,...){char buf[1024];va_list args;int n;va_start(args,format);n=vsnprintf(buf,sizeof buf,format,args);va_end(args);if(n<0||(size_t)n>=sizeof buf||(size_t)n>OUTPUT_LIMIT-c->bytes)fz_throw(c->ctx,FZ_ERROR_LIMIT,"output bound");if(fwrite(buf,1,(size_t)n,c->out)!=(size_t)n)fz_throw(c->ctx,FZ_ERROR_SYSTEM,"temporary output write");c->bytes+=(size_t)n;}
static void quote(capture*c,const char *s){emit(c,"\"");for(;*s;s++){if(*s=='"'||*s=='\\')emit(c,"\\%c",*s);else emit(c,"%c",*s);}emit(c,"\"");}
static void command(capture*c,const char *kind,const FT_Vector*a,const FT_Vector*b,const FT_Vector*d){const FT_Vector*points[]={a,b,d};int i;if(++c->commands>1000000||++c->glyph_commands>4096)fz_throw(c->ctx,FZ_ERROR_LIMIT,"outline command bound");emit(c,"%s[\"%s\"",c->first?"":",",kind);c->first=0;for(i=0;i<3;i++)if(points[i]){if(points[i]->x < -1000000 || points[i]->x>1000000 || points[i]->y < -1000000 || points[i]->y>1000000)fz_throw(c->ctx,FZ_ERROR_LIMIT,"coordinate bound");emit(c,",%ld,%ld",points[i]->x,points[i]->y);}emit(c,"]");}
static int move_to(const FT_Vector*p,void*opaque){capture*c=opaque;if(c->open)command(c,"h",NULL,NULL,NULL);command(c,"m",p,NULL,NULL);c->open=1;return 0;}
static int line_to(const FT_Vector*p,void*opaque){command(opaque,"l",p,NULL,NULL);return 0;}
static int quad_to(const FT_Vector*a,const FT_Vector*b,void*opaque){capture*c=opaque;(void)a;(void)b;fz_throw(c->ctx,FZ_ERROR_FORMAT,"quadratic outside pinned CFF contract");}
static int cubic_to(const FT_Vector*a,const FT_Vector*b,const FT_Vector*d,void*opaque){command(opaque,"c",a,b,d);return 0;}
static int read_name(fz_context*ctx,char *line){size_t n=0;int ch;while((ch=fgetc(stdin))!=EOF&&ch!='\n'){if(ch<33||ch>126||n>=127)fz_throw(ctx,FZ_ERROR_LIMIT,"glyph query syntax/length");line[n++]=(char)ch;}if(ch==EOF&&!n)return 0;if(!n)fz_throw(ctx,FZ_ERROR_FORMAT,"empty query");line[n]=0;return 1;}
static void diagnostic(void*opaque,const char*message){int*n=opaque;(*n)++;fprintf(stderr,"native diagnostic: %.500s\n",message);}
int main(int argc,char **argv){
 fz_context*ctx;fz_font*font=NULL;int status=0,diagnostics=0;FILE*out=NULL;char name[128];unsigned queries=0;budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};capture c={0};struct rlimit cpu={30,30};
 setrlimit(RLIMIT_CPU,&cpu);alarm(45);if(argc!=2||(strcmp(argv[1],"Helvetica")&&strcmp(argv[1],"Times-Roman")))return 1;out=tmpfile();if(!out)return 1;ctx=fz_new_context(&allocator,NULL,0);if(!ctx){fclose(out);return 1;}c.ctx=ctx;c.out=out;fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);fz_var(font);fz_var(status);fz_var(diagnostics);
 fz_try(ctx){FT_Face face;unsigned char*bytes,digest[32];size_t length;fz_sha256 hash;unsigned i;
  font=fz_new_base14_font(ctx,argv[1]);face=fz_font_ft_face(ctx,font);if(!face||face->units_per_EM!=1000||face->num_glyphs<1||face->num_glyphs>65535)fz_throw(ctx,FZ_ERROR_FORMAT,"pinned font metadata");length=fz_buffer_storage(ctx,font->buffer,&bytes);fz_sha256_init(&hash);fz_sha256_update(&hash,bytes,length);fz_sha256_final(&hash,digest);
  emit(&c,"{\"face\":");quote(&c,argv[1]);emit(&c,",\"asset_bytes\":%zu,\"asset_sha256\":\"",length);for(i=0;i<32;i++)emit(&c,"%02x",digest[i]);emit(&c,"\",\"units_per_em\":%u,\"glyph_count\":%ld,\"glyphs\":[",face->units_per_EM,face->num_glyphs);
  while(read_name(ctx,name)){FT_UInt gid;FT_Fixed advance=0;char actual[128];int found;
   if(++queries>4096)fz_throw(ctx,FZ_ERROR_LIMIT,"query count");fz_ft_lock(ctx);
   fz_try(ctx){gid=FT_Get_Name_Index(face,name);if(FT_Get_Glyph_Name(face,gid,actual,sizeof actual))fz_throw(ctx,FZ_ERROR_FORMAT,"native glyph name");found=!strcmp(name,actual);if(found&&FT_Get_Advance(face,gid,FT_LOAD_NO_SCALE|FT_LOAD_NO_HINTING|FT_LOAD_NO_AUTOHINT|FT_LOAD_NO_BITMAP|FT_LOAD_IGNORE_TRANSFORM,&advance))fz_throw(ctx,FZ_ERROR_FORMAT,"native advance");
    emit(&c,"%s{\"name\":",queries==1?"":",");quote(&c,name);emit(&c,",\"found\":%s,\"native_gid\":%u,\"advance\":%ld,\"commands\":[",found?"true":"false",gid,(long)advance);c.first=1;c.open=0;c.glyph_commands=0;
    if(found){FT_Outline_Funcs callbacks={move_to,line_to,quad_to,cubic_to,0,0};if(FT_Load_Glyph(face,gid,FT_LOAD_NO_SCALE|FT_LOAD_NO_HINTING|FT_LOAD_NO_AUTOHINT|FT_LOAD_NO_BITMAP|FT_LOAD_IGNORE_TRANSFORM))fz_throw(ctx,FZ_ERROR_FORMAT,"native glyph load");if(face->glyph->format!=FT_GLYPH_FORMAT_OUTLINE||face->glyph->outline.n_points<0||face->glyph->outline.n_points>32768||face->glyph->outline.n_contours<0||face->glyph->outline.n_contours>4096)fz_throw(ctx,FZ_ERROR_FORMAT,"native glyph outline bounds");if(FT_Outline_Decompose(&face->glyph->outline,&callbacks,&c))fz_throw(ctx,FZ_ERROR_FORMAT,"native outline decompose");if(c.open)command(&c,"h",NULL,NULL,NULL);}
    emit(&c,"]}");
   }fz_always(ctx){fz_ft_unlock(ctx);}fz_catch(ctx){fz_rethrow(ctx);}
  }
  if(ferror(stdin))fz_throw(ctx,FZ_ERROR_SYSTEM,"input failure");emit(&c,"],\"commands\":%zu}\n",c.commands);fz_flush_warnings(ctx);if(diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native diagnostics forbidden");if(fflush(out)||fseek(out,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"temporary output seek");for(;;){unsigned char buffer[8192];size_t n=fread(buffer,1,sizeof buffer,out);if(n&&fwrite(buffer,1,n,stdout)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout write");if(n<sizeof buffer){if(ferror(out))fz_throw(ctx,FZ_ERROR_SYSTEM,"output read");break;}}
 }fz_always(ctx){fz_drop_font(ctx,font);}fz_catch(ctx){fprintf(stderr,"builtin outline oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}fz_drop_context(ctx);fclose(out);if(fflush(stdout)||ferror(stdout))status=2;return status;
}
