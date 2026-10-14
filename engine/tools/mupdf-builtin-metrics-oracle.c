/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned builtin CFF glyph-name advance oracle. No PDF/system fonts.
 * Input: at most4096 printable ASCII glyph names, 1..127bytes each.
 * OutputTSV: name found gid upem rawAdvance pdfIntegerWidth.
 * Exact FT_Get_Name_Index is deliberate: MuPDF's public name encoder falls
 * back through Unicode on a missing name and is not a strict name oracle.
 */
#include <stdio.h>
#include <stdint.h>
#include <inttypes.h>
#include <string.h>
#include "mupdf/fitz.h"
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_ADVANCES_H
static int read_name(fz_context *ctx, char *line)
{
    size_t len=0;int ch;
    while((ch=fgetc(stdin))!=EOF && ch!='\n') {
        if(ch<33||ch>126||len>=127)fz_throw(ctx,FZ_ERROR_LIMIT,"glyph query syntax/length");
        line[len++]=(char)ch;
    }
    if(ch==EOF&&!len)return 0;
    if(!len)fz_throw(ctx,FZ_ERROR_FORMAT,"empty glyph query");
    line[len]=0;return 1;
}
int main(int argc, char **argv)
{
    fz_context *ctx;
    fz_font *font=NULL;
    int status=0;
    char line[130];
    unsigned queries=0;
    if(argc!=2 || (strcmp(argv[1],"Helvetica") && strcmp(argv[1],"Times-Roman"))) {
        fprintf(stderr,"usage: mupdf-builtin-metrics-oracle Helvetica|Times-Roman\n");return 1;
    }
    ctx=fz_new_context(NULL,NULL,FZ_STORE_UNLIMITED);
    if(!ctx)return 1;
    fz_var(font);fz_var(status);
    fz_try(ctx) {
        FT_Face face;
        font=fz_new_base14_font(ctx,argv[1]);
        face=(FT_Face)fz_font_ft_face(ctx,font);
        if(!face || !face->units_per_EM)fz_throw(ctx,FZ_ERROR_FORMAT,"missing pinned FreeType face/units");
        while(read_name(ctx,line)) {
            FT_UInt gid;
            FT_Fixed advance=0;
            char actual[128];
            int found,name_error,advance_error=0;
            if(++queries>4096)fz_throw(ctx,FZ_ERROR_LIMIT,"glyph query bounds");
            fz_ft_lock(ctx);
            gid=FT_Get_Name_Index(face,line);
            name_error=FT_Get_Glyph_Name(face,gid,actual,sizeof actual);
            found=!name_error&&!strcmp(actual,line);
            if(found)advance_error=FT_Get_Advance(face,gid,FT_LOAD_NO_SCALE|FT_LOAD_NO_HINTING|FT_LOAD_NO_BITMAP|FT_LOAD_IGNORE_TRANSFORM,&advance);
            fz_ft_unlock(ctx);
            if(name_error||advance_error)fz_throw(ctx,FZ_ERROR_FORMAT,"glyph name/advance read failed");
            if(printf("%s\t%d\t%u\t%u\t%ld\t%" PRId64 "\n",line,found,gid,face->units_per_EM,(long)advance,(int64_t)advance*1000/face->units_per_EM)<0)fz_throw(ctx,FZ_ERROR_SYSTEM,"output failed");
        }
        if(ferror(stdin))fz_throw(ctx,FZ_ERROR_SYSTEM,"input failed");
    }
    fz_catch(ctx) {fprintf(stderr,"%s\n",fz_caught_message(ctx));status=2;}
    fz_drop_font(ctx,font);fz_drop_context(ctx);
    if(fflush(stdout)||ferror(stdout))status=2;
    return status;
}
