// Implementation: forward C calls to common_speculative_* in
// vendor/llama.cpp/common/speculative.{h,cpp}. All C++ exceptions caught at
// the extern "C" boundary. Pattern mirrors arkavo_chat_wrapper.cpp.

#include "arkavo_spec_wrapper.h"
#include "speculative.h"
#include "llama.h"
#include "common.h"

#include <stdexcept>
#include <vector>

struct arkavo_spec {
    common_speculative_ptr ptr;
};

extern "C" {

arkavo_spec *arkavo_spec_init_ngram(uint32_t n_seq) {
    try {
        common_params_speculative params;
        params.types = { COMMON_SPECULATIVE_TYPE_NGRAM_SIMPLE };
        auto *raw = common_speculative_init(params, n_seq);
        if (!raw) return nullptr;
        auto *handle = new arkavo_spec();
        handle->ptr.reset(raw);
        return handle;
    } catch (...) {
        return nullptr;
    }
}

void arkavo_spec_free(arkavo_spec *spec) {
    delete spec; // unique_ptr in ptr handles cleanup via common_speculative_deleter
}

void arkavo_spec_begin(
    arkavo_spec *spec,
    int32_t seq_id,
    const int32_t *prompt_tokens,
    uint32_t n_prompt_tokens)
{
    if (!spec) return;
    try {
        llama_tokens tokens(prompt_tokens, prompt_tokens + n_prompt_tokens);
        common_speculative_begin(spec->ptr.get(), seq_id, tokens);
    } catch (...) {}
}

int arkavo_spec_process(arkavo_spec *spec, const struct llama_batch *batch) {
    if (!spec || !batch) return -1;
    try {
        return common_speculative_process(spec->ptr.get(), *batch) ? 0 : -2;
    } catch (...) {
        return -3;
    }
}

uint32_t arkavo_spec_draft(
    arkavo_spec *spec,
    int32_t seq_id,
    int32_t n_past,
    int32_t id_last,
    int32_t n_max,
    int32_t *out_tokens)
{
    if (!spec || n_max <= 0) return 0;
    try {
        auto &params = common_speculative_get_draft_params(spec->ptr.get(), seq_id);
        params.drafting = true;
        params.n_max = n_max;
        params.n_past = n_past;
        params.id_last = id_last;

        llama_tokens prompt_buf;
        llama_tokens result_buf;
        params.prompt = &prompt_buf;
        params.result = &result_buf;

        common_speculative_draft(spec->ptr.get());

        uint32_t n = static_cast<uint32_t>(result_buf.size());
        if (n > static_cast<uint32_t>(n_max)) n = n_max;
        for (uint32_t i = 0; i < n; ++i) out_tokens[i] = result_buf[i];
        return n;
    } catch (...) {
        return 0;
    }
}

void arkavo_spec_accept(arkavo_spec *spec, int32_t seq_id, uint16_t n_accepted) {
    if (!spec) return;
    try {
        common_speculative_accept(spec->ptr.get(), seq_id, n_accepted);
    } catch (...) {}
}

} // extern "C"
