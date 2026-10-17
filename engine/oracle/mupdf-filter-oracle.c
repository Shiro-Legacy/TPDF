/* SPDX-License-Identifier: AGPL-3.0-or-later
 * Test-only MuPDF 1.28.1 filter adapter. See ../NOTICE and ../COPYING.
 * Usage: mupdf-filter-oracle flate|asciihex|ascii85|runlength
 * Or: mupdf-filter-oracle flate PREDICTOR COLORS BPC COLUMNS
 * Or: mupdf-filter-oracle lzw EARLY_CHANGE [PREDICTOR COLORS BPC COLUMNS]
 * stdin/stdout are raw bytes, each bounded to 16 MiB. No partial stdout
 * is emitted on failure. Exit 0: decoded; 1: usage/I/O; 2: decode/limit.
 */
#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "mupdf/fitz.h"
#define BYTE_LIMIT (16u * 1024u * 1024u)

static int positive_int(const char *text, int *value)
{
    char *end;
    long parsed;
    const char *cursor = text;
    if (!*cursor) return 0;
    while (*cursor) {
        if (*cursor < '0' || *cursor > '9') return 0;
        ++cursor;
    }
    errno = 0;
    parsed = strtol(text, &end, 10);
    if (errno || *end || parsed <= 0 || parsed > INT_MAX) return 0;
    *value = (int)parsed;
    return 1;
}

int main(int argc, char **argv)
{
    unsigned char *input, *output, *predicted_output = NULL;
    size_t input_len, output_len = 0;
    fz_context *ctx;
    fz_stream *raw = NULL, *decoded = NULL;
    fz_stream *predict_input = NULL, *predicted = NULL;
    int filter = -1, status = 0, early_change = 1, has_predictor = 0;
    int predictor = 1, colors = 1, bpc = 8, columns = 1;
    if (argc >= 2) {
        if (!strcmp(argv[1], "flate") && (argc == 2 || argc == 6)) filter = 0;
        if (!strcmp(argv[1], "asciihex") && argc == 2) filter = 1;
        if (!strcmp(argv[1], "ascii85") && argc == 2) filter = 2;
        if (!strcmp(argv[1], "runlength") && argc == 2) filter = 3;
        if (!strcmp(argv[1], "lzw") && (argc == 3 || argc == 7)) {
            if (!strcmp(argv[2], "0")) { filter = 4; early_change = 0; }
            if (!strcmp(argv[2], "1")) { filter = 4; early_change = 1; }
        }
    }
    has_predictor = (filter == 0 && argc == 6) || (filter == 4 && argc == 7);
    if (has_predictor) {
        int first = filter == 4 ? 3 : 2;
        if (!positive_int(argv[first], &predictor)
            || !positive_int(argv[first + 1], &colors) || !positive_int(argv[first + 2], &bpc)
            || !positive_int(argv[first + 3], &columns)
            || colors > 64 || !(bpc == 1 || bpc == 2 || bpc == 4 || bpc == 8 || bpc == 16)) {
            fputs("invalid predictor arguments (positive values; colors<=64; bpc=1,2,4,8,16)\n", stderr);
            return 1;
        }
        if (((uint64_t)columns * (uint64_t)colors * (uint64_t)bpc + 7) / 8 > BYTE_LIMIT) {
            fputs("predictor row exceeds 16 MiB\n", stderr);
            return 2;
        }
    }
    if (filter < 0) {
        fputs("usage: mupdf-filter-oracle flate|asciihex|ascii85|runlength; flate PREDICTOR COLORS BPC COLUMNS; lzw EARLY_CHANGE [PREDICTOR COLORS BPC COLUMNS]\n", stderr);
        return 1;
    }
    input = malloc(BYTE_LIMIT + 1);
    output = malloc(BYTE_LIMIT + 1);
    if (has_predictor) predicted_output = malloc(BYTE_LIMIT + 1);
    if (!input || !output || (has_predictor && !predicted_output)) {
        fputs("oracle allocation failed\n", stderr);
        free(input); free(output); free(predicted_output); return 1;
    }
    input_len = fread(input, 1, BYTE_LIMIT + 1, stdin);
    if (ferror(stdin) || input_len > BYTE_LIMIT) {
        fputs("input read failed or input exceeds 16 MiB\n", stderr);
        free(input); free(output); free(predicted_output); return 1;
    }
    ctx = fz_new_context(NULL, NULL, FZ_STORE_UNLIMITED);
    if (!ctx) {
        fputs("MuPDF context allocation failed\n", stderr);
        free(input); free(output); free(predicted_output); return 1;
    }
    fz_var(raw);
    fz_var(decoded);
    fz_var(predict_input);
    fz_var(predicted);
    fz_var(output_len);
    fz_var(status);
    fz_try(ctx) {
        raw = fz_open_memory(ctx, input, input_len);
        switch (filter) {
        case 0: decoded = fz_open_flated(ctx, raw, 15); break;
        case 1: decoded = fz_open_ahxd(ctx, raw); break;
        case 2: decoded = fz_open_a85d(ctx, raw); break;
        case 3: decoded = fz_open_rld(ctx, raw); break;
        case 4: decoded = fz_open_lzwd(ctx, raw, early_change, 9, 0, 0); break;
        }
        output_len = fz_read(ctx, decoded, output, BYTE_LIMIT + 1);
        /* fz_read may catch decoder exceptions and set stream->error. */
        if (decoded->error || raw->error) {
            fputs("MuPDF decoder reported a stream error\n", stderr);
            status = 2;
        } else if (output_len > BYTE_LIMIT) {
            fputs("decoded output exceeds 16 MiB\n", stderr);
            status = 2;
        }
        if (!status && has_predictor) {
            predict_input = fz_open_memory(ctx, output, output_len);
            predicted = fz_open_predict(ctx, predict_input, predictor, columns, colors, bpc);
            output_len = fz_read(ctx, predicted, predicted_output, BYTE_LIMIT + 1);
            if (predicted->error || predict_input->error) {
                fputs("MuPDF predictor reported a stream error\n", stderr);
                status = 2;
            } else if (output_len > BYTE_LIMIT) {
                fputs("predicted output exceeds 16 MiB\n", stderr);
                status = 2;
            }
        }
    }
    fz_always(ctx) {
        fz_drop_stream(ctx, predicted);
        fz_drop_stream(ctx, predict_input);
        fz_drop_stream(ctx, decoded);
        fz_drop_stream(ctx, raw);
    }
    fz_catch(ctx) {
        fprintf(stderr, "MuPDF exception: %s\n", fz_caught_message(ctx));
        status = 2;
    }
    fz_drop_context(ctx);
    if (!status) {
        if (fwrite(has_predictor ? predicted_output : output, 1, output_len, stdout) != output_len || fflush(stdout) == EOF)
            status = 1;
    }
    free(input); free(output); free(predicted_output);
    return status;
}
