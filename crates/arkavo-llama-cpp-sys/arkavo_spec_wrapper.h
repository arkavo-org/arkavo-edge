// Thin C wrapper around llama.cpp's common_speculative API (b9292+).
// Exposes COMMON_SPECULATIVE_TYPE_NGRAM_SIMPLE only — other types are a follow-up.

#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct llama_batch;

// Opaque handle.
typedef struct arkavo_spec arkavo_spec;

// Init with NGRAM_SIMPLE. n_seq is the number of parallel sequences (use 1).
// Returns NULL on failure.
arkavo_spec *arkavo_spec_init_ngram(uint32_t n_seq);

void arkavo_spec_free(arkavo_spec *spec);

// Begin a new generation for seq_id with the given prompt tokens.
// Pass the prompt token IDs (token_t = llama_token = int32_t).
void arkavo_spec_begin(
    arkavo_spec *spec,
    int32_t seq_id,
    const int32_t *prompt_tokens,
    uint32_t n_prompt_tokens);

// Process a verified batch through the speculative context.
// Returns 0 on success, non-zero on failure.
int arkavo_spec_process(arkavo_spec *spec, const struct llama_batch *batch);

// Generate a draft for seq_id given n_past (current KV position) and id_last
// (most recently sampled token). Writes up to n_max draft tokens into out_tokens.
// Returns the number of draft tokens written (0 if cache has no useful prediction).
// out_tokens must have capacity >= n_max.
uint32_t arkavo_spec_draft(
    arkavo_spec *spec,
    int32_t seq_id,
    int32_t n_past,
    int32_t id_last,
    int32_t n_max,
    int32_t *out_tokens);

// Inform the speculative context that n_accepted of the drafted tokens were
// accepted by sampling against the target model.
void arkavo_spec_accept(arkavo_spec *spec, int32_t seq_id, uint16_t n_accepted);

#ifdef __cplusplus
}
#endif
