/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only MuPDF1.28.1 native text placement oracle; generated one-page PDF stdin.
 * Identity processor CTM (no page y flip), primary/Unicode-continuation items.
 * Input4MiB, native heap128MiB, decoded content4MiB, output64MiB,
 * items1million, tokens1million, CPU10s/wall15s. Atomic successful output.
 * Uses bundled Base14 fonts only; system-loader callbacks cannot load files.
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

typedef struct { fz_device super; FILE *out; size_t bytes,items,callbacks; } trace_device;
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
static void text_event(fz_context *ctx,trace_device *d,const fz_text *text,fz_matrix ctm,int mode)
{
 const fz_text_span *span;size_t callback=d->callbacks++;if(d->callbacks>100000)fz_throw(ctx,FZ_ERROR_LIMIT,"callback limit");
 for(span=text->head;span;span=span->next){int i;if(span->wmode)fz_throw(ctx,FZ_ERROR_FORMAT,"vertical native text");
  if(span->len<0||span->len>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"span limit");
  for(i=0;i<span->len;i++){const fz_text_item *item=&span->items[i];fz_matrix m=span->trm;int primary=item->gid>=0&&item->cid>=0;
   if(++d->items>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"item limit");
   m.e=item->x;m.f=item->y;
   emit(ctx,d,"%s{\"callback\":%zu,\"mode\":%d,\"primary\":%s,\"cid\":%d,\"gid\":%d,\"unicode\":%d,\"advance\":",d->items==1?"":",",callback,mode,primary?"true":"false",item->cid,item->gid,item->ucs);number(ctx,d,item->adv);
   emit(ctx,d,",\"matrix\":");matrix(ctx,d,m);emit(ctx,d,",\"ctm\":");matrix(ctx,d,ctm);emit(ctx,d,",\"page_matrix\":");matrix(ctx,d,fz_concat(m,ctm));emit(ctx,d,"}");
  }
 }
}
static void fill_text(fz_context *ctx,fz_device *dev,const fz_text *text,fz_matrix ctm,fz_colorspace *cs,const float *color,float alpha,fz_color_params params)
{(void)cs;(void)color;(void)params;if(alpha!=1)fz_throw(ctx,FZ_ERROR_FORMAT,"unexpected text alpha");text_event(ctx,(trace_device*)dev,text,ctm,0);}
static void ignore_text(fz_context *ctx,fz_device *dev,const fz_text *text,fz_matrix ctm)
{text_event(ctx,(trace_device*)dev,text,ctm,3);}
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
{int i,n=pdf_dict_len(ctx,obj);if(!pdf_is_dict(ctx,obj)||n>32)fz_throw(ctx,FZ_ERROR_FORMAT,"dictionary shape limit");for(i=0;i<n;i++){const char *key=pdf_to_name(ctx,pdf_dict_get_key(ctx,obj,i));size_t j;for(j=0;j<count;j++)if(!strcmp(key,allowed[j]))break;if(j==count)fz_throw(ctx,FZ_ERROR_FORMAT,"unsupported dictionary key %s",key);}}
#define KEYS(ctx,obj,list) keys(ctx,obj,list,sizeof(list)/sizeof(list[0]))
static void lexical(fz_context *ctx,const unsigned char *bytes,size_t length,int cmap,pdf_obj *fonts)
{
 static const char *const words[]={"q","Q","cm","BT","ET","Tc","Tw","Tz","TL","Tf","Tr","Ts","Td","TD","Tm","T*","Tj","TJ","'","\""};
 fz_stream *stream=NULL;pdf_lexbuf lex;size_t tokens=0;int array=0,inside=0,saves=0;pdf_token previous=PDF_TOK_EOF;int64_t integer=0;char font_name[128]={0};int has_name=0;
 pdf_lexbuf_init(ctx,&lex,PDF_LEXBUF_SMALL);fz_var(stream);
 fz_try(ctx){stream=fz_open_memory(ctx,bytes,length);for(;;){pdf_token t;size_t j;lex.len=0;t=pdf_lex(ctx,stream,&lex);if(t==PDF_TOK_EOF)break;if(++tokens>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"token limit");
  if(cmap){if(t==PDF_TOK_KEYWORD&&!strcmp(lex.scratch,"usecmap"))fz_throw(ctx,FZ_ERROR_FORMAT,"usecmap forbidden");continue;}
  if(t==PDF_TOK_INT){integer=lex.i;previous=t;continue;}
  if(t==PDF_TOK_REAL){if(!isfinite(lex.f))fz_throw(ctx,FZ_ERROR_FORMAT,"nonfinite operand");previous=t;continue;}
  if(t==PDF_TOK_NAME&&!array){size_t n=strlen(lex.scratch);if(n>=sizeof(font_name))fz_throw(ctx,FZ_ERROR_LIMIT,"font resource name");memcpy(font_name,lex.scratch,n+1);has_name=1;}
  if(t==PDF_TOK_NAME||t==PDF_TOK_STRING){if(lex.len>1024*1024)fz_throw(ctx,FZ_ERROR_LIMIT,"operand length");previous=t;continue;}
  if(t==PDF_TOK_OPEN_ARRAY){if(array++)fz_throw(ctx,FZ_ERROR_FORMAT,"nested TJ array");previous=t;continue;}
  if(t==PDF_TOK_CLOSE_ARRAY){if(--array!=0)fz_throw(ctx,FZ_ERROR_FORMAT,"unmatched array");previous=t;continue;}
  if(t!=PDF_TOK_KEYWORD||array)fz_throw(ctx,FZ_ERROR_FORMAT,"outside T0 lexical subset");
  for(j=0;j<sizeof(words)/sizeof(words[0]);j++)if(!strcmp(lex.scratch,words[j]))break;if(j==sizeof(words)/sizeof(words[0]))fz_throw(ctx,FZ_ERROR_FORMAT,"operator outside T0");
  if(!strcmp(lex.scratch,"BT")){if(inside)fz_throw(ctx,FZ_ERROR_FORMAT,"nested BT");inside=1;}
  else if(!strcmp(lex.scratch,"ET")){if(!inside)fz_throw(ctx,FZ_ERROR_FORMAT,"unmatched ET");inside=0;}
  else if(!strcmp(lex.scratch,"q")||!strcmp(lex.scratch,"Q")||!strcmp(lex.scratch,"cm")){
   if(inside)fz_throw(ctx,FZ_ERROR_FORMAT,"special graphics state inside BT");
   if(!strcmp(lex.scratch,"q")){if(++saves>128)fz_throw(ctx,FZ_ERROR_LIMIT,"save depth");}else if(!strcmp(lex.scratch,"Q")&&--saves<0)fz_throw(ctx,FZ_ERROR_FORMAT,"unmatched Q");
  }else if(!strcmp(lex.scratch,"Tr")){if(previous!=PDF_TOK_INT||(integer!=0&&integer!=3))fz_throw(ctx,FZ_ERROR_FORMAT,"text rendering mode outside T0");}
  else if((!strcmp(lex.scratch,"Tm")||!strcmp(lex.scratch,"Td")||!strcmp(lex.scratch,"TD")||!strcmp(lex.scratch,"T*")||!strcmp(lex.scratch,"Tj")||!strcmp(lex.scratch,"TJ")||!strcmp(lex.scratch,"'")||!strcmp(lex.scratch,"\""))&&!inside)fz_throw(ctx,FZ_ERROR_FORMAT,"text operation outside BT");
  if(!strcmp(lex.scratch,"Tf")&&(!has_name||absent(ctx,pdf_dict_gets(ctx,fonts,font_name))))fz_throw(ctx,FZ_ERROR_FORMAT,"missing explicit font binding");
  has_name=0;previous=t;
 }if(!cmap&&(array||inside||saves))fz_throw(ctx,FZ_ERROR_FORMAT,"unclosed text/graphics state");}
 fz_always(ctx){fz_drop_stream(ctx,stream);pdf_lexbuf_fin(ctx,&lex);}fz_catch(ctx){fz_rethrow(ctx);}
}
static void stream_shape(fz_context *ctx,pdf_obj *obj)
{static const char *const allowed[]={"Length"};pdf_obj *length;KEYS(ctx,obj,allowed);if(!pdf_is_stream(ctx,obj))fz_throw(ctx,FZ_ERROR_FORMAT,"expected raw stream");length=pdf_dict_get(ctx,obj,PDF_NAME(Length));if(!pdf_is_int(ctx,length)||pdf_to_int64(ctx,length)<0||pdf_to_int64(ctx,length)>INPUT_LIMIT)fz_throw(ctx,FZ_ERROR_LIMIT,"raw stream length");}
static void encoding(fz_context *ctx,pdf_obj *obj)
{
 static const char *const allowed[]={"Type","BaseEncoding","Differences"};pdf_obj *base,*diff;int i,index=-1,n;
 if(pdf_is_name(ctx,obj)){if(!named(ctx,obj,"WinAnsiEncoding")&&!named(ctx,obj,"MacRomanEncoding")&&!named(ctx,obj,"StandardEncoding"))fz_throw(ctx,FZ_ERROR_FORMAT,"unsupported encoding");return;}
 KEYS(ctx,obj,allowed);base=pdf_dict_get(ctx,obj,PDF_NAME(BaseEncoding));if(!pdf_is_name(ctx,base))fz_throw(ctx,FZ_ERROR_FORMAT,"missing base encoding");encoding(ctx,base);diff=pdf_dict_get(ctx,obj,PDF_NAME(Differences));if(absent(ctx,diff))return;
 if(!pdf_is_array(ctx,diff)||(n=pdf_array_len(ctx,diff))>4096)fz_throw(ctx,FZ_ERROR_LIMIT,"Differences limit");
 for(i=0;i<n;i++){pdf_obj *v=pdf_array_get(ctx,diff,i);if(pdf_is_int(ctx,v)){int64_t x=pdf_to_int64(ctx,v);if(x<0||x>255)fz_throw(ctx,FZ_ERROR_FORMAT,"Differences index");index=(int)x;}else if(pdf_is_name(ctx,v)){if(index<0||index>255||strlen(pdf_to_name(ctx,v))>127)fz_throw(ctx,FZ_ERROR_FORMAT,"Differences name/index");index++;}else fz_throw(ctx,FZ_ERROR_FORMAT,"Differences type");}
}
static void resources(fz_context *ctx,pdf_obj *res)
{
 static const char *const allowed[]={"Font","ProcSet"};static const char *const fontkeys[]={"Type","Subtype","BaseFont","Encoding","FirstChar","LastChar","Widths","FontDescriptor","ToUnicode"};
 static const char *const desckeys[]={"Type","FontName","Flags","FontBBox","ItalicAngle","Ascent","Descent","CapHeight","StemV","MissingWidth"};
 pdf_obj *fonts;int i,n;KEYS(ctx,res,allowed);fonts=pdf_dict_get(ctx,res,PDF_NAME(Font));if(!pdf_is_dict(ctx,fonts)||(n=pdf_dict_len(ctx,fonts))>16)fz_throw(ctx,FZ_ERROR_LIMIT,"font count/type");
 for(i=0;i<n;i++){pdf_obj *font=pdf_dict_get_val(ctx,fonts,i),*widths,*first,*last,*desc,*tu;int j,count;KEYS(ctx,font,fontkeys);
  if(strlen(pdf_to_name(ctx,pdf_dict_get_key(ctx,fonts,i)))>127||!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(Subtype)),"Type1"))fz_throw(ctx,FZ_ERROR_FORMAT,"font subtype/name");
  if(!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(BaseFont)),"Helvetica")&&!named(ctx,pdf_dict_get(ctx,font,PDF_NAME(BaseFont)),"Times-Roman"))fz_throw(ctx,FZ_ERROR_FORMAT,"font outside bundled pair");
  encoding(ctx,pdf_dict_get(ctx,font,PDF_NAME(Encoding)));widths=pdf_dict_get(ctx,font,PDF_NAME(Widths));first=pdf_dict_get(ctx,font,PDF_NAME(FirstChar));last=pdf_dict_get(ctx,font,PDF_NAME(LastChar));
  if(!absent(ctx,widths)){int64_t low=pdf_to_int64(ctx,first),high=pdf_to_int64(ctx,last);if(!pdf_is_int(ctx,first)||!pdf_is_int(ctx,last)||low<0||high>255||low>high||!pdf_is_array(ctx,widths)||(count=pdf_array_len(ctx,widths))!=high-low+1)fz_throw(ctx,FZ_ERROR_FORMAT,"width declaration");for(j=0;j<count;j++){pdf_obj *v=pdf_array_get(ctx,widths,j);if(!pdf_is_int(ctx,v)||pdf_to_int64(ctx,v)<0||pdf_to_int64(ctx,v)>1000)fz_throw(ctx,FZ_ERROR_FORMAT,"native gate needs integer width0..1000");}}
  else if(!absent(ctx,first)||!absent(ctx,last))fz_throw(ctx,FZ_ERROR_FORMAT,"incomplete width declaration");
  desc=pdf_dict_get(ctx,font,PDF_NAME(FontDescriptor));if(!absent(ctx,desc)){pdf_obj *missing;KEYS(ctx,desc,desckeys);missing=pdf_dict_get(ctx,desc,PDF_NAME(MissingWidth));if(!absent(ctx,missing)&&(!pdf_is_int(ctx,missing)||pdf_to_int64(ctx,missing)<0||pdf_to_int64(ctx,missing)>1000))fz_throw(ctx,FZ_ERROR_FORMAT,"MissingWidth gate");}
  tu=pdf_dict_get(ctx,font,PDF_NAME(ToUnicode));if(!absent(ctx,tu)){fz_buffer *buf=NULL;unsigned char *data;size_t length;stream_shape(ctx,tu);fz_var(buf);fz_try(ctx){buf=pdf_load_stream(ctx,tu);length=fz_buffer_storage(ctx,buf,&data);if(length>INPUT_LIMIT)fz_throw(ctx,FZ_ERROR_LIMIT,"ToUnicode length");lexical(ctx,data,length,1,NULL);}fz_always(ctx){fz_drop_buffer(ctx,buf);}fz_catch(ctx){fz_rethrow(ctx);}}
 }
}
int main(int argc,char **argv)
{
 unsigned char *bytes=malloc(INPUT_LIMIT+1);size_t length;int status=0,diagnostics=0;FILE *output=tmpfile();budget heap={0};fz_alloc_context allocator={&heap,limited_malloc,limited_realloc,limited_free};
 fz_context *ctx;fz_stream *raw=NULL;pdf_document *doc=NULL;pdf_processor *proc=NULL;fz_buffer *content=NULL;trace_device *dev=NULL;struct rlimit cpu={10,10};(void)argv;
 setrlimit(RLIMIT_CPU,&cpu);alarm(15);if(argc!=1||!bytes||!output){free(bytes);if(output)fclose(output);return 1;}length=fread(bytes,1,INPUT_LIMIT+1,stdin);if(ferror(stdin)||length>INPUT_LIMIT){free(bytes);fclose(output);return 1;}
 ctx=fz_new_context(&allocator,NULL,0);if(!ctx){free(bytes);fclose(output);return 1;}fz_set_warning_callback(ctx,diagnostic,&diagnostics);fz_set_error_callback(ctx,diagnostic,&diagnostics);fz_install_load_system_font_funcs(ctx,no_system_font,no_cjk_font,no_fallback_font);
 fz_var(raw);fz_var(doc);fz_var(proc);fz_var(content);fz_var(dev);fz_var(status);fz_var(diagnostics);
 fz_try(ctx){pdf_obj *page,*res,*contents;unsigned char *data;size_t len;
  raw=fz_open_memory(ctx,bytes,length);doc=pdf_open_document_with_stream(ctx,raw);pdf_disable_js(ctx,doc);if(pdf_needs_password(ctx,doc)||!absent(ctx,pdf_dict_get(ctx,pdf_trailer(ctx,doc),PDF_NAME(Encrypt))))fz_throw(ctx,FZ_ERROR_FORMAT,"encryption forbidden");
  if(pdf_count_pages(ctx,doc)!=1)fz_throw(ctx,FZ_ERROR_FORMAT,"expected one generated page");page=pdf_lookup_page_obj(ctx,doc,0);res=pdf_dict_get_inheritable(ctx,page,PDF_NAME(Resources));resources(ctx,res);contents=pdf_dict_get(ctx,page,PDF_NAME(Contents));stream_shape(ctx,contents);content=pdf_load_stream(ctx,contents);len=fz_buffer_storage(ctx,content,&data);if(len>INPUT_LIMIT)fz_throw(ctx,FZ_ERROR_LIMIT,"content length");lexical(ctx,data,len,0,pdf_dict_get(ctx,res,PDF_NAME(Font)));
  dev=fz_new_derived_device(ctx,trace_device);dev->out=output;dev->super.fill_text=fill_text;dev->super.ignore_text=ignore_text;emit(ctx,dev,"{\"items\":[");proc=pdf_new_run_processor(ctx,doc,&dev->super,fz_identity,-1,"View",NULL,NULL,NULL,NULL,NULL);pdf_process_glyph(ctx,proc,doc,res,content);pdf_close_processor(ctx,proc);fz_close_device(ctx,&dev->super);fz_flush_warnings(ctx);
  if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair forbidden");emit(ctx,dev,"],\"callbacks\":%zu,\"repaired\":false}\n",dev->callbacks);
  if(fflush(output)||fseek(output,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace seek I/O");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),output);if(n&&fwrite(chunk,1,n,stdout)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout I/O");if(n<sizeof(chunk)){if(ferror(output))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace read I/O");break;}}if(fflush(stdout))status=1;
 }fz_always(ctx){pdf_drop_processor(ctx,proc);fz_drop_device(ctx,(fz_device*)dev);fz_drop_buffer(ctx,content);pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"text-position oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}
 fz_drop_context(ctx);fclose(output);free(bytes);return status;
}
