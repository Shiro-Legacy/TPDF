/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only MuPDF1.28.1 Image/Form-aware original-page embedded and pinned Base14 outline placement trace; PDF stdin.
 * Native page CTM and initial page-bounds clip metadata; ordered callbacks.
 * Input16MiB, native heap128MiB, decoded page4MiB/document128MiB, output64MiB.
 * Page/document work caps, clips1024, CPU30s/wall45s.
 * Atomic successful output; implicit cleanup clip pops separately counted.
 * Shown embedded TrueType or two pinned Base14 faces; system-loader callbacks cannot load files.
 * Unhinted actual drawing origin is primary. fz_outline is separate hinted evidence.
 */
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_OUTLINE_H
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
typedef struct {pdf_font_desc *desc;int object,generation,program,program_generation,builtin;} native_font_binding;
typedef struct {fz_device super;native_font_binding fonts[256];size_t font_count;FILE *out;size_t bytes,items,callbacks,events,commands,cleanup_pops;int depth,recording,path_first;size_t clips[1024];work_budget *work;text_context *contexts;size_t context_count,context_capacity;size_t *context_clips;size_t context_clip_count,context_clip_capacity;} trace_device;
static void emit(fz_context *ctx,trace_device *d,const char *format,...)
{
 char buffer[1024];int n;va_list args;va_start(args,format);n=vsnprintf(buffer,sizeof(buffer),format,args);va_end(args);
 if(n<0||(size_t)n>=sizeof(buffer)||(size_t)n>OUTPUT_LIMIT-d->bytes)fz_throw(ctx,FZ_ERROR_LIMIT,"trace output limit");
 if(fwrite(buffer,1,(size_t)n,d->out)!=(size_t)n)fz_throw(ctx,FZ_ERROR_SYSTEM,"trace I/O");d->bytes+=(size_t)n;
}
static void hex_bytes(fz_context *ctx,trace_device *d,const unsigned char *bytes,size_t length)
{size_t i;char chunk[513];const char *hex="0123456789abcdef";emit(ctx,d,"\"");for(i=0;i<length;){size_t j,n=length-i;if(n>256)n=256;for(j=0;j<n;j++){chunk[2*j]=hex[bytes[i+j]>>4];chunk[2*j+1]=hex[bytes[i+j]&15];}chunk[2*n]=0;emit(ctx,d,"%s",chunk);i+=n;}emit(ctx,d,"\"");}
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
typedef struct {const unsigned char *bytes;size_t length;} font_slice;
static font_slice font_sub(fz_context *ctx,font_slice s,size_t off,size_t length){if(off>s.length||length>s.length-off)fz_throw(ctx,FZ_ERROR_ARGUMENT,"font table bounds");return (font_slice){s.bytes+off,length};}
static unsigned font_u16(fz_context *ctx,font_slice s,size_t at){font_slice p=font_sub(ctx,s,at,2);return ((unsigned)p.bytes[0]<<8)|p.bytes[1];}
static int font_i16(fz_context *ctx,font_slice s,size_t at){unsigned n=font_u16(ctx,s,at);return n<32768?(int)n:(int)n-65536;}
static unsigned font_u32(fz_context *ctx,font_slice s,size_t at){font_slice p=font_sub(ctx,s,at,4);return ((unsigned)p.bytes[0]<<24)|((unsigned)p.bytes[1]<<16)|((unsigned)p.bytes[2]<<8)|p.bytes[3];}
static font_slice font_table(fz_context *ctx,font_slice s,const char *tag){unsigned n=font_u16(ctx,s,4),i;int found=0;font_slice result={NULL,0};if(n>256)fz_throw(ctx,FZ_ERROR_LIMIT,"font table count");font_sub(ctx,s,12,n*16u);for(i=0;i<n;i++){size_t p=12+i*16u;if(!memcmp(s.bytes+p,tag,4)){if(found++)fz_throw(ctx,FZ_ERROR_ARGUMENT,"duplicate font table");result=font_sub(ctx,s,font_u32(ctx,s,p+8),font_u32(ctx,s,p+12));}}if(!found)fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing required font table");return result;}
static int glyph_bearing_adjustment(fz_context *ctx,fz_font *font,int gid,int *left,size_t ranges[4]){
 unsigned char *bytes;size_t length=fz_buffer_storage(ctx,font->buffer,&bytes);font_slice s={bytes,length},head,maxp,loca,glyf,hhea,hmtx,body;unsigned count,format,metrics;size_t begin,end;int lsb;
 if(length>32u*1024u*1024u||font_u32(ctx,s,0)!=0x10000)fz_throw(ctx,FZ_ERROR_ARGUMENT,"standalone TrueType only");head=font_table(ctx,s,"head");maxp=font_table(ctx,s,"maxp");loca=font_table(ctx,s,"loca");glyf=font_table(ctx,s,"glyf");hhea=font_table(ctx,s,"hhea");hmtx=font_table(ctx,s,"hmtx");count=font_u16(ctx,maxp,4);format=font_u16(ctx,head,50);metrics=font_u16(ctx,hhea,34);
 if(gid<=0||(unsigned)gid>=count||format>1||!metrics||metrics>count)fz_throw(ctx,FZ_ERROR_ARGUMENT,"glyph/metric metadata");begin=format?font_u32(ctx,loca,(size_t)gid*4):font_u16(ctx,loca,(size_t)gid*2)*2u;end=format?font_u32(ctx,loca,(size_t)(gid+1)*4):font_u16(ctx,loca,(size_t)(gid+1)*2)*2u;if(end<begin)fz_throw(ctx,FZ_ERROR_ARGUMENT,"loca order");body=font_sub(ctx,glyf,begin,end-begin);lsb=(unsigned)gid<metrics?font_i16(ctx,hmtx,(size_t)gid*4+2):font_i16(ctx,hmtx,(size_t)metrics*4+((size_t)gid-metrics)*2);*left=lsb;ranges[0]=(size_t)(hhea.bytes-bytes);ranges[1]=ranges[0]+hhea.length;ranges[2]=(size_t)(hmtx.bytes-bytes);ranges[3]=ranges[2]+hmtx.length;if(!body.length)return -lsb;if(font_i16(ctx,body,0)<0)fz_throw(ctx,FZ_ERROR_ARGUMENT,"composite glyph outside O0");font_sub(ctx,body,0,10);
 lsb=(unsigned)gid<metrics?font_i16(ctx,hmtx,(size_t)gid*4+2):font_i16(ctx,hmtx,(size_t)metrics*4+((size_t)gid-metrics)*2);return font_i16(ctx,body,2)-lsb;
}
typedef struct {fz_context *ctx;trace_device *d;fz_matrix placement;unsigned units;int raw,adjustment,open;} glyph_capture;
static void glyph_command(glyph_capture *c,const char *name,const FT_Vector *first,const FT_Vector *second,const FT_Vector *third){
 const FT_Vector *points[3]={first,second,third};int i;trace_device *d=c->d;fz_context *ctx=c->ctx;
 if(++d->commands>1000000||++d->work->commands>4000000)fz_throw(ctx,FZ_ERROR_LIMIT,"glyph command limit");emit(ctx,d,"%s[\"%s\"",d->path_first?"":",",name);d->path_first=0;
 for(i=0;i<3;i++)if(points[i]){const FT_Vector *p=points[i];if(c->raw)emit(ctx,d,",%ld,%ld",p->x+2*c->adjustment,p->y);else{fz_point q=fz_transform_point_xy((float)p->x/(2*c->units),(float)p->y/(2*c->units),c->placement);emit(ctx,d,",");number(ctx,d,q.x);emit(ctx,d,",");number(ctx,d,q.y);}}emit(ctx,d,"]");
}
static int glyph_move(const FT_Vector *p,void *value){glyph_capture*c=value;if(c->open)glyph_command(c,"h",NULL,NULL,NULL);glyph_command(c,"m",p,NULL,NULL);c->open=1;return 0;}
static int glyph_line(const FT_Vector *p,void *value){glyph_command(value,"l",p,NULL,NULL);return 0;}
static int glyph_quad(const FT_Vector *a,const FT_Vector *b,void *value){glyph_command(value,"q",a,b,NULL);return 0;}
static int glyph_cubic(const FT_Vector *a,const FT_Vector *b,const FT_Vector *p,void *value){glyph_command(value,"c",a,b,p);return 0;}
static void quadto(fz_context *ctx,void *p,float a,float b,float x,float y){float v[]={a,b,x,y};command(ctx,p,"q",v,4);}
static void quote(fz_context *ctx,trace_device *d,const char *value);
static void glyph_outlines(fz_context *ctx,trace_device *d,fz_font *font,int gid,fz_matrix placement,int builtin){
 FT_Face face=font->ft_face;FT_Outline *outline;unsigned units;int adjustment,i,left,selected=-1;size_t ranges[4];glyph_capture capture;fz_path *path=NULL;
 static const FT_Outline_Funcs callbacks={glyph_move,glyph_line,glyph_quad,glyph_cubic,0,0};static const fz_path_walker walker={moveto,lineto,curveto,closepath,quadto,NULL,NULL,NULL};
 fz_ft_lock(ctx);fz_try(ctx){
 if(!face||font->flags.ft_substitute||font->flags.fake_bold||font->flags.fake_italic||font->flags.ft_stretch)fz_throw(ctx,FZ_ERROR_ARGUMENT,"synthetic/substitute font outside R20");units=face->units_per_EM;if(units<16||units>16384)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unitsPerEm");if(builtin){char name[128];if(units!=1000||gid<=0||gid>=face->num_glyphs||FT_Get_Glyph_Name(face,gid,name,sizeof name))fz_throw(ctx,FZ_ERROR_ARGUMENT,"builtin glyph identity");adjustment=0;emit(ctx,d,",\"source_kind\":\"pinned_base14\",\"face\":\"%s\",\"glyph_name\":",builtin==1?"Helvetica":"Times-Roman");quote(ctx,d,name);emit(ctx,d,",\"asset_sha256\":\"%s\"",builtin==1?"366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706":"eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c");}else{emit(ctx,d,",\"source_kind\":\"embedded_true_type\"");adjustment=glyph_bearing_adjustment(ctx,font,gid,&left,ranges);for(i=0;i<face->num_charmaps;i++)if(face->charmaps[i]==face->charmap)selected=i;if(selected<0)fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing selected charmap");emit(ctx,d,",\"metric_lsb\":%d,\"hhea_range\":[%zu,%zu],\"hmtx_range\":[%zu,%zu],\"selection\":{\"index\":%d,\"platform\":%u,\"encoding\":%u,\"format\":%ld}",left,ranges[0],ranges[1],ranges[2],ranges[3],selected,face->charmap->platform_id,face->charmap->encoding_id,FT_Get_CMap_Format(face->charmap));}
 if(FT_Load_Glyph(face,gid,FT_LOAD_NO_SCALE|FT_LOAD_NO_HINTING|FT_LOAD_NO_AUTOHINT|FT_LOAD_NO_BITMAP|FT_LOAD_IGNORE_TRANSFORM))fz_throw(ctx,FZ_ERROR_FORMAT,"load unhinted glyph");if(face->glyph->format!=FT_GLYPH_FORMAT_OUTLINE)fz_throw(ctx,FZ_ERROR_ARGUMENT,"native non-outline glyph");outline=&face->glyph->outline;if(outline->n_points<0||outline->n_points>32768||outline->n_contours<0||outline->n_contours>4096)fz_throw(ctx,FZ_ERROR_LIMIT,"native outline size");
 for(i=0;i<outline->n_points;i++){FT_Vector *p=&outline->points[i];if(p->x < -131072 || p->x > 131072 || p->y < -131072 || p->y > 131072)fz_throw(ctx,FZ_ERROR_ARGUMENT,"outline coordinate bounds");p->x*=2;p->y*=2;}
 capture=(glyph_capture){ctx,d,placement,units,1,adjustment,0};emit(ctx,d,",\"units_per_em\":%u,\"bearing_adjustment\":%d,\"raw_commands\":[",units,adjustment);d->path_first=1;if(FT_Outline_Decompose(outline,&callbacks,&capture))fz_throw(ctx,FZ_ERROR_FORMAT,"raw decompose");if(capture.open)glyph_command(&capture,"h",NULL,NULL,NULL);
 emit(ctx,d,"],\"unhinted_commands\":[");capture.raw=0;capture.open=0;d->path_first=1;if(FT_Outline_Decompose(outline,&callbacks,&capture))fz_throw(ctx,FZ_ERROR_FORMAT,"unhinted decompose");if(capture.open)glyph_command(&capture,"h",NULL,NULL,NULL);emit(ctx,d,"],\"native_outline\":[");d->path_first=1;}fz_always(ctx){fz_ft_unlock(ctx);}fz_catch(ctx){fz_rethrow(ctx);}fz_var(path);fz_try(ctx){path=fz_outline_glyph(ctx,font,gid,placement);if(!path)fz_throw(ctx,FZ_ERROR_ARGUMENT,"native outline unavailable");fz_walk_path(ctx,path,&walker,d);}fz_always(ctx){fz_drop_path(ctx,path);}fz_catch(ctx){fz_rethrow(ctx);}emit(ctx,d,"]");
}
static void compact_code(fz_context *ctx,trace_device *d,int cid,const int *unicode,size_t count,fz_matrix placement,float advance,int mode,size_t context_id,size_t callback,int gid,fz_font *font){
 size_t i;int match=-1;if(++d->events>300000||++d->work->events>4000000)fz_throw(ctx,FZ_ERROR_LIMIT,"event count limit");for(i=0;i<d->font_count;i++)if(d->fonts[i].desc->font==font){if(match>=0&&(d->fonts[match].object!=d->fonts[i].object||d->fonts[match].program!=d->fonts[i].program))fz_throw(ctx,FZ_ERROR_ARGUMENT,"ambiguous native font identity");match=(int)i;}
 if(match<0||!d->fonts[match].object||(!d->fonts[match].builtin&&(!d->fonts[match].program||!d->fonts[match].desc->is_embedded)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"shown font outside indirect embedded TrueType or pinned Base14");
 emit(ctx,d,"%s{\"kind\":\"glyph\",\"code\":%d,\"gid\":%d,\"unicode\":[",d->events==1?"":",",cid,gid);for(i=0;i<count;i++){if(i)emit(ctx,d,",");emit(ctx,d,"%d",unicode[i]);}emit(ctx,d,"],\"matrix\":");matrix(ctx,d,placement);emit(ctx,d,",\"advance\":");number(ctx,d,advance);emit(ctx,d,",\"mode\":%d,\"context\":%zu,\"callback\":%zu,\"font\":[%d,%d]",mode,context_id,callback,d->fonts[match].object,d->fonts[match].generation);if(!d->fonts[match].builtin)emit(ctx,d,",\"program\":[%d,%d]",d->fonts[match].program,d->fonts[match].program_generation);glyph_outlines(ctx,d,font,gid,placement,d->fonts[match].builtin);emit(ctx,d,"}");
}
static void text_event(fz_context *ctx,trace_device *d,const fz_text *text,fz_matrix ctm,int mode,fz_colorspace *cs,const float *values,float alpha)
{
 const fz_text_span *span;size_t callback=d->callbacks++,context_id=context(ctx,d,cs,values,alpha),count=0;int cid=-1,gid=-1,unicode[256];fz_font *font=NULL;fz_matrix placement=fz_identity;float advance=0;
 if(d->callbacks>100000||++d->work->callbacks>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"callback limit");
 for(span=text->head;span;span=span->next){int i;if(span->wmode)fz_throw(ctx,FZ_ERROR_ARGUMENT,"vertical native text");if(span->len<0||span->len>1000000)fz_throw(ctx,FZ_ERROR_LIMIT,"span limit");
  for(i=0;i<span->len;i++){const fz_text_item *item=&span->items[i];int primary=item->gid>=0&&item->cid>=0;if(++d->items>1000000||++d->work->items>4000000)fz_throw(ctx,FZ_ERROR_LIMIT,"item limit");
   if(primary){fz_matrix m=span->trm;if(cid>=0)compact_code(ctx,d,cid,unicode,count,placement,advance,mode,context_id,callback,gid,font);if(item->cid>255)fz_throw(ctx,FZ_ERROR_ARGUMENT,"non-byte source code");cid=item->cid;gid=item->gid;font=span->font;count=0;m.e=item->x;m.f=item->y;placement=fz_concat(m,ctm);advance=item->adv;}
   else if(cid<0)fz_throw(ctx,FZ_ERROR_FORMAT,"orphan Unicode continuation");
   if(count>=256)fz_throw(ctx,FZ_ERROR_LIMIT,"Unicode continuation limit");if(item->ucs<0||item->ucs>0x10ffff||(item->ucs>=0xd800&&item->ucs<=0xdfff))fz_throw(ctx,FZ_ERROR_ARGUMENT,"native undefined Unicode");unicode[count++]=item->ucs;
  }
 }
 if(cid>=0)compact_code(ctx,d,cid,unicode,count,placement,advance,mode,context_id,callback,gid,font);
}
static void emit_contexts(fz_context *ctx,trace_device *d)
{
 size_t i,j;emit(ctx,d,"],\"contexts\":[");for(i=0;i<d->context_count;i++){text_context *c=&d->contexts[i];if(i)emit(ctx,d,",");emit(ctx,d,"{\"clip_ids\":[");for(j=0;j<(size_t)c->depth;j++){if(j)emit(ctx,d,",");emit(ctx,d,"%zu",d->context_clips[c->offset+j]);}emit(ctx,d,"],\"color_observable\":%s",c->color_kind?"true":"false");if(c->color_kind)color(ctx,d,c->color_kind==1?fz_device_gray(ctx):c->color_kind==2?fz_device_rgb(ctx):fz_device_cmyk(ctx),c->values,1);else emit(ctx,d,",\"color\":null");emit(ctx,d,"}");}emit(ctx,d,"]");
}
static void drop_trace(fz_context *ctx,fz_device *dev){trace_device *d=(trace_device*)dev;size_t i;for(i=0;i<d->font_count;i++)pdf_drop_font(ctx,d->fonts[i].desc);fz_free(ctx,d->contexts);fz_free(ctx,d->context_clips);}
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
typedef struct {int object,invocation,phase;size_t start,end;char op[8];} native_origin;
typedef struct {int object,generation;size_t origin;} image_call;
typedef struct {image_call images[4096];size_t image_count,image_cursor;pdf_document *doc;pdf_obj *resources[65];int objects[65];int depth,current;size_t invocations,origin_count,origin_capacity;native_origin *origins;} form_preflight;
static form_preflight *preflight;
static void preflight_form(fz_context*,const char*,pdf_obj*,work_budget*,size_t,size_t);
static void origin(fz_context *ctx,int invocation,int phase,size_t start,size_t end,const char *op)
{native_origin *r;size_t cap;if(preflight->origin_count>=100000||strlen(op)>=8)fz_throw(ctx,FZ_ERROR_LIMIT,"origin work limit");if(preflight->origin_count==preflight->origin_capacity){cap=preflight->origin_capacity?preflight->origin_capacity*2:256;if(cap>100000)cap=100000;preflight->origins=fz_realloc_array(ctx,preflight->origins,cap,native_origin);preflight->origin_capacity=cap;}r=&preflight->origins[preflight->origin_count++];r->object=preflight->objects[preflight->depth];r->invocation=invocation;r->phase=phase;r->start=start;r->end=end;strcpy(r->op,op);}
static size_t token_start(const unsigned char *bytes,size_t length,size_t pos)
{while(pos<length){unsigned char c=bytes[pos];if(c==0||c==9||c==10||c==12||c==13||c==32){pos++;continue;}if(c=='%'){while(pos<length&&bytes[pos]!=10&&bytes[pos]!=13)pos++;continue;}break;}return pos;}
static pdf_obj *scoped_resource(fz_context *ctx,const char *category,const char *name)
{int i;for(i=preflight->depth;i>=0;i--){pdf_obj *v=pdf_dict_gets(ctx,pdf_dict_gets(ctx,preflight->resources[i],category),name);if(!absent(ctx,v))return v;}return NULL;}
static void lexical(fz_context *ctx,const unsigned char *bytes,size_t length,int cmap,pdf_obj *fonts,pdf_obj *used,work_budget *work)
{
 static const char *const words[]={"Do","q","Q","cm","BT","ET","Tc","Tw","Tz","TL","Tf","Tr","Ts","Td","TD","Tm","T*","Tj","TJ","'","\"","w","J","j","M","d","m","l","c","v","y","h","re","S","s","f","F","f*","B","B*","b","b*","n","W","W*","G","g","RG","rg","K","k"};
 fz_stream *stream=NULL;pdf_lexbuf lex;size_t tokens=0,operation_start=SIZE_MAX;int array=0,inside=0,saves=0,active_path=0,pending_clip=0;pdf_token previous=PDF_TOK_EOF;int64_t integer=0;char font_name[128]={0};int has_name=0;
 (void)fonts;pdf_lexbuf_init(ctx,&lex,PDF_LEXBUF_SMALL);fz_var(stream);
 fz_try(ctx){stream=fz_open_memory(ctx,bytes,length);for(;;){pdf_token t;size_t j,begin=token_start(bytes,length,(size_t)fz_tell(ctx,stream));lex.len=0;t=pdf_lex(ctx,stream,&lex);if(t==PDF_TOK_EOF)break;if(operation_start==SIZE_MAX)operation_start=begin;if(++tokens>1000000||++work->tokens>8000000)fz_throw(ctx,FZ_ERROR_LIMIT,"token limit");
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
  if(!strcmp(lex.scratch,"Do")){if(!has_name||inside||active_path||pending_clip)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form call requires name and idle graphics/text state");preflight_form(ctx,font_name,used,work,operation_start,(size_t)fz_tell(ctx,stream));}
  if(!strcmp(lex.scratch,"Tf")&&(!has_name||absent(ctx,scoped_resource(ctx,"Font",font_name))))fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing explicit font binding");
  if(!strcmp(lex.scratch,"Tf")){pdf_obj *value=scoped_resource(ctx,"Font",font_name);char key[64];if(!pdf_is_indirect(ctx,value))fz_throw(ctx,FZ_ERROR_ARGUMENT,"direct font outline outside R22");snprintf(key,sizeof key,"%d:%d",pdf_to_num(ctx,value),pdf_to_gen(ctx,value));if(pdf_dict_len(ctx,used)>=256&&!pdf_dict_gets(ctx,used,key))fz_throw(ctx,FZ_ERROR_LIMIT,"used font bindings");pdf_dict_puts(ctx,used,key,value);}
  if(strcmp(lex.scratch,"Do"))origin(ctx,preflight->current,0,operation_start,(size_t)fz_tell(ctx,stream),lex.scratch);operation_start=SIZE_MAX;has_name=0;previous=t;
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
static void validate_form(fz_context *ctx,pdf_obj *obj)
{
 static const char *const forbidden[]={"Group","OC","Ref","Subtype2","OPI"};pdf_obj *v;size_t i;float a[6];fz_matrix m;fz_rect b;
 if(!pdf_is_indirect(ctx,obj)||!pdf_is_stream(ctx,obj)||!named(ctx,pdf_dict_get(ctx,obj,PDF_NAME(Subtype)),"Form"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"XObject is not an indirect ordinary Form");
 v=pdf_dict_get(ctx,obj,PDF_NAME(Type));if(!absent(ctx,v)&&!named(ctx,v,"XObject"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form Type");v=pdf_dict_gets(ctx,obj,"FormType");if(!absent(ctx,v)&&(!pdf_is_int(ctx,v)||pdf_to_int(ctx,v)!=1))fz_throw(ctx,FZ_ERROR_ARGUMENT,"FormType");
 for(i=0;i<sizeof(forbidden)/sizeof(forbidden[0]);i++)if(!absent(ctx,pdf_dict_gets(ctx,obj,forbidden[i])))fz_throw(ctx,FZ_ERROR_ARGUMENT,"unsupported Form metadata %s",forbidden[i]);
 v=pdf_dict_get(ctx,obj,PDF_NAME(BBox));if(!pdf_is_array(ctx,v)||pdf_array_len(ctx,v)!=4)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form BBox shape");for(i=0;i<4;i++){pdf_obj *x=pdf_array_get(ctx,v,(int)i);if(!pdf_is_number(ctx,x)||!isfinite(a[i]=pdf_to_real(ctx,x)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form BBox number");}if(a[0]>=a[2]||a[1]>=a[3])fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form BBox ordered nonempty");b=pdf_to_rect(ctx,v);(void)b;
 v=pdf_dict_get(ctx,obj,PDF_NAME(Matrix));if(!absent(ctx,v)){if(!pdf_is_array(ctx,v)||pdf_array_len(ctx,v)!=6)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form Matrix shape");for(i=0;i<6;i++){pdf_obj *x=pdf_array_get(ctx,v,(int)i);if(!pdf_is_number(ctx,x)||!isfinite(a[i]=pdf_to_real(ctx,x)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form Matrix number");}m=pdf_to_matrix(ctx,v);if(!isfinite(m.a*m.d-m.b*m.c)||m.a*m.d-m.b*m.c==0)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form singular Matrix");}
 v=pdf_dict_get(ctx,obj,PDF_NAME(Resources));if(!absent(ctx,v)&&!pdf_is_dict(ctx,v))fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form Resources dictionary");stream_shape(ctx,obj);
}
static void validate_image(fz_context *ctx,pdf_obj *obj)
{
 static const char *const excluded[]={"Mask","OC","Alternates","OPI","F","FFilter","FDecodeParms","Matte","Intent"};size_t i;int w,h;pdf_obj *v;
 if(!pdf_is_indirect(ctx,obj)||!pdf_is_stream(ctx,obj))fz_throw(ctx,FZ_ERROR_ARGUMENT,"indirect image stream required");
 w=pdf_dict_get_int(ctx,obj,PDF_NAME(Width));h=pdf_dict_get_int(ctx,obj,PDF_NAME(Height));
 if(w<=0||h<=0||w>16384||h>16384||(uint64_t)w*h>4194304||pdf_dict_get_int(ctx,obj,PDF_NAME(BitsPerComponent))!=8)fz_throw(ctx,FZ_ERROR_ARGUMENT,"image dimension/depth profile");
 if(!named(ctx,pdf_dict_get(ctx,obj,PDF_NAME(ColorSpace)),"DeviceGray")&&!named(ctx,pdf_dict_get(ctx,obj,PDF_NAME(ColorSpace)),"DeviceRGB"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"image device colorspace profile");
 for(i=0;i<sizeof(excluded)/sizeof(excluded[0]);i++)if(!absent(ctx,pdf_dict_gets(ctx,obj,excluded[i])))fz_throw(ctx,FZ_ERROR_ARGUMENT,"image excluded metadata %s",excluded[i]);
 v=pdf_dict_gets(ctx,obj,"Interpolate");if(!absent(ctx,v)&&(!pdf_is_bool(ctx,v)||pdf_to_bool(ctx,v)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"image interpolation excluded");
 v=pdf_dict_gets(ctx,obj,"ImageMask");if(!absent(ctx,v)&&(!pdf_is_bool(ctx,v)||pdf_to_bool(ctx,v)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"image mask excluded");
 v=pdf_dict_gets(ctx,obj,"SMaskInData");if(!absent(ctx,v)&&(!pdf_is_int(ctx,v)||pdf_to_int(ctx,v)))fz_throw(ctx,FZ_ERROR_ARGUMENT,"embedded image mask excluded");
 stream_shape(ctx,obj);
}
static void clip_opaque_mask(fz_context *ctx,fz_device *dev,fz_image *image,fz_matrix m,fz_rect scissor)
{
 trace_device *d=(trace_device*)dev;image_call *call;pdf_obj *ref=NULL,*mask;fz_image *base=NULL;fz_pixmap *pix=NULL;int w,h,n,x,y;(void)scissor;
 if(d->depth>=1024||preflight->image_cursor>=preflight->image_count)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unexpected mask clip");
 call=&preflight->images[preflight->image_cursor];fz_var(ref);fz_var(base);fz_var(pix);
 fz_try(ctx){
  ref=pdf_new_indirect(ctx,preflight->doc,call->object,call->generation);mask=pdf_dict_get(ctx,ref,PDF_NAME(SMask));
  if(!pdf_is_indirect(ctx,mask)||!pdf_is_stream(ctx,mask)||!named(ctx,pdf_dict_get(ctx,mask,PDF_NAME(ColorSpace)),"DeviceGray"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"original Gray SMask edge required");
  validate_image(ctx,mask);
  if(!absent(ctx,pdf_dict_gets(ctx,mask,"SMask"))||!absent(ctx,pdf_dict_gets(ctx,mask,"TR"))||!absent(ctx,pdf_dict_gets(ctx,mask,"SMaskInData")))fz_throw(ctx,FZ_ERROR_ARGUMENT,"mask role semantics");
  base=pdf_load_image(ctx,preflight->doc,ref);
  if(base->mask!=image||image->mask||image->interpolate||image->w!=base->w||image->h!=base->h)fz_throw(ctx,FZ_ERROR_ARGUMENT,"attached mask identity/shape");
  pix=fz_get_unscaled_pixmap_from_image(ctx,image);w=fz_pixmap_width(ctx,pix);h=fz_pixmap_height(ctx,pix);n=fz_pixmap_components(ctx,pix);
  if(w!=image->w||h!=image->h||n!=1||fz_pixmap_spots(ctx,pix))fz_throw(ctx,FZ_ERROR_ARGUMENT,"attached alpha layout");
  if((size_t)w*h>TOTAL_DECODED_LIMIT-d->work->decoded)fz_throw(ctx,FZ_ERROR_LIMIT,"mask decoded bound");d->work->decoded+=(size_t)w*h;
  event_start(ctx,d,"opaque_mask_clip");emit(ctx,d,",\"base_object\":[%d,%d],\"mask_object\":[%d,%d],\"width\":%d,\"height\":%d,\"matrix\":",call->object,call->generation,pdf_to_num(ctx,mask),pdf_to_gen(ctx,mask),w,h);matrix(ctx,d,m);emit(ctx,d,",\"alpha_hex\":\"");
  for(y=0;y<h;y++)for(x=0;x<w;x++){unsigned char a=fz_pixmap_samples(ctx,pix)[(size_t)y*fz_pixmap_stride(ctx,pix)+x];if(a!=255)fz_throw(ctx,FZ_ERROR_ARGUMENT,"nonopaque attached alpha");emit(ctx,d,"%02x",a);}emit(ctx,d,"\"}");d->clips[d->depth++]=d->events-1;
 }fz_always(ctx){fz_drop_pixmap(ctx,pix);fz_drop_image(ctx,base);pdf_drop_obj(ctx,ref);}fz_catch(ctx){fz_rethrow(ctx);}
}
static void fill_image(fz_context *ctx,fz_device *dev,fz_image *image,fz_matrix m,float alpha,fz_color_params params)
{
 trace_device *d=(trace_device*)dev;fz_pixmap *pix=NULL;image_call *call;int w,h,n,c,x,y;size_t size;(void)params;
 if(alpha!=1||image->imagemask||image->interpolate)fz_throw(ctx,FZ_ERROR_ARGUMENT,"image callback profile");
 if(preflight->image_cursor>=preflight->image_count)fz_throw(ctx,FZ_ERROR_ARGUMENT,"unexpected image callback");call=&preflight->images[preflight->image_cursor++];
 fz_var(pix);fz_try(ctx){unsigned char *samples;pix=fz_get_unscaled_pixmap_from_image(ctx,image);w=fz_pixmap_width(ctx,pix);h=fz_pixmap_height(ctx,pix);n=fz_pixmap_components(ctx,pix);c=n-fz_pixmap_alpha(ctx,pix);
 if(w!=image->w||h!=image->h||w<=0||h<=0||(c!=1&&c!=3)||fz_pixmap_spots(ctx,pix))fz_throw(ctx,FZ_ERROR_ARGUMENT,"native image decoded layout");size=(size_t)w*h*c;if(size>TOTAL_DECODED_LIMIT-d->work->decoded)fz_throw(ctx,FZ_ERROR_LIMIT,"image decoded work");d->work->decoded+=size;samples=fz_pixmap_samples(ctx,pix);
 event_start(ctx,d,"image");emit(ctx,d,",\"object\":[%d,%d],\"operation_index\":%zu,\"width\":%d,\"height\":%d,\"components\":%d,\"matrix\":",call->object,call->generation,call->origin,w,h,c);matrix(ctx,d,m);emit(ctx,d,",\"top_down\":true,\"samples_hex\":\"");
 for(y=0;y<h;y++)for(x=0;x<w;x++){unsigned char *p=samples+(size_t)y*fz_pixmap_stride(ctx,pix)+(size_t)x*n;int k;if(n!=c&&p[n-1]!=255)fz_throw(ctx,FZ_ERROR_ARGUMENT,"image nonopaque alpha");for(k=0;k<c;k++)emit(ctx,d,"%02x",p[k]);}emit(ctx,d,"\"}");
 }fz_always(ctx){fz_drop_pixmap(ctx,pix);}fz_catch(ctx){fz_rethrow(ctx);}
}
static void preflight_form(fz_context *ctx,const char *name,pdf_obj *used,work_budget *work,size_t start,size_t end)
{
 pdf_obj *obj=scoped_resource(ctx,"XObject",name),*res;fz_stream *stream=NULL;fz_buffer *buffer=NULL;int i,id,parent=preflight->current,invocation=(int)preflight->invocations;unsigned char *bytes;size_t length;
 if(named(ctx,pdf_dict_get(ctx,obj,PDF_NAME(Subtype)),"Image")){image_call *call;validate_image(ctx,obj);if(preflight->image_count>=4096)fz_throw(ctx,FZ_ERROR_LIMIT,"image call count");call=&preflight->images[preflight->image_count++];call->object=pdf_to_num(ctx,obj);call->generation=pdf_to_gen(ctx,obj);call->origin=preflight->origin_count;origin(ctx,parent,0,start,end,"Do");return;}
 if(++preflight->invocations>4096||preflight->depth>=32)fz_throw(ctx,FZ_ERROR_LIMIT,"Form invocation/depth limit");validate_form(ctx,obj);id=pdf_to_num(ctx,obj);for(i=1;i<=preflight->depth;i++)if(preflight->objects[i]==id)fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form ancestry cycle");
 res=pdf_dict_get(ctx,obj,PDF_NAME(Resources));if(absent(ctx,res))res=preflight->resources[preflight->depth];
 {pdf_obj *cs=pdf_dict_get(ctx,res,PDF_NAME(ColorSpace));if(!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultGray"))||!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultRGB"))||!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultCMYK")))fz_throw(ctx,FZ_ERROR_ARGUMENT,"Form default device override");}
 fz_var(stream);fz_var(buffer);fz_try(ctx){stream=pdf_open_stream(ctx,obj);buffer=read_bounded(ctx,stream,work);length=fz_buffer_storage(ctx,buffer,&bytes);origin(ctx,invocation,1,start,end,"Do");preflight->current=invocation;preflight->depth++;preflight->objects[preflight->depth]=id;preflight->resources[preflight->depth]=res;fz_try(ctx){lexical(ctx,bytes,length,0,NULL,used,work);}fz_always(ctx){preflight->depth--;preflight->current=parent;}fz_catch(ctx){fz_rethrow(ctx);}origin(ctx,invocation,2,start,end,"Do");}
 fz_always(ctx){fz_drop_stream(ctx,stream);fz_drop_buffer(ctx,buffer);}fz_catch(ctx){fz_rethrow(ctx);}
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
typedef struct {fz_buffer *source;int object,generation,parent,caller_provider,effective_provider;char name[128],source_sha256[65];fz_matrix matrix,ctm;fz_rect bbox;size_t begin,end;int clip_begin,clip_end;} form_record;
typedef struct {trace_device *device;form_record records[4096];size_t count;int parent,provider,stack_top;fz_matrix current,stack[256];void (*form)(fz_context*,pdf_processor*,const char*,pdf_obj*);void (*q)(fz_context*,pdf_processor*);void (*Q)(fz_context*,pdf_processor*);void (*cm)(fz_context*,pdf_processor*,float,float,float,float,float,float);} form_hooks;
static form_hooks *hooks;
static void hooked_q(fz_context *ctx,pdf_processor *proc){if(hooks->stack_top>=256)fz_throw(ctx,FZ_ERROR_LIMIT,"native matrix stack");hooks->stack[hooks->stack_top++]=hooks->current;hooks->q(ctx,proc);}
static void hooked_Q(fz_context *ctx,pdf_processor *proc){if(hooks->stack_top<=0)fz_throw(ctx,FZ_ERROR_ARGUMENT,"native tracked Q underflow");hooks->Q(ctx,proc);hooks->current=hooks->stack[--hooks->stack_top];}
static void hooked_cm(fz_context *ctx,pdf_processor *proc,float a,float b,float c,float d,float e,float f){hooks->cm(ctx,proc,a,b,c,d,e,f);hooks->current=fz_concat((fz_matrix){a,b,c,d,e,f},hooks->current);}
static void hooked_form(fz_context *ctx,pdf_processor *proc,const char *name,pdf_obj *obj)
{
 size_t index=hooks->count;form_record *r;int parent=hooks->parent,provider=hooks->provider,top=hooks->stack_top;fz_matrix old=hooks->current;fz_buffer *buffer=NULL;fz_stream *stream=NULL;
 if(index>=4096||strlen(name)>=128)fz_throw(ctx,FZ_ERROR_LIMIT,"native invocation bound");r=&hooks->records[hooks->count++];r->object=pdf_to_num(ctx,obj);r->generation=pdf_to_gen(ctx,obj);r->parent=parent;r->caller_provider=provider;r->effective_provider=absent(ctx,pdf_dict_get(ctx,obj,PDF_NAME(Resources)))?provider:r->object;strcpy(r->name,name);r->matrix=pdf_xobject_matrix(ctx,obj);r->bbox=pdf_xobject_bbox(ctx,obj);r->ctm=fz_concat(r->matrix,old);r->begin=hooks->device->events;r->clip_begin=hooks->device->depth;
 fz_var(buffer);fz_var(stream);fz_try(ctx){unsigned char *bytes,digest[32];size_t length;fz_sha256 hash;int i;stream=pdf_open_stream(ctx,obj);buffer=read_bounded(ctx,stream,hooks->device->work);length=fz_buffer_storage(ctx,buffer,&bytes);r->source=fz_keep_buffer(ctx,buffer);fz_sha256_init(&hash);fz_sha256_update(&hash,bytes,length);fz_sha256_final(&hash,digest);for(i=0;i<32;i++)snprintf(r->source_sha256+2*i,3,"%02x",digest[i]);hooks->parent=(int)index;hooks->provider=r->effective_provider;hooks->current=r->ctm;hooks->form(ctx,proc,name,obj);r->end=hooks->device->events;r->clip_end=hooks->device->depth;if(hooks->stack_top!=top||r->clip_begin!=r->clip_end)fz_throw(ctx,FZ_ERROR_ARGUMENT,"native Form state restoration");}
 fz_always(ctx){hooks->parent=parent;hooks->provider=provider;hooks->current=old;hooks->stack_top=top;fz_drop_stream(ctx,stream);fz_drop_buffer(ctx,buffer);}fz_catch(ctx){fz_rethrow(ctx);}
}
static void emit_forms(fz_context *ctx,trace_device *d)
{size_t i;emit(ctx,d,",\"form_invocations\":[");for(i=0;i<hooks->count;i++){form_record *r=&hooks->records[i];emit(ctx,d,"%s{\"id\":%zu,\"parent\":",i?",":"",i);if(r->parent<0)emit(ctx,d,"null");else emit(ctx,d,"%d",r->parent);emit(ctx,d,",\"object\":[%d,%d],\"caller_provider\":%d,\"effective_provider\":%d,\"name\":",r->object,r->generation,r->caller_provider,r->effective_provider);quote(ctx,d,r->name);emit(ctx,d,",\"matrix\":");matrix(ctx,d,r->matrix);emit(ctx,d,",\"ctm\":");matrix(ctx,d,r->ctm);emit(ctx,d,",\"bbox\":[%.9g,%.9g,%.9g,%.9g],\"event_range\":[%zu,%zu],\"clip_depth\":%d,\"source_sha256\":\"%s\"",r->bbox.x0,r->bbox.y0,r->bbox.x1,r->bbox.y1,r->begin,r->end,r->clip_begin,r->source_sha256);{unsigned char *bytes;size_t length=fz_buffer_storage(ctx,r->source,&bytes);emit(ctx,d,",\"source_hex\":");hex_bytes(ctx,d,bytes,length);}emit(ctx,d,"}");}emit(ctx,d,"]");}
static void emit_origins(fz_context *ctx,trace_device *d)
{size_t i;emit(ctx,d,",\"operation_origins\":[");for(i=0;i<preflight->origin_count;i++){native_origin *r=&preflight->origins[i];emit(ctx,d,"%s{\"object\":%d,\"invocation\":",i?",":"",r->object);if(r->invocation<0)emit(ctx,d,"null");else emit(ctx,d,"%d",r->invocation);emit(ctx,d,",\"phase\":%d,\"span\":[%zu,%zu],\"op\":\"%s\"}",r->phase,r->start,r->end,r->op);}emit(ctx,d,"]");}
static void page_trace(fz_context *ctx,pdf_document *doc,int index,trace_device *d,work_budget *work)
{
 form_preflight pf={0};form_hooks *fh=NULL;pdf_page *page=NULL;pdf_obj *used=NULL;fz_buffer *content=NULL;fz_stream *stream=NULL;pdf_processor *proc=NULL;fz_var(page);fz_var(used);fz_var(content);fz_var(stream);fz_var(proc);fz_var(fh);fz_var(pf);
 fz_try(ctx){pdf_obj *res,*fonts,*contents,*cs;fz_rect crop,bounds;fz_matrix transform;unsigned char *bytes;size_t length;int i,n;
  page=pdf_load_page(ctx,doc,index);res=pdf_page_resources(ctx,page);fonts=pdf_dict_get(ctx,res,PDF_NAME(Font));contents=pdf_page_contents(ctx,page);
  if(!absent(ctx,pdf_dict_get(ctx,page->obj,PDF_NAME(Group))))fz_throw(ctx,FZ_ERROR_ARGUMENT,"page transparency Group outside PP1");cs=pdf_dict_get(ctx,res,PDF_NAME(ColorSpace));if(!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultGray"))||!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultRGB"))||!absent(ctx,pdf_dict_gets(ctx,cs,"DefaultCMYK")))fz_throw(ctx,FZ_ERROR_ARGUMENT,"default device color override outside PP1");
  if(!absent(ctx,contents)){n=pdf_is_array(ctx,contents)?pdf_array_len(ctx,contents):1;if(n>10000)fz_throw(ctx,FZ_ERROR_LIMIT,"content stream occurrences");for(i=0;i<n;i++)stream_shape(ctx,pdf_is_array(ctx,contents)?pdf_array_get(ctx,contents,i):contents);stream=pdf_open_contents_stream(ctx,doc,contents);content=read_bounded(ctx,stream,work);}else content=fz_new_buffer(ctx,0);
  pf.doc=doc;pf.current=-1;pf.objects[0]=pdf_to_num(ctx,page->obj);pf.resources[0]=res;preflight=&pf;length=fz_buffer_storage(ctx,content,&bytes);used=pdf_new_dict(ctx,doc,4);lexical(ctx,bytes,length,0,fonts,used,work);resources(ctx,used,work);
  for(i=0;i<pdf_dict_len(ctx,used);i++){pdf_obj *value=pdf_dict_get_val(ctx,used,i),*program=pdf_dict_get(ctx,pdf_dict_get(ctx,value,PDF_NAME(FontDescriptor)),PDF_NAME(FontFile2));native_font_binding *binding;if(d->font_count>=256)fz_throw(ctx,FZ_ERROR_LIMIT,"font bindings");binding=&d->fonts[d->font_count];binding->object=pdf_to_num(ctx,value);binding->generation=pdf_to_gen(ctx,value);binding->program=pdf_to_num(ctx,program);binding->program_generation=pdf_to_gen(ctx,program);binding->desc=pdf_load_font(ctx,doc,NULL,value);d->font_count++;if(!binding->desc->is_embedded&&pdf_name_eq(ctx,pdf_dict_get(ctx,value,PDF_NAME(Subtype)),PDF_NAME(Type1))){const char *face=pdf_to_name(ctx,pdf_dict_get(ctx,value,PDF_NAME(BaseFont)));pdf_obj *fd=pdf_dict_get(ctx,value,PDF_NAME(FontDescriptor));if((!strcmp(face,"Helvetica")||!strcmp(face,"Times-Roman"))&&absent(ctx,pdf_dict_get(ctx,fd,PDF_NAME(FontFile)))&&absent(ctx,pdf_dict_get(ctx,fd,PDF_NAME(FontFile2)))&&absent(ctx,pdf_dict_get(ctx,fd,PDF_NAME(FontFile3)))){unsigned char *font_bytes,digest[32];char hex[65];size_t size=fz_buffer_storage(ctx,binding->desc->font->buffer,&font_bytes);fz_sha256 hash;int k;binding->builtin=!strcmp(face,"Helvetica")?1:2;fz_sha256_init(&hash);fz_sha256_update(&hash,font_bytes,size);fz_sha256_final(&hash,digest);for(k=0;k<32;k++)snprintf(hex+2*k,3,"%02x",digest[k]);if(strcmp(hex,binding->builtin==1?"366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706":"eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c"))fz_throw(ctx,FZ_ERROR_ARGUMENT,"native builtin asset mismatch");}}}
  pdf_page_transform(ctx,page,&crop,&transform);bounds=fz_transform_rect(crop,transform);if(!isfinite(bounds.x0)||!isfinite(bounds.y0)||!isfinite(bounds.x1)||!isfinite(bounds.y1)||bounds.x0>=bounds.x1||bounds.y0>=bounds.y1)fz_throw(ctx,FZ_ERROR_ARGUMENT,"page geometry");
  emit(ctx,d,"{\"page\":%d,\"status\":\"accepted\",\"advance_policy\":\"allow_pinned_base14\",\"initial_ctm\":",index+1);matrix(ctx,d,transform);emit(ctx,d,",\"initial_clip\":[");number(ctx,d,bounds.x0);emit(ctx,d,",");number(ctx,d,bounds.y0);emit(ctx,d,",");number(ctx,d,bounds.x1);emit(ctx,d,",");number(ctx,d,bounds.y1);emit(ctx,d,"],\"content_hex\":");hex_bytes(ctx,d,bytes,length);emit(ctx,d,",\"events\":[");
  proc=pdf_new_run_processor(ctx,doc,&d->super,transform,-1,"View",NULL,NULL,NULL,NULL,NULL);fh=fz_malloc_struct(ctx,form_hooks);memset(fh,0,sizeof(*fh));hooks=fh;fh->device=d;fh->parent=-1;{pdf_obj *provider=page->obj;int depth=0;while(absent(ctx,pdf_dict_get(ctx,provider,PDF_NAME(Resources)))){if(++depth>64)fz_throw(ctx,FZ_ERROR_LIMIT,"page resource provider ancestry");provider=pdf_dict_get(ctx,provider,PDF_NAME(Parent));if(absent(ctx,provider))break;}fh->provider=absent(ctx,provider)?pdf_to_num(ctx,page->obj):pdf_to_num(ctx,provider);}fh->current=transform;fh->form=proc->op_Do_form;fh->q=proc->op_q;fh->Q=proc->op_Q;fh->cm=proc->op_cm;proc->op_Do_form=hooked_form;proc->op_q=hooked_q;proc->op_Q=hooked_Q;proc->op_cm=hooked_cm;pdf_process_glyph(ctx,proc,doc,res,content);if(pf.image_cursor!=pf.image_count)fz_throw(ctx,FZ_ERROR_ARGUMENT,"missing image callbacks");d->recording=0;pdf_close_processor(ctx,proc);fz_close_device(ctx,&d->super);emit_contexts(ctx,d);emit_forms(ctx,d);emit_origins(ctx,d);emit(ctx,d,",\"text_callbacks\":%zu,\"cleanup_pops\":%zu}",d->callbacks,d->cleanup_pops);
 }fz_always(ctx){/* Device exceptions disable callbacks; closing also prevents cleanup from tracing aborted work. */if(proc && !proc->closed){fz_close_device(ctx,&d->super);pdf_close_processor(ctx,proc);}pdf_drop_processor(ctx,proc);if(fh){size_t k;for(k=0;k<fh->count;k++)fz_drop_buffer(ctx,fh->records[k].source);}fz_free(ctx,fh);fz_free(ctx,pf.origins);hooks=NULL;preflight=NULL;fz_drop_stream(ctx,stream);fz_drop_buffer(ctx,content);pdf_drop_obj(ctx,used);fz_drop_page(ctx,(fz_page*)page);}fz_catch(ctx){fz_rethrow(ctx);}
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
   fz_try(ctx){d=fz_new_derived_device(ctx,trace_device);d->out=page_output;d->work=&work;d->recording=1;d->super.fill_image=fill_image;d->super.clip_image_mask=clip_opaque_mask;d->super.fill_path=fill;d->super.stroke_path=stroke;d->super.clip_path=clip;d->super.pop_clip=pop;d->super.fill_text=fill_text;d->super.ignore_text=ignore_text;d->super.drop_device=drop_trace;
    fz_try(ctx){page_trace(ctx,doc,index,d,&work);accepted=1;}fz_catch(ctx){if(fz_caught(ctx)!=FZ_ERROR_ARGUMENT)fz_rethrow(ctx);snprintf(refusal,sizeof(refusal),"%s",fz_caught_message(ctx));fz_ignore_error(ctx);}
    fz_flush_warnings(ctx);if(raw->error||pdf_was_repaired(ctx,doc)||diagnostics)fz_throw(ctx,FZ_ERROR_FORMAT,"native warning/error/repair forbidden");if(pages++)emit(ctx,&envelope,",");
    if(accepted){if(d->bytes>OUTPUT_LIMIT-envelope.bytes)fz_throw(ctx,FZ_ERROR_LIMIT,"document output budget");if(fflush(page_output)||fseek(page_output,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"page trace seek");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),page_output);if(n&&fwrite(chunk,1,n,output)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"page trace copy");if(n<sizeof(chunk)){if(ferror(page_output))fz_throw(ctx,FZ_ERROR_SYSTEM,"page trace read");break;}}envelope.bytes+=d->bytes;}
    else {emit(ctx,&envelope,"{\"page\":%d,\"status\":\"refused\",\"error\":",index+1);quote(ctx,&envelope,refusal);emit(ctx,&envelope,"}");}
   }fz_always(ctx){fz_drop_device(ctx,(fz_device*)d);fclose(page_output);}fz_catch(ctx){fz_rethrow(ctx);}
  }
  emit(ctx,&envelope,"],\"repaired\":false,\"work\":{\"tokens\":%zu,\"decoded_bytes\":%zu,\"font_preparations\":%zu,\"items\":%zu,\"commands\":%zu,\"events\":%zu}}\n",work.tokens,work.decoded,work.font_preparations,work.items,work.commands,work.events);
  if(fflush(output)||fseek(output,0,SEEK_SET))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace seek I/O");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),output);if(n&&fwrite(chunk,1,n,stdout)!=n)fz_throw(ctx,FZ_ERROR_SYSTEM,"stdout I/O");if(n<sizeof(chunk)){if(ferror(output))fz_throw(ctx,FZ_ERROR_SYSTEM,"trace read I/O");break;}}if(fflush(stdout))status=1;
 }fz_always(ctx){pdf_drop_document(ctx,doc);fz_drop_stream(ctx,raw);}fz_catch(ctx){fprintf(stderr,"image-trace oracle: %s\n",fz_caught_message(ctx));fz_ignore_error(ctx);status=2;}fz_drop_context(ctx);fclose(output);free(bytes);return status;
}
