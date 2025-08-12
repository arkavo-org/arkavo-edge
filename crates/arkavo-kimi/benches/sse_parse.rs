#![allow(clippy::disallowed_methods)] // Benchmarks need block_on

use bytes::Bytes;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use futures::{StreamExt, stream};
use std::time::Duration;

/// Generate a realistic SSE response chunk
fn generate_sse_chunk(index: usize, content: &str) -> String {
    format!(
        r#"data: {{"id":"chatcmpl-{index}","object":"chat.completion.chunk","created":1234567890,"model":"moonshot-v1-8k","choices":[{{"index":0,"delta":{{"content":"{content}"}},"finish_reason":null}}]}}"#
    )
}

/// Benchmark parsing a single SSE line
fn bench_parse_single_line(c: &mut Criterion) {
    let line = generate_sse_chunk(1, "Hello, world!");

    c.bench_function("parse_single_sse_line", |b| {
        b.iter(|| {
            // We need to make the parse function public for benchmarking
            // For now, we'll benchmark the JSON parsing part
            let data = line.strip_prefix("data: ").unwrap();
            let _: serde_json::Value = serde_json::from_str(black_box(data)).unwrap();
        });
    });
}

/// Benchmark parsing 100 streaming completions
fn bench_parse_stream_100_completions(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("parse_100_streaming_completions", |b| {
        b.to_async(&runtime).iter(|| async {
            // Generate 100 completions with varying content
            let mut chunks = Vec::new();

            // Add some initial messages
            for i in 0..100 {
                let content = match i % 5 {
                    0 => "The quick brown fox",
                    1 => "jumps over the lazy dog.",
                    2 => "This is a longer response with more content to parse.",
                    3 => "Short.",
                    4 => "Another medium-length response for variety.",
                    _ => unreachable!(),
                };

                let chunk = generate_sse_chunk(i, content);
                chunks.push(Ok(Bytes::from(format!("{chunk}\n"))));

                // Add some empty lines for realism
                if i % 10 == 0 {
                    chunks.push(Ok(Bytes::from("\n")));
                }
            }

            // Add the done signal
            chunks.push(Ok(Bytes::from("data: [DONE]\n")));

            let body_stream = stream::iter(chunks);
            let mut parser = arkavo_kimi::stream::SseParser::new(body_stream, 1024);

            let mut count = 0;
            while let Some(result) = parser.next().await {
                if let Ok(_) = result {
                    count += 1;
                }
            }

            assert_eq!(count, 101); // 100 messages + 1 done signal
        });
    });
}

/// Benchmark memory allocations in String::from_utf8_lossy
fn bench_utf8_conversion(c: &mut Criterion) {
    let bytes = Bytes::from("This is a test string with UTF-8 content: Hello 世界!");

    c.bench_function("utf8_lossy_conversion", |b| {
        b.iter(|| {
            let _ = String::from_utf8_lossy(black_box(&bytes));
        });
    });
}

/// Benchmark with partial/split chunks
fn bench_parse_split_chunks(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("parse_split_sse_chunks", |b| {
        b.to_async(&runtime).iter(|| async {
            // Create chunks that split in the middle of JSON
            let full_chunk = generate_sse_chunk(1, "This is split across chunks");
            let mid = full_chunk.len() / 2;

            let chunks = vec![
                Ok(Bytes::from(full_chunk[..mid].to_string())),
                Ok(Bytes::from(full_chunk[mid..].to_string())),
                Ok(Bytes::from("\n")),
                Ok(Bytes::from("data: [DONE]\n")),
            ];

            let body_stream = stream::iter(chunks);
            let mut parser = arkavo_kimi::stream::SseParser::new(body_stream, 1024);

            let mut count = 0;
            while let Some(result) = parser.next().await {
                if let Ok(_) = result {
                    count += 1;
                }
            }

            assert_eq!(count, 2); // 1 message + 1 done signal
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .sample_size(100);
    targets = bench_parse_single_line,
              bench_parse_stream_100_completions,
              bench_utf8_conversion,
              bench_parse_split_chunks
}
criterion_main!(benches);
