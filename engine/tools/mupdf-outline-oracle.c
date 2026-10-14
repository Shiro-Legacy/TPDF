/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only exact unhinted simple-glyf oracle, linked to pinned bundled FreeType.
 * Raw sfnt stdin; argv[1] is all or a decimal GID query file. No system fonts.
 * Font32MiB/query1MiB/native heap128MiB/output64MiB; CPU10s/wall15s.
 * Geometry is raw font units, then x2 exact quadratic commands. Atomic output.
 */
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_OUTLINE_H
#include FT_MODULE_H
#include FT_SYSTEM_H
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <limits.h>
#include <sys/resource.h>
#include <unistd.h>
#define FONT_LIMIT (32u*1024u*1024u)
#define QUERY_LIMIT (1024u*1024u)
#define HEAP_LIMIT (128u*1024u*1024u)
#define OUTPUT_LIMIT (64u*1024u*1024u)
#define QUERY_COUNT 65536u
static void fail(const char *message) { fprintf(stderr,"outline oracle: %s\n",message);exit(2); }
typedef union { size_t size; max_align_t alignment; } header;
typedef struct {size_t used;} budget;
static void *allocate(FT_Memory memory,long amount) {
 budget *b=memory->user;header *h;size_t n;if(amount<=0)return NULL;n=(size_t)amount;
 if(n>HEAP_LIMIT-b->used||n>SIZE_MAX-sizeof(*h))return NULL;h=malloc(sizeof(*h)+n);if(!h)return NULL;h->size=n;b->used+=n;return h+1;
}
static void release(FT_Memory memory,void *p) {if(p){budget *b=memory->user;header *h=(header*)p-1;b->used-=h->size;free(h);}}
static void *resize(FT_Memory memory,long old_size,long new_size,void *p) {
 budget *b=memory->user;header *h,*next;size_t n,old;(void)old_size;if(!p)return allocate(memory,new_size);if(new_size<=0){release(memory,p);return NULL;}
 n=(size_t)new_size;h=(header*)p-1;old=h->size;if(n>HEAP_LIMIT-(b->used-old)||n>SIZE_MAX-sizeof(*h))return NULL;next=realloc(h,sizeof(*h)+n);if(!next)return NULL;next->size=n;b->used=b->used-old+n;return next+1;
}
typedef struct {const unsigned char *data;size_t size,base;} slice;
static slice sub(slice s,size_t start,size_t length){slice r;if(start>s.size||length>s.size-start)fail("truncated sfnt range");r.data=s.data+start;r.size=length;r.base=s.base+start;return r;}
static unsigned u16(slice s,size_t p){slice r=sub(s,p,2);return (unsigned)r.data[0]*256+r.data[1];}
static int i16(slice s,size_t p){unsigned n=u16(s,p);return n<32768?(int)n:(int)n-65536;}
static uint32_t u32(slice s,size_t p){slice r=sub(s,p,4);return ((uint32_t)r.data[0]<<24)|((uint32_t)r.data[1]<<16)|((uint32_t)r.data[2]<<8)|r.data[3];}
static slice table(slice s,const char *tag,int required){unsigned n=u16(s,4),i;int found=0;slice result={NULL,0,0};if(n>256)fail("table count limit");sub(s,0,12+16*(size_t)n);for(i=0;i<n;i++){size_t p=12+16*(size_t)i;if(!memcmp(s.data+p,tag,4)){if(found++)fail("duplicate queried table");result=sub(s,u32(s,p+8),u32(s,p+12));}}if(required&&!found)fail("missing required native table");return result;}
typedef struct {FILE *file;size_t bytes,points,commands,glyph_bytes;int first,open;} output;
static void emit(output *out,const char *format,...){char buffer[1024];int n;va_list a;va_start(a,format);n=vsnprintf(buffer,sizeof(buffer),format,a);va_end(a);if(n<0||(size_t)n>=sizeof(buffer)||(size_t)n>OUTPUT_LIMIT-out->bytes)fail("output limit");if(fwrite(buffer,1,(size_t)n,out->file)!=(size_t)n)fail("output I/O");out->bytes+=(size_t)n;}
static void command(output *out,const char *name,const FT_Vector *a,const FT_Vector *b){if(++out->commands>2097152)fail("command count limit");emit(out,"%s[\"%s\"",out->first?"":",",name);out->first=0;if(a)emit(out,",%ld,%ld",a->x,a->y);if(b)emit(out,",%ld,%ld",b->x,b->y);emit(out,"]");}
static int move_to(const FT_Vector *p,void *opaque){output *out=opaque;if(out->open)command(out,"h",NULL,NULL);command(out,"m",p,NULL);out->open=1;return 0;}
static int line_to(const FT_Vector *p,void *opaque){command(opaque,"l",p,NULL);return 0;}
static int conic_to(const FT_Vector *a,const FT_Vector *b,void *opaque){command(opaque,"q",a,b);return 0;}
static int cubic_to(const FT_Vector *a,const FT_Vector *b,const FT_Vector *c,void *opaque){(void)a;(void)b;(void)c;(void)opaque;fail("unexpected cubic outline");return 1;}
static void ft(FT_Error e,const char *what){if(e){fprintf(stderr,"outline oracle: %s (FreeType error %d)\n",what,e);exit(2);}}
static uint32_t *queries(const char *path,unsigned count,size_t *length){
 uint32_t *ids=malloc(QUERY_COUNT*sizeof(*ids));unsigned char *bytes;size_t n,i=0;FILE *f;if(!ids)fail("query allocation");*length=0;
 if(!strcmp(path,"all")){for(i=0;i<count;i++)ids[(*length)++]=(uint32_t)i;return ids;}
 f=fopen(path,"rb");if(!f)fail("query file open");bytes=malloc(QUERY_LIMIT+1);if(!bytes)fail("query byte allocation");n=fread(bytes,1,QUERY_LIMIT+1,f);if(ferror(f)||n>QUERY_LIMIT)fail("query input limit");fclose(f);
 while(i<n){size_t end=i,start;uint32_t id=0;while(end<n&&bytes[end]!='\n')end++;start=i;while(start<end&&(bytes[start]==' '||bytes[start]=='\t'||bytes[start]=='\r'))start++;while(end>start&&(bytes[end-1]==' '||bytes[end-1]=='\t'||bytes[end-1]=='\r'))end--;if(start<end){size_t j;for(j=start;j<end;j++){unsigned d;if(bytes[j]<'0'||bytes[j]>'9')fail("query requires decimal GID");d=bytes[j]-'0';if(id>(UINT32_MAX-d)/10)fail("query GID overflow");id=id*10+d;}if(*length>=QUERY_COUNT)fail("query count limit");if(id>=count)fail("query GID out of range");ids[(*length)++]=id;}while(i<n&&bytes[i]!='\n')i++;if(i<n)i++;}
 free(bytes);return ids;
}
int main(int argc,char **argv){
 unsigned char *bytes;size_t size,nqueries,qi;uint32_t *ids;FT_Library library;FT_Face face;budget heap={0};struct FT_MemoryRec_ memory={&heap,allocate,release,resize};output out={0};struct rlimit cpu={10,10};slice font,head,maxp,loca,glyf,hhea,hmtx;unsigned glyphs,units,format,metrics;
 setrlimit(RLIMIT_CPU,&cpu);alarm(15);if(argc!=2){fprintf(stderr,"usage: mupdf-outline-oracle all|QUERYFILE < FONT.ttf\n");return 1;}
 bytes=malloc(FONT_LIMIT+1);if(!bytes)fail("font allocation");size=fread(bytes,1,FONT_LIMIT+1,stdin);if(ferror(stdin)||size>FONT_LIMIT)fail("font input limit");font=(slice){bytes,size,0};if(u32(font,0)!=0x10000)fail("only standalone TrueType sfnt");
 head=table(font,"head",1);maxp=table(font,"maxp",1);loca=table(font,"loca",1);glyf=table(font,"glyf",1);hhea=table(font,"hhea",1);hmtx=table(font,"hmtx",1);
 {const char *tags[]={"fvar","gvar","avar","HVAR","VVAR","MVAR"};unsigned j;for(j=0;j<sizeof(tags)/sizeof(tags[0]);j++)if(table(font,tags[j],0).data)fail("variation table forbidden");}
 sub(head,0,54);sub(maxp,0,32);sub(hhea,0,36);if(u32(head,0)!=0x10000||u32(head,12)!=0x5f0f3cf5||u32(maxp,0)!=0x10000)fail("native metadata shape");glyphs=u16(maxp,4);units=u16(head,18);format=u16(head,50);metrics=u16(hhea,34);if(!glyphs||units<16||units>16384||format>1||!metrics||metrics>glyphs)fail("native metadata value");sub(loca,0,(glyphs+1)*(format?4u:2u));sub(hmtx,0,metrics*4u+(glyphs-metrics)*2u);
 ids=queries(argv[1],glyphs,&nqueries);out.file=tmpfile();if(!out.file)fail("temporary output");ft(FT_New_Library(&memory,&library),"new library");FT_Add_Default_Modules(library);ft(FT_New_Memory_Face(library,bytes,(FT_Long)size,0,&face),"open font");if(face->num_faces!=1||face->num_glyphs!=(FT_Long)glyphs||face->units_per_EM!=units)fail("native metadata disagreement");
 emit(&out,"{\"units_per_em\":%u,\"glyph_count\":%u,\"glyphs\":[",units,glyphs);
 for(qi=0;qi<nqueries;qi++){
  unsigned gid=ids[qi];size_t begin=format?u32(loca,gid*4u):u16(loca,gid*2u)*2u;size_t end=format?u32(loca,(gid+1)*4u):u16(loca,(gid+1)*2u)*2u;int count=0,adjust=0,lsb;const char *kind;FT_Outline *o;int i;slice body;
  if(end<begin)fail("nonmonotone selected loca");body=sub(glyf,begin,end-begin);if(body.size>(64u*1024u*1024u)-out.glyph_bytes)fail("cumulative glyph bytes");out.glyph_bytes+=body.size;
  kind=body.size?"simple":"empty_loca";if(body.size){sub(body,0,10);count=i16(body,0);if(count<0)fail("composite glyph forbidden");if(!count)kind="zero_contours";lsb=gid<metrics?i16(hmtx,gid*4u+2):i16(hmtx,metrics*4u+(gid-metrics)*2u);adjust=i16(body,2)-lsb;}
  ft(FT_Load_Glyph(face,gid,FT_LOAD_NO_SCALE|FT_LOAD_NO_HINTING|FT_LOAD_NO_AUTOHINT|FT_LOAD_NO_BITMAP|FT_LOAD_IGNORE_TRANSFORM),"load unhinted glyph");if(face->glyph->format!=FT_GLYPH_FORMAT_OUTLINE)fail("native non-outline glyph");o=&face->glyph->outline;if(o->n_points<0||o->n_contours<0||o->n_contours!=count)fail("native contour count mismatch");if((size_t)o->n_points>1048576-out.points)fail("point count limit");out.points+=(size_t)o->n_points;if(!body.size&&o->n_points)fail("native nonempty empty slot");
  emit(&out,"%s{\"gid\":%u,\"kind\":\"%s\",\"range\":[%zu,%zu],\"bearing_adjustment\":%d,\"points\":[",qi?",":"",gid,kind,glyf.base+begin,glyf.base+end,adjust);
  for(i=0;i<o->n_points;i++){FT_Vector *p=&o->points[i];int tag=FT_CURVE_TAG(o->tags[i]);if(tag!=FT_CURVE_TAG_ON&&tag!=FT_CURVE_TAG_CONIC)fail("native nonquadratic point tag");if(p->x<INT32_MIN||p->x>INT32_MAX||p->y<INT16_MIN||p->y>INT16_MAX)fail("native coordinate range");p->x+=adjust;if(p->x<INT16_MIN||p->x>INT16_MAX)fail("restored coordinate range");emit(&out,"%s[%ld,%ld,%s]",i?",":"",p->x,p->y,tag==FT_CURVE_TAG_ON?"true":"false");p->x*=2;p->y*=2;}
  emit(&out,"],\"contour_endpoints\":[");for(i=0;i<o->n_contours;i++){if(o->contours[i]<0||o->contours[i]>=o->n_points||(i&&o->contours[i]<=o->contours[i-1]))fail("native endpoint bounds");emit(&out,"%s%d",i?",":"",o->contours[i]);}emit(&out,"],\"commands\":[");out.first=1;out.open=0;
  {const FT_Outline_Funcs funcs={move_to,line_to,conic_to,cubic_to,0,0};ft(FT_Outline_Decompose(o,&funcs,&out),"decompose doubled outline");}if(out.open)command(&out,"h",NULL,NULL);emit(&out,"]}");
 }
 emit(&out,"]}\n");ft(FT_Done_Face(face),"drop face");ft(FT_Done_Library(library),"drop library");if(heap.used)fail("native allocator imbalance");if(fflush(out.file)||fseek(out.file,0,SEEK_SET))fail("output seek");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),out.file);if(n&&fwrite(chunk,1,n,stdout)!=n)fail("stdout I/O");if(n<sizeof(chunk)){if(ferror(out.file))fail("temporary read");break;}}if(fflush(stdout))fail("stdout flush");fclose(out.file);free(ids);free(bytes);return 0;
}
