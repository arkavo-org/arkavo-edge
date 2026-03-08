// Thin C wrapper around llama.cpp's common_chat_templates_apply() C++ API.
// Provides extern "C" functions callable from Rust via FFI.

#pragma once

#ifdef __cplusplus
extern "C" {
#endif

// Forward declare llama_model (defined in llama.h)
struct llama_model;

// Opaque handle to common_chat_templates
typedef struct arkavo_chat_templates arkavo_chat_templates;

typedef struct {
    const char *role;
    const char *content;
    const char *tool_call_id;
    const char *tool_name;
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
    int add_generation_prompt);

void arkavo_chat_result_free(arkavo_chat_result *result);

#ifdef __cplusplus
}
#endif
