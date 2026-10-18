/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only adapter for MuPDF 1.28.1. See ../NOTICE and ../COPYING.
 */
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "mupdf/fitz.h"
#include "mupdf/pdf.h"

#define INPUT_LIMIT (16u * 1024u * 1024u)

static void hex_token(const char *kind, const char *bytes, size_t len)
{
    size_t i;
    printf("%s\t", kind);
    for (i = 0; i < len; ++i)
        printf("%02x", (unsigned char)bytes[i]);
    putchar('\n');
}

static const char *reserved(pdf_token token)
{
    switch (token) {
    case PDF_TOK_R: return "R";
    case PDF_TOK_TRUE: return "true";
    case PDF_TOK_FALSE: return "false";
    case PDF_TOK_NULL: return "null";
    case PDF_TOK_OBJ: return "obj";
    case PDF_TOK_ENDOBJ: return "endobj";
    case PDF_TOK_STREAM: return "stream";
    case PDF_TOK_ENDSTREAM: return "endstream";
    case PDF_TOK_XREF: return "xref";
    case PDF_TOK_TRAILER: return "trailer";
    case PDF_TOK_STARTXREF: return "startxref";
    case PDF_TOK_NEWOBJ: return "newobj";
    default: return NULL;
    }
}

int main(void)
{
    unsigned char *data = NULL;
    size_t used = 0, capacity = 4096;
    fz_context *ctx;
    fz_stream *stream = NULL;
    pdf_lexbuf_large lexbuf;
    int status = 0;

    data = malloc(capacity);
    if (!data) { fputs("input allocation failed\n", stderr); return 1; }
    for (;;) {
        size_t n = fread(data + used, 1, capacity - used, stdin);
        used += n;
        if (ferror(stdin)) { fputs("input read failed\n", stderr); free(data); return 1; }
        if (feof(stdin)) break;
        if (used == capacity) {
            unsigned char *next;
            if (capacity == INPUT_LIMIT) {
                int extra = fgetc(stdin);
                if (extra != EOF || ferror(stdin)) {
                    fputs("input exceeds 16 MiB or read failed\n", stderr);
                    free(data); return 1;
                }
                break;
            }
            capacity *= 2;
            next = realloc(data, capacity);
            if (!next) { fputs("input allocation failed\n", stderr); free(data); return 1; }
            data = next;
        }
    }
    ctx = fz_new_context(NULL, NULL, FZ_STORE_UNLIMITED);
    if (!ctx) { fputs("MuPDF context allocation failed\n", stderr); free(data); return 1; }
    memset(&lexbuf, 0, sizeof(lexbuf));
    pdf_lexbuf_init(ctx, &lexbuf.base, PDF_LEXBUF_LARGE);
    fz_var(stream);
    fz_var(status);
    fz_try(ctx) {
        stream = fz_open_memory(ctx, data, used);
        for (;;) {
            pdf_token token;
            const char *word;
            int64_t before = fz_tell(ctx, stream);
            /* Numeric-to-keyword fallback does not set len. Reset it so
             * stale bytes from a preceding token cannot escape. */
            lexbuf.base.len = 0;
            token = pdf_lex(ctx, stream, &lexbuf.base);
            if (token == PDF_TOK_EOF) break;
            if (token == PDF_TOK_ERROR) {
                fprintf(stderr, "MuPDF lexical error after input offset %" PRId64 "\n", before);
                status = 2; break;
            }
            switch (token) {
            case PDF_TOK_INT: printf("integer\t%" PRId64 "\n", lexbuf.base.i); break;
            case PDF_TOK_REAL: printf("real\t%.9g\n", (double)lexbuf.base.f); break;
            case PDF_TOK_NAME: hex_token("name", lexbuf.base.scratch, lexbuf.base.len); break;
            case PDF_TOK_STRING: hex_token("string", lexbuf.base.scratch, lexbuf.base.len); break;
            case PDF_TOK_KEYWORD:
                /* MuPDF's numeric fallback is a NUL-terminated C string.
                 * Ordinary keywords retain their explicit byte length. */
                hex_token("keyword", lexbuf.base.scratch,
                    lexbuf.base.len ? lexbuf.base.len : strlen(lexbuf.base.scratch));
                break;
            case PDF_TOK_OPEN_ARRAY: puts("array-start"); break;
            case PDF_TOK_CLOSE_ARRAY: puts("array-end"); break;
            case PDF_TOK_OPEN_DICT: puts("dict-start"); break;
            case PDF_TOK_CLOSE_DICT: puts("dict-end"); break;
            case PDF_TOK_OPEN_BRACE: puts("brace-start"); break;
            case PDF_TOK_CLOSE_BRACE: puts("brace-end"); break;
            default:
                word = reserved(token);
                if (word) hex_token("keyword", word, strlen(word));
                else { fputs("unknown MuPDF token\n", stderr); status = 2; }
                break;
            }
            if (status) break;
            if (fz_tell(ctx, stream) <= before) {
                fputs("MuPDF lexer did not advance\n", stderr); status = 2; break;
            }
        }
    }
    fz_always(ctx) {
        pdf_lexbuf_fin(ctx, &lexbuf.base);
        fz_drop_stream(ctx, stream);
    }
    fz_catch(ctx) {
        fprintf(stderr, "MuPDF exception: %s\n", fz_caught_message(ctx));
        status = 2;
    }
    fz_drop_context(ctx);
    free(data);
    if (fflush(stdout) == EOF || ferror(stdout)) status = 1;
    return status;
}
