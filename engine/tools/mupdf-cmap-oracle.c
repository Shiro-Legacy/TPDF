/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only pinned MuPDF 1.28.1 standalone CMap oracle. No system CMap loading.
 * Usage: mupdf-cmap-oracle QUERY_FILE; CMap bytes on stdin. Query file lines:
 * lookup HEX (1..4 bytes) or decode HEX (0..4 bytes, '-' means empty).
 * JSON output describes native behavior, including widthless lookup and the
 * native decode fallback consumed=1/value=0, even for empty input.
 * Caps: CMap4MiB, queries8MiB/262144, output64MiB, MuPDF heap128MiB,
 * CPU10s/wall15s. Exit1 input/query/I/O error, exit2 MuPDF/limit error.
 * No partial stdout on refusal. Callers must impose subprocess timeouts too.
 */
#include <inttypes.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>
#include "mupdf/fitz.h"
#include "mupdf/pdf.h"
#define INPUT_LIMIT (4u * 1024u * 1024u)
#define QUERY_BYTES (8u * 1024u * 1024u)
#define QUERY_LIMIT 262144u
#define OUTPUT_LIMIT (64u * 1024u * 1024u)
#define HEAP_LIMIT (128u * 1024u * 1024u)
typedef union { size_t size; max_align_t alignment; } allocation_header;
typedef struct { size_t used; } budget;
typedef struct { unsigned char bytes[4]; unsigned int width; int decode; } query;
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
static int nibble(unsigned char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}
static int read_queries(FILE *file, query *queries, size_t *count)
{
    char line[32];
    size_t bytes = 0, line_number = 0;
    *count = 0;
    while (fgets(line, sizeof(line), file)) {
        size_t len = strlen(line), start, i;
        query *q;
        ++line_number; bytes += len;
        if (bytes > QUERY_BYTES || *count >= QUERY_LIMIT ||
            (len && line[len-1] != '\n' && !feof(file))) goto invalid;
        if (len && line[len-1] == '\n') --len;
        if (len && line[len-1] == '\r') --len;
        if (len >= 7 && !memcmp(line, "lookup ", 7)) start = 7;
        else if (len >= 7 && !memcmp(line, "decode ", 7)) start = 7;
        else goto invalid;
        q = &queries[(*count)++]; memset(q, 0, sizeof(*q));
        q->decode = line[0] == 'd';
        if (len == start+1 && line[start] == '-' && q->decode) continue;
        if (len <= start || len-start > 8 || (len-start)%2) goto invalid;
        q->width = (unsigned int)((len-start)/2);
        for (i = 0; i < q->width; ++i) {
            int a = nibble((unsigned char)line[start+i*2]);
            int b = nibble((unsigned char)line[start+i*2+1]);
            if (a < 0 || b < 0) goto invalid;
            q->bytes[i] = (unsigned char)((a<<4)|b);
        }
    }
    if (ferror(file)) { fputs("query file read error\n", stderr); return 0; }
    return 1;
invalid:
    fprintf(stderr, "invalid query or query limit at line %zu\n", line_number);
    return 0;
}
static void emit(fz_context *ctx, FILE *file, size_t *count, const char *format, ...)
{
    va_list ap;
    int n;
    va_start(ap, format); n = vfprintf(file, format, ap); va_end(ap);
    if (n < 0) fz_throw(ctx, FZ_ERROR_SYSTEM, "oracle output I/O");
    if ((size_t)n > OUTPUT_LIMIT - *count) fz_throw(ctx, FZ_ERROR_LIMIT, "oracle output limit");
    *count += (size_t)n;
}
static void diagnostics(void *user, const char *message)
{
    unsigned int *count = user;
    if (*count < 32) fprintf(stderr, "MuPDF: %.512s\n", message);
    else if (*count == 32) fputs("MuPDF: further diagnostics suppressed\n", stderr);
    if (*count < 33) ++*count;
}
int main(int argc, char **argv)
{
    unsigned char *input;
    size_t input_len, query_count, output_size = 0, i;
    query *queries;
    FILE *qfile, *output;
    fz_context *ctx;
    fz_stream *stream = NULL;
    pdf_cmap *cmap = NULL;
    int status = 0;
    budget heap = {0};
    fz_alloc_context alloc = {&heap, limited_malloc, limited_realloc, limited_free};
    unsigned int diagnostic_count = 0;
    struct rlimit cpu;
    if (argc != 2) { fputs("usage: mupdf-cmap-oracle QUERY_FILE (CMap stdin)\n", stderr); return 1; }
    if (getrlimit(RLIMIT_CPU, &cpu)) return 1;
    if (cpu.rlim_cur == RLIM_INFINITY || cpu.rlim_cur > 10) cpu.rlim_cur = 10;
    if (setrlimit(RLIMIT_CPU, &cpu)) return 1;
    alarm(15);
    input = malloc(INPUT_LIMIT + 1);
    queries = calloc(QUERY_LIMIT, sizeof(*queries));
    if (!input || !queries) { free(input); free(queries); return 1; }
    input_len = fread(input, 1, INPUT_LIMIT+1, stdin);
    if (ferror(stdin) || input_len > INPUT_LIMIT) {
        fputs("CMap input I/O or 4MiB limit\n", stderr); free(input); free(queries); return 1;
    }
    qfile = fopen(argv[1], "rb");
    if (!qfile) { perror("query file"); free(input); free(queries); return 1; }
    status = !read_queries(qfile, queries, &query_count); fclose(qfile);
    if (status) { free(input); free(queries); return 1; }
    output = tmpfile();
    if (!output) { free(input); free(queries); return 1; }
    ctx = fz_new_context(&alloc, NULL, 0);
    if (!ctx) { fclose(output); free(input); free(queries); return 1; }
    fz_set_warning_callback(ctx, diagnostics, &diagnostic_count);
    fz_set_error_callback(ctx, diagnostics, &diagnostic_count);
    fz_var(stream); fz_var(cmap); fz_var(status); fz_var(output_size);
    fz_try(ctx) {
        stream = fz_open_memory(ctx, input, input_len);
        cmap = pdf_load_cmap(ctx, stream);
        if (stream->error) fz_throw(ctx, FZ_ERROR_FORMAT, "CMap read failed");
        /* pdf_load_cmap only records a name. Never resolve/install that base. */
        if (cmap->usecmap) fz_throw(ctx, FZ_ERROR_FORMAT, "unexpected resolved base CMap");
        emit(ctx, output, &output_size, "{\"usecmap_ignored\":%s,\"lookup_widthless\":true,\"queries\":[", cmap->usecmap_name[0] ? "true" : "false");
        for (i = 0; i < query_count; ++i) {
            query *q = &queries[i];
            unsigned int value = 0, j, width = q->width;
            int consumed = 0, matched = 0, values[PDF_MRANGE_CAP], n, valid = 1;
            for (j = 0; j < q->width; ++j) value = (value << 8) | q->bytes[j];
            if (q->decode) {
                unsigned int original;
                consumed = pdf_decode_cmap(cmap, q->bytes, q->bytes+q->width, &value);
                width = (unsigned int)consumed;
                original = 0;
                if (consumed > 0 && consumed <= (int)q->width) {
                    for (j = 0; j < width; ++j) original = (original << 8) | q->bytes[j];
                    for (j = 0; j < (unsigned int)cmap->codespace_len; ++j)
                        if (cmap->codespace[j].n == consumed && original == value &&
                            value >= cmap->codespace[j].low && value <= cmap->codespace[j].high) matched = 1;
                }
            }
            n = pdf_lookup_cmap_full(cmap, value, values);
            if (n < 0 || n > PDF_MRANGE_CAP) fz_throw(ctx, FZ_ERROR_LIMIT, "lookup length");
            for (j = 0; j < (unsigned int)n; ++j)
                if (values[j] < 0 || values[j] > 0x10ffff || (values[j] >= 0xd800 && values[j] <= 0xdfff)) valid = 0;
            emit(ctx, output, &output_size, "%s{\"kind\":\"%s\",\"input\":\"", i ? "," : "", q->decode ? "decode" : "lookup");
            for (j = 0; j < q->width; ++j) emit(ctx, output, &output_size, "%02x", q->bytes[j]);
            emit(ctx, output, &output_size, "\",\"width\":%u,\"value\":%" PRIu32, width, (uint32_t)value);
            if (q->decode) emit(ctx, output, &output_size, ",\"consumed\":%d,\"matched_codespace\":%s", consumed, matched ? "true" : "false");
            emit(ctx, output, &output_size, ",\"valid_scalars\":%s,\"scalars\":", valid ? "true" : "false");
            if (!n) emit(ctx, output, &output_size, "null");
            else {
                emit(ctx, output, &output_size, "[");
                for (j = 0; j < (unsigned int)n; ++j) emit(ctx, output, &output_size, "%s%" PRIu32, j ? "," : "", (uint32_t)values[j]);
                emit(ctx, output, &output_size, "]");
            }
            emit(ctx, output, &output_size, "}");
        }
        emit(ctx, output, &output_size, "]}\n");
    }
    fz_always(ctx) { pdf_drop_cmap(ctx, cmap); fz_drop_stream(ctx, stream); }
    fz_catch(ctx) { fprintf(stderr, "MuPDF CMap error: %s\n", fz_caught_message(ctx)); status = 2; }
    if (!status) {
        unsigned char chunk[8192];
        size_t n;
        if (fflush(output) || fseek(output, 0, SEEK_SET)) status = 1;
        while (!status && (n = fread(chunk, 1, sizeof(chunk), output)))
            if (fwrite(chunk, 1, n, stdout) != n) status = 1;
        if (ferror(output) || fflush(stdout) || ferror(stdout)) status = 1;
    }
    fz_drop_context(ctx); fclose(output); free(input); free(queries);
    return status;
}
