/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned MuPDF page-geometry oracle; not a production backend.
 */
#include <stdio.h>
#include <stdlib.h>
#include "mupdf/fitz.h"
#include "mupdf/pdf.h"
#define INPUT_LIMIT (16u * 1024u * 1024u)

int main(void)
{
    unsigned char *bytes = malloc(INPUT_LIMIT + 1u);
    size_t length;
    fz_context *ctx;
    fz_stream *stream = NULL;
    pdf_document *document = NULL;
    pdf_page *page = NULL;
    fz_rect *rectangles = NULL;
    fz_matrix *matrices = NULL;
    int status = 0;
    if (!bytes) return 1;
    length = fread(bytes, 1, INPUT_LIMIT + 1u, stdin);
    if (ferror(stdin) || length > INPUT_LIMIT) { free(bytes); return 1; }
    ctx = fz_new_context(NULL, NULL, FZ_STORE_DEFAULT);
    if (!ctx) { free(bytes); return 1; }
    fz_var(stream); fz_var(document); fz_var(page); fz_var(status);
    fz_var(rectangles); fz_var(matrices);
    fz_try(ctx) {
        int count, i;
        stream = fz_open_memory(ctx, bytes, length);
        document = pdf_open_document_with_stream(ctx, stream);
        pdf_disable_js(ctx, document);
        if (pdf_needs_password(ctx, document)) fz_throw(ctx, FZ_ERROR_ARGUMENT, "password required");
        count = pdf_count_pages(ctx, document);
        if (count < 0 || count > 10000) fz_throw(ctx, FZ_ERROR_LIMIT, "page limit");
        rectangles = fz_malloc_array(ctx, count, fz_rect);
        matrices = fz_malloc_array(ctx, count, fz_matrix);
        for (i = 0; i < count; ++i) {
            page = pdf_load_page(ctx, document, i);
            pdf_page_transform(ctx, page, NULL, &matrices[i]);
            rectangles[i] = pdf_bound_page(ctx, page, FZ_CROP_BOX);
            pdf_drop_page(ctx, page); page = NULL;
        }
        /* Repairs may happen lazily while a page is loaded. Query only after
         * every requested page has been visited; emit nothing on exceptions. */
        printf("{\"repaired\":%s,\"pages\":[", pdf_was_repaired(ctx, document) ? "true" : "false");
        for (i = 0; i < count; ++i) {
            fz_rect bounds = rectangles[i];
            fz_matrix m = matrices[i];
            printf("%s{\"matrix\":[%.9g,%.9g,%.9g,%.9g,%.9g,%.9g],\"bounds\":[%.9g,%.9g,%.9g,%.9g]}", i ? "," : "", m.a,m.b,m.c,m.d,m.e,m.f,bounds.x0,bounds.y0,bounds.x1,bounds.y1);
        }
        puts("]}");
        if (fflush(stdout) == EOF || ferror(stdout)) status = 1;
    }
    fz_always(ctx) {
        fz_free(ctx, rectangles); fz_free(ctx, matrices);
        pdf_drop_page(ctx, page);
        pdf_drop_document(ctx, document);
        fz_drop_stream(ctx, stream);
    }
    fz_catch(ctx) {
        fprintf(stderr, "MuPDF page geometry oracle: %s\n", fz_caught_message(ctx));
        status = 2;
    }
    fz_drop_context(ctx);
    free(bytes);
    return status;
}
