/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Development-only canonical outline extraction. FreeType is not a runtime
 * dependency. Input assets are hash-checked by generate-substitute-outlines.py.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_OUTLINE_H
static unsigned total, count;
static int opened, first;
static void command(const char *op, const FT_Vector *a, const FT_Vector *b, const FT_Vector *c) {
    const FT_Vector *points[]={a,b,c};
    if(++count>4096 || ++total>1000000) exit(2);
    printf("%s[\"%s\"",first?"":",",op);first=0;
    for(int i=0;i<3;i++) if(points[i]) {
        if(labs(points[i]->x)>1000000 || labs(points[i]->y)>1000000) exit(2);
        printf(",%ld,%ld",points[i]->x,points[i]->y);
    }
    printf("]");
}
static int move(const FT_Vector *a,void *p){(void)p;if(opened)command("h",NULL,NULL,NULL);command("m",a,NULL,NULL);opened=1;return 0;}
static int line(const FT_Vector *a,void *p){(void)p;command("l",a,NULL,NULL);return 0;}
static int quad(const FT_Vector *a,const FT_Vector *b,void *p){(void)a;(void)b;(void)p;return 1;}
static int cubic(const FT_Vector *a,const FT_Vector *b,const FT_Vector *c,void *p){(void)p;command("c",a,b,c);return 0;}
int main(int argc,char **argv){
    FT_Library lib;FT_Face face;FT_Outline_Funcs funcs={move,line,quad,cubic,0,0};
    if(argc!=2 || FT_Init_FreeType(&lib) || FT_New_Face(lib,argv[1],0,&face))return 1;
    if(face->units_per_EM!=1000 || face->num_glyphs!=654)return 2;
    printf("[");
    for(FT_UInt gid=0;gid<(FT_UInt)face->num_glyphs;gid++){
        char name[128];
        if(FT_Get_Glyph_Name(face,gid,name,sizeof name))return 2;
        for(size_t i=0;i<strlen(name);i++) if(name[i]<33 || name[i]>126 || name[i]=='"' || name[i]=='\\')return 2;
        if(FT_Load_Glyph(face,gid,FT_LOAD_NO_SCALE|FT_LOAD_NO_HINTING|FT_LOAD_NO_AUTOHINT|FT_LOAD_NO_BITMAP|FT_LOAD_IGNORE_TRANSFORM))return 2;
        if(face->glyph->format!=FT_GLYPH_FORMAT_OUTLINE)return 2;
        printf("%s{\"name\":\"%s\",\"gid\":%u,\"advance\":%ld,\"commands\":[",gid?",":"",name,gid,face->glyph->advance.x);
        count=0;first=1;opened=0;
        if(FT_Outline_Decompose(&face->glyph->outline,&funcs,NULL))return 2;
        if(opened)command("h",NULL,NULL,NULL);
        printf("]}");
    }
    printf("]\n");FT_Done_Face(face);FT_Done_FreeType(lib);
    return ferror(stdout)?2:0;
}
