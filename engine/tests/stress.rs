// SPDX-License-Identifier: MIT
//! Reproducible adversarial smoke tests; not a substitute for coverage-guided fuzzing.
use tandem_pdf_engine::lexer::{LexLimits, Lexer};
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn arbitrary_bytes_terminate_with_bounded_monotonic_token_spans() {
    let mut seed = 0x74616e64656d_u64;
    for _ in 0..20_000 {
        let length = (random(&mut seed) % 512) as usize;
        let input: Vec<u8> = (0..length).map(|_| random(&mut seed) as u8).collect();
        let mut lexer = Lexer::new(
            &input,
            LexLimits {
                max_token_bytes: 128,
                max_tokens: 64,
            },
        );
        let mut end = 0;
        let mut count = 0;
        while let Ok(Some(token)) = lexer.next_token() {
            assert!(token.span.start >= end);
            assert!(token.span.end > token.span.start);
            assert!(token.span.end <= input.len());
            assert!(token.span.len() <= 128);
            end = token.span.end;
            count += 1;
            assert!(count <= 64);
        }
        let limits = ParseLimits {
            max_depth: 16,
            max_objects: 64,
            max_decoded_bytes: 512,
            lex: LexLimits {
                max_token_bytes: 128,
                max_tokens: 64,
            },
        };
        // Every arbitrary input must return a value or explicit error, never panic.
        let _ = parse_complete(&input, limits);
    }
}

#[test]
fn syntax_biased_mutations_terminate_under_limits() {
    let seeds: &[&[u8]] = &[
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        b"[true false null (hello) <00ff> /A#20B 1.25]",
        b"[ [ [ [ 1 ] ] ] ]",
    ];
    let mut rng = 20261003;
    for base in seeds {
        for _ in 0..2000 {
            let mut input = base.to_vec();
            for _ in 0..(random(&mut rng) % 8) {
                let at = random(&mut rng) as usize % input.len();
                input[at] = random(&mut rng) as u8;
            }
            if random(&mut rng) % 3 == 0 {
                let end = random(&mut rng) as usize % input.len();
                input.truncate(end);
            }
            let _ = parse_complete(&input, ParseLimits::default());
        }
    }
}
