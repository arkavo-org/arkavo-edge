// Thin C wrapper around llama.cpp's common_chat_templates_apply() C++ API.
// Provides extern "C" functions callable from Rust via FFI.

#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Forward declare llama_model (defined in llama.h)
struct llama_model;

// Opaque handle to common_chat_templates
typedef struct arkavo_chat_templates arkavo_chat_templates;

// A tool call: used both for parsed model output and for carrying an
// assistant message's prior tool calls back into the chat template.
typedef struct {
    const char *name;
    const char *arguments;   // JSON string
    const char *id;
} arkavo_tool_call;

typedef struct {
    const char *role;
    const char *content;
    const char *tool_call_id;
    const char *tool_name;
    // Structured tool calls made by this (assistant) message. Required by
    // templates that render tool calls from a structured list rather than
    // from inline content markup (e.g. Gemma 4's convert_tool_responses_gemma4).
    const arkavo_tool_call *tool_calls;
    int num_tool_calls;
} arkavo_chat_msg;

typedef struct {
    const char *name;
    const char *description;
    const char *parameters_json;
} arkavo_chat_tool;

typedef struct {
    int type;       // 0=TOKEN, 1=WORD, 2=PATTERN, 3=PATTERN_FULL
    const char *value;
    int token;      // LLAMA_TOKEN_NULL if not a token trigger
} arkavo_grammar_trigger;

typedef struct {
    char *prompt;
    char *grammar;
    int grammar_lazy;
    int thinking_forced_open;
    arkavo_grammar_trigger *triggers;
    int num_triggers;
    char **additional_stops;
    int num_additional_stops;
    int format;              // common_chat_format enum value for output parsing
    char *parser_str;        // PEG parser string for output parsing
    char *generation_prompt; // generation prompt for grammar prefilling
} arkavo_chat_result;

arkavo_chat_templates *arkavo_chat_templates_init(
    const struct llama_model *model,
    const char *chat_template_override);

void arkavo_chat_templates_free(arkavo_chat_templates *tmpls);

arkavo_chat_result arkavo_chat_templates_apply(
    const arkavo_chat_templates *tmpls,
    const arkavo_chat_msg *messages, int num_messages,
    const arkavo_chat_tool *tools, int num_tools,
    int tool_choice,
    int enable_thinking,
    int add_generation_prompt,
    int parallel_tool_calls);

void arkavo_chat_result_free(arkavo_chat_result *result);

// Result from parsing model output
typedef struct {
    char *content;
    char *reasoning_content;
    arkavo_tool_call *tool_calls;
    int num_tool_calls;
} arkavo_chat_parse_result;

// Parse model output text into structured content + tool calls.
// Uses llama.cpp's built-in PEG parser (supports Gemma 4, Mistral, etc.)
// `format` and `parser_str` come from the arkavo_chat_result of template application.
arkavo_chat_parse_result arkavo_chat_parse(
    const char *output_text,
    int format,
    const char *parser_str,
    int is_partial);

void arkavo_chat_parse_result_free(arkavo_chat_parse_result *result);

// Grammar sampler with both pattern and token triggers.
// trigger_patterns: regex patterns matched against generated text (Word/Pattern types)
// trigger_tokens: token IDs that activate the grammar when sampled (Token type)
struct llama_sampler *arkavo_sampler_init_grammar_lazy_with_tokens(
    const struct llama_vocab *vocab,
    const char *grammar_str,
    const char *grammar_root,
    const char **trigger_patterns,
    int num_trigger_patterns,
    const int32_t *trigger_tokens,
    int num_trigger_tokens);

// Quiet llama.cpp's "common" library logging — the chat-template and
// speculative-decoding notices emitted as LOG_INF/LOG_WRN. When quiet != 0,
// only errors are emitted; when quiet == 0, the upstream default verbosity is
// restored so debug runs see everything. This is a separate logging system from
// the ggml/llama_log_set callback, so it needs its own control.
void arkavo_set_common_log_quiet(int quiet);

#ifdef __cplusplus
}
#endif
