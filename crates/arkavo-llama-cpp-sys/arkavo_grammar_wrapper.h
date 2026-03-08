// Exception-safe wrappers around llama grammar sampler init functions.
// Catches C++ exceptions at the extern "C" boundary to prevent
// foreign exception aborts in Rust.

#pragma once

// Requires llama.h to be included before this header
// (for llama_vocab, llama_sampler types)

#ifdef __cplusplus
extern "C" {
#endif

// Exception-safe wrapper around llama_sampler_init_grammar.
// Returns NULL on C++ exception instead of aborting.
struct llama_sampler *arkavo_sampler_init_grammar(
    const struct llama_vocab *vocab,
    const char *grammar_str,
    const char *grammar_root);

// Exception-safe wrapper around llama_sampler_init_grammar_lazy_patterns.
// Returns NULL on C++ exception instead of aborting.
struct llama_sampler *arkavo_sampler_init_grammar_lazy(
    const struct llama_vocab *vocab,
    const char *grammar_str,
    const char *grammar_root,
    const char **trigger_patterns,
    int num_triggers);

// Exception-safe wrapper around llama_sampler_sample.
// Returns the sampled token, or -1 on C++ exception.
int32_t arkavo_sampler_sample(
    struct llama_sampler *sampler,
    struct llama_context *ctx,
    int idx);

// Exception-safe wrapper around llama_sampler_accept.
void arkavo_sampler_accept(
    struct llama_sampler *sampler,
    int32_t token);

// Exception-safe wrapper around llama_sampler_free.
void arkavo_sampler_free(struct llama_sampler *sampler);

#ifdef __cplusplus
}
#endif
