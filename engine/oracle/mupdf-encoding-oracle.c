/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only fixed table oracle; never loads fonts, documents, or system CMaps.
 * TSV: encoding byte glyph-name-or-'-' strict-native-value loose-native-value.
 * Native strict zero means missing; loose may apply lossy name heuristics.
 */
#include <stdio.h>
#include "mupdf/fitz.h"
int main(void)
{
    const char **tables[] = {fz_glyph_name_from_win_ansi, fz_glyph_name_from_mac_roman,
        fz_glyph_name_from_adobe_standard, fz_glyph_name_from_mac_expert};
    const char *names[] = {"WinAnsiEncoding", "MacRomanEncoding", "StandardEncoding", "MacExpertEncoding"};
    unsigned int t, b;
    for (t = 0; t < 4; ++t)
        for (b = 0; b < 256; ++b) {
            const char *name = tables[t][b];
            if (printf("%s\t%u\t%s\t%d\t%d\n", names[t], b, name ? name : "-",
                name ? fz_unicode_from_glyph_name_strict(name) : 0,
                name ? fz_unicode_from_glyph_name(name) : 0) < 0) return 1;
        }
    return fflush(stdout) || ferror(stdout) ? 1 : 0;
}
