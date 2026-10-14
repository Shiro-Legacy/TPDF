/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only exhaustive cmap4 oracle, linked to pinned bundled FreeType.
 * Raw sfnt stdin; emits all65536 results per original format4 record.
 * Font32MiB/native heap128MiB/output64MiB; records128, CPU10s/wall15s.
 * Direct FT charmap query: no PDF encoding or symbol offset heuristics.
 */
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_TRUETYPE_TABLES_H
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
static uint32_t u32(slice s,size_t p){slice r=sub(s,p,4);return ((uint32_t)r.data[0]<<24)|((uint32_t)r.data[1]<<16)|((uint32_t)r.data[2]<<8)|r.data[3];}
static slice table(slice s,const char *tag,int required){unsigned n=u16(s,4),i;int found=0;slice result={NULL,0,0};if(n>256)fail("table count limit");sub(s,0,12+16*(size_t)n);for(i=0;i<n;i++){size_t p=12+16*(size_t)i;if(!memcmp(s.data+p,tag,4)){if(found++)fail("duplicate queried table");result=sub(s,u32(s,p+8),u32(s,p+12));}}if(required&&!found)fail("missing required native table");return result;}

static void ft(FT_Error error,const char *what){if(error){fprintf(stderr,"cmap4 oracle: %s (FreeType %d)\n",what,error);exit(2);}}
int main(int argc,char **argv){
 unsigned char *bytes;size_t length;FT_Library library;FT_Face face;budget heap={0};struct FT_MemoryRec_ memory={&heap,allocate,release,resize};FILE *out;struct rlimit cpu={10,10};slice font,cmap,maxp;unsigned records,i,glyphs;(void)argv;
 setrlimit(RLIMIT_CPU,&cpu);alarm(15);if(argc!=1)return 1;bytes=malloc(FONT_LIMIT+1);if(!bytes)fail("input allocation");length=fread(bytes,1,FONT_LIMIT+1,stdin);if(ferror(stdin)||length>FONT_LIMIT)fail("input limit");font=(slice){bytes,length,0};if(u32(font,0)!=0x10000)fail("standalone TrueType only");cmap=table(font,"cmap",1);maxp=table(font,"maxp",1);glyphs=u16(maxp,4);if(!glyphs||u16(cmap,0))fail("metadata");records=u16(cmap,2);if(!records||records>128)fail("record limit");sub(cmap,4,records*8u);
 ft(FT_New_Library(&memory,&library),"new library");FT_Add_Default_Modules(library);ft(FT_New_Memory_Face(library,bytes,(FT_Long)length,0,&face),"open font");if(face->num_faces!=1||face->num_glyphs!=(FT_Long)glyphs||face->num_charmaps!=(FT_Int)records)fail("native metadata/record count disagreement");out=tmpfile();if(!out)fail("temporary output");fprintf(out,"{\"glyph_count\":%u,\"records\":[",glyphs);
 for(i=0;i<records;i++){size_t p=4+i*8u;unsigned platform=u16(cmap,p),encoding=u16(cmap,p+2),off=u32(cmap,p+4),format=u16(cmap,off);FT_CharMap record=face->charmaps[i];unsigned code;
  if(format!=4||record->platform_id!=platform||record->encoding_id!=encoding||FT_Get_CMap_Format(record)!=4)fail("native original format4 record identity differs");ft(FT_Set_Charmap(face,record),"set charmap");fprintf(out,"%s{\"index\":%u,\"platform\":%u,\"encoding\":%u,\"format\":4,\"mappings\":[",i?",":"",i,platform,encoding);
  for(code=0;code<65536;code++){FT_UInt gid=FT_Get_Char_Index(face,code);if(gid>=glyphs)fail("native GID outside maxp");fprintf(out,"%s%u",code?",":"",gid);}fprintf(out,"]}");if(ftell(out)<0||ftell(out)>OUTPUT_LIMIT)fail("output limit");
 }
 fprintf(out,"]}\n");ft(FT_Done_Face(face),"drop face");ft(FT_Done_Library(library),"drop library");if(heap.used)fail("allocator imbalance");if(fflush(out)||fseek(out,0,SEEK_SET))fail("output seek");for(;;){unsigned char chunk[8192];size_t n=fread(chunk,1,sizeof(chunk),out);if(n&&fwrite(chunk,1,n,stdout)!=n)fail("output write");if(n<sizeof(chunk)){if(ferror(out))fail("output read");break;}}if(fflush(stdout))fail("output flush");fclose(out);free(bytes);return 0;
}
