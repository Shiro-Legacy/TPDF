/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Pinned MuPDF1.28.1 content-array joining oracle; not a production backend.
 * Usage: mupdf-content-oracle PAGE_NUMBER (one based), PDF bytes on stdin.
 * Raw joined decoded bytes on stdout; input/output each <=16MiB, <=10000pages.
 * Exit0 success,1usage/I/O,2PDF/decode/limit/repair. No partial stdout on refusal.
 */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include "mupdf/fitz.h"
#include "mupdf/pdf.h"
#define BYTE_LIMIT (16u * 1024u * 1024u)

int main(int argc, char **argv)
{
    const char *cursor;
    char *end;
    long page;
    unsigned char *input, *output;
    size_t input_len, output_len = 0;
    int status = 0;
    fz_context *ctx;
    fz_stream *raw = NULL, *content = NULL;
    pdf_document *doc = NULL;
    if (argc != 2) {
        fputs("usage: mupdf-content-oracle PAGE_NUMBER (one based)\n", stderr);
        return 1;
    }
    cursor = argv[1];
    if (!*cursor) return 1;
    while (*cursor) {
        if (*cursor < '0' || *cursor > '9') return 1;
        ++cursor;
    }
    errno = 0;
    page = strtol(argv[1], &end, 10);
    if (errno || *end || page < 1 || page > 10000) return 1;
    input = malloc(BYTE_LIMIT + 1);
    output = malloc(BYTE_LIMIT + 1);
    if (!input || !output) {
        fputs("oracle allocation failed\n", stderr);
        free(input); free(output); return 1;
    }
    input_len = fread(input, 1, BYTE_LIMIT + 1, stdin);
    if (ferror(stdin) || input_len > BYTE_LIMIT) {
        fputs("input failed or exceeds16MiB\n", stderr);
        free(input); free(output); return 1;
    }
    ctx = fz_new_context(NULL, NULL, FZ_STORE_DEFAULT);
    if (!ctx) { free(input); free(output); return 1; }
    fz_var(raw); fz_var(content); fz_var(doc); fz_var(status); fz_var(output_len);
    fz_try(ctx) {
        int count;
        pdf_obj *page_obj, *contents;
        raw = fz_open_memory(ctx, input, input_len);
        doc = pdf_open_document_with_stream(ctx, raw);
        pdf_disable_js(ctx, doc);
        if (pdf_needs_password(ctx, doc))
            fz_throw(ctx, FZ_ERROR_FORMAT, "encrypted input");
        count = pdf_count_pages(ctx, doc);
        if (count > 10000 || page > count)
            fz_throw(ctx, FZ_ERROR_LIMIT, "page count/index limit");
        page_obj = pdf_lookup_page_obj(ctx, doc, (int)page - 1);
        contents = pdf_dict_get(ctx, page_obj, PDF_NAME(Contents));
        content = pdf_open_contents_stream(ctx, doc, contents);
        output_len = fz_read(ctx, content, output, BYTE_LIMIT + 1);
        if (content->error || raw->error || output_len > BYTE_LIMIT)
            fz_throw(ctx, FZ_ERROR_LIMIT, "content decode failed or exceeds16MiB");
        if (pdf_was_repaired(ctx, doc))
            fz_throw(ctx, FZ_ERROR_FORMAT, "reference repaired input");
    }
    fz_always(ctx) {
        fz_drop_stream(ctx, content);
        pdf_drop_document(ctx, doc);
        fz_drop_stream(ctx, raw);
    }
    fz_catch(ctx) {
        fprintf(stderr, "MuPDF content error: %s\n", fz_caught_message(ctx));
        status = 2;
    }
    if (!status && (fwrite(output, 1, output_len, stdout) != output_len || fflush(stdout) || ferror(stdout)))
        status = 1;
    fz_drop_context(ctx);
    free(input); free(output);
    return status;
}
