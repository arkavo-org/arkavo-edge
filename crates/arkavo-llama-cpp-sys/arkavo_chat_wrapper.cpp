// Thin C wrapper around llama.cpp's common_chat_templates_apply() C++ API.
// Converts between C-compatible structs and C++ types.
// All C++ exceptions are caught at the extern "C" boundary to prevent UB.

#include "arkavo_chat_wrapper.h"
#include "llama.h"
#include "chat.h"
#include "peg-parser.h"

#include <cstdlib>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>

// The opaque handle wraps a common_chat_templates_ptr (unique_ptr)
struct arkavo_chat_templates {
    common_chat_templates_ptr ptr;
};

static char *strdup_safe(const std::string &s) {
    if (s.empty()) {
        return nullptr;
    }
    char *dup = static_cast<char *>(malloc(s.size() + 1));
    if (dup) {
        memcpy(dup, s.c_str(), s.size() + 1);
    }
    return dup;
}

extern "C" {

arkavo_chat_templates *arkavo_chat_templates_init(
    const struct llama_model *model,
    const char *chat_template_override)
{
    try {
        std::string override_str = chat_template_override ? chat_template_override : "";
        auto cpp_ptr = common_chat_templates_init(model, override_str);
        if (!cpp_ptr) {
            return nullptr;
        }
        auto *result = new arkavo_chat_templates();
        result->ptr = std::move(cpp_ptr);
        return result;
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_chat_templates_init: %s\n", e.what());
        return nullptr;
    } catch (...) {
        fprintf(stderr, "arkavo_chat_templates_init: unknown exception\n");
        return nullptr;
    }
}

void arkavo_chat_templates_free(arkavo_chat_templates *tmpls) {
    try {
        delete tmpls;
    } catch (...) {
        // Suppress exceptions from destructor
    }
}

arkavo_chat_result arkavo_chat_templates_apply(
    const arkavo_chat_templates *tmpls,
    const arkavo_chat_msg *messages, int num_messages,
    const arkavo_chat_tool *tools, int num_tools,
    int tool_choice,
    int enable_thinking,
    int add_generation_prompt,
    int parallel_tool_calls)
{
    arkavo_chat_result result = {};

    if (!tmpls || !tmpls->ptr) {
        return result;
    }

    try {
        // Convert messages
        common_chat_templates_inputs inputs;
        inputs.add_generation_prompt = (add_generation_prompt != 0);
        inputs.use_jinja = true;
        inputs.enable_thinking = (enable_thinking != 0);
        inputs.parallel_tool_calls = (parallel_tool_calls != 0);

        for (int i = 0; i < num_messages; i++) {
            common_chat_msg msg;
            msg.role = messages[i].role ? messages[i].role : "";
            msg.content = messages[i].content ? messages[i].content : "";
            if (messages[i].tool_call_id) {
                msg.tool_call_id = messages[i].tool_call_id;
            }
            if (messages[i].tool_name) {
                msg.tool_name = messages[i].tool_name;
            }
            for (int j = 0; j < messages[i].num_tool_calls; j++) {
                const arkavo_tool_call &src = messages[i].tool_calls[j];
                common_chat_tool_call tc;
                tc.name = src.name ? src.name : "";
                tc.arguments = src.arguments ? src.arguments : "{}";
                if (src.id) {
                    tc.id = src.id;
                }
                msg.tool_calls.push_back(std::move(tc));
            }
            inputs.messages.push_back(std::move(msg));
        }

        // Convert tools
        for (int i = 0; i < num_tools; i++) {
            common_chat_tool tool;
            tool.name = tools[i].name ? tools[i].name : "";
            tool.description = tools[i].description ? tools[i].description : "";
            tool.parameters = tools[i].parameters_json ? tools[i].parameters_json : "";
            inputs.tools.push_back(std::move(tool));
        }

        // Convert tool_choice
        switch (tool_choice) {
            case 1: inputs.tool_choice = COMMON_CHAT_TOOL_CHOICE_REQUIRED; break;
            case 2: inputs.tool_choice = COMMON_CHAT_TOOL_CHOICE_NONE; break;
            default: inputs.tool_choice = COMMON_CHAT_TOOL_CHOICE_AUTO; break;
        }

        // Apply template
        common_chat_params params = common_chat_templates_apply(tmpls->ptr.get(), inputs);

        // Convert result
        result.prompt = strdup_safe(params.prompt);
        result.grammar = strdup_safe(params.grammar);
        result.grammar_lazy = params.grammar_lazy ? 1 : 0;
        result.thinking_forced_open = params.supports_thinking ? 1 : 0;
        result.format = static_cast<int>(params.format);
        // Serialize the PEG parser so Rust can pass it back for output parsing
        result.parser_str = strdup_safe(params.parser);
        result.generation_prompt = strdup_safe(params.generation_prompt);

        // Convert grammar triggers
        if (!params.grammar_triggers.empty()) {
            result.num_triggers = static_cast<int>(params.grammar_triggers.size());
            result.triggers = static_cast<arkavo_grammar_trigger *>(
                calloc(result.num_triggers, sizeof(arkavo_grammar_trigger)));
            for (int i = 0; i < result.num_triggers; i++) {
                result.triggers[i].type = static_cast<int>(params.grammar_triggers[i].type);
                result.triggers[i].value = strdup_safe(params.grammar_triggers[i].value);
                result.triggers[i].token = params.grammar_triggers[i].token;
            }
        }

        // Convert additional stops
        if (!params.additional_stops.empty()) {
            result.num_additional_stops = static_cast<int>(params.additional_stops.size());
            result.additional_stops = static_cast<char **>(
                calloc(result.num_additional_stops, sizeof(char *)));
            for (int i = 0; i < result.num_additional_stops; i++) {
                result.additional_stops[i] = strdup_safe(params.additional_stops[i]);
            }
        }
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_chat_templates_apply: %s\n", e.what());
        // result stays zeroed — caller sees null prompt and falls back to legacy
    } catch (...) {
        fprintf(stderr, "arkavo_chat_templates_apply: unknown exception\n");
    }

    return result;
}

void arkavo_chat_result_free(arkavo_chat_result *result) {
    if (!result) {
        return;
    }
    free(result->prompt);
    free(result->grammar);
    free(result->parser_str);
    free(result->generation_prompt);
    for (int i = 0; i < result->num_triggers; i++) {
        free(const_cast<char *>(result->triggers[i].value));
    }
    free(result->triggers);
    for (int i = 0; i < result->num_additional_stops; i++) {
        free(result->additional_stops[i]);
    }
    free(result->additional_stops);
    *result = {};
}

arkavo_chat_parse_result arkavo_chat_parse(
    const char *output_text,
    int format,
    const char *parser_str,
    int is_partial)
{
    arkavo_chat_parse_result result = {};

    if (!output_text) {
        return result;
    }

    try {
        common_chat_parser_params params;
        params.format = static_cast<common_chat_format>(format);
        params.parse_tool_calls = true;

        if (getenv("ARKAVO_DEBUG_PEG")) {
            params.debug = true;
        }

        common_chat_msg msg;
        if (parser_str && parser_str[0] != '\0') {
            common_peg_arena arena;
            arena.load(parser_str);
            msg = common_chat_peg_parse(arena, output_text, is_partial != 0, params);
        } else {
            msg = common_chat_parse(output_text, is_partial != 0, params);
        }

        if (getenv("ARKAVO_DEBUG_PEG")) {
            fprintf(stderr, "[PEG-RESULT] content_len=%zu tool_calls=%zu\n",
                    msg.content.size(), msg.tool_calls.size());
            for (size_t i = 0; i < msg.tool_calls.size() && i < 3; i++) {
                fprintf(stderr, "[PEG-RESULT] call[%zu] name=%s args_len=%zu args=%.100s\n",
                        i, msg.tool_calls[i].name.c_str(),
                        msg.tool_calls[i].arguments.size(),
                        msg.tool_calls[i].arguments.c_str());
            }
        }

        result.content = strdup_safe(msg.content);
        result.reasoning_content = strdup_safe(msg.reasoning_content);

        if (!msg.tool_calls.empty()) {
            result.num_tool_calls = static_cast<int>(msg.tool_calls.size());
            result.tool_calls = static_cast<arkavo_tool_call *>(
                calloc(result.num_tool_calls, sizeof(arkavo_tool_call)));
            for (int i = 0; i < result.num_tool_calls; i++) {
                result.tool_calls[i].name = strdup_safe(msg.tool_calls[i].name);
                result.tool_calls[i].arguments = strdup_safe(msg.tool_calls[i].arguments);
                result.tool_calls[i].id = strdup_safe(msg.tool_calls[i].id);
            }
        }
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_chat_parse: %s\n", e.what());
    } catch (...) {
        fprintf(stderr, "arkavo_chat_parse: unknown exception\n");
    }

    return result;
}

void arkavo_chat_parse_result_free(arkavo_chat_parse_result *result) {
    if (!result) {
        return;
    }
    free(result->content);
    free(result->reasoning_content);
    for (int i = 0; i < result->num_tool_calls; i++) {
        free(const_cast<char *>(result->tool_calls[i].name));
        free(const_cast<char *>(result->tool_calls[i].arguments));
        free(const_cast<char *>(result->tool_calls[i].id));
    }
    free(result->tool_calls);
    *result = {};
}

struct llama_sampler *arkavo_sampler_init_grammar(
    const struct llama_vocab *vocab,
    const char *grammar_str,
    const char *grammar_root)
{
    try {
        return llama_sampler_init_grammar(vocab, grammar_str, grammar_root);
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_sampler_init_grammar: %s\n", e.what());
        return nullptr;
    } catch (...) {
        fprintf(stderr, "arkavo_sampler_init_grammar: unknown exception\n");
        return nullptr;
    }
}

struct llama_sampler *arkavo_sampler_init_grammar_lazy(
    const struct llama_vocab *vocab,
    const char *grammar_str,
    const char *grammar_root,
    const char **trigger_patterns,
    int num_triggers)
{
    try {
        return llama_sampler_init_grammar_lazy_patterns(
            vocab, grammar_str, grammar_root,
            trigger_patterns, static_cast<size_t>(num_triggers),
            nullptr, 0);
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_sampler_init_grammar_lazy: %s\n", e.what());
        return nullptr;
    } catch (...) {
        fprintf(stderr, "arkavo_sampler_init_grammar_lazy: unknown exception\n");
        return nullptr;
    }
}

struct llama_sampler *arkavo_sampler_init_grammar_lazy_with_tokens(
    const struct llama_vocab *vocab,
    const char *grammar_str,
    const char *grammar_root,
    const char **trigger_patterns,
    int num_trigger_patterns,
    const int32_t *trigger_tokens,
    int num_trigger_tokens)
{
    try {
        return llama_sampler_init_grammar_lazy_patterns(
            vocab, grammar_str, grammar_root,
            trigger_patterns, static_cast<size_t>(num_trigger_patterns),
            trigger_tokens, static_cast<size_t>(num_trigger_tokens));
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_sampler_init_grammar_lazy_with_tokens: %s\n", e.what());
        return nullptr;
    } catch (...) {
        fprintf(stderr, "arkavo_sampler_init_grammar_lazy_with_tokens: unknown exception\n");
        return nullptr;
    }
}

int32_t arkavo_sampler_sample(
    struct llama_sampler *sampler,
    struct llama_context *ctx,
    int idx)
{
    try {
        return llama_sampler_sample(sampler, ctx, idx);
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_sampler_sample: %s\n", e.what());
        return -1;
    } catch (...) {
        fprintf(stderr, "arkavo_sampler_sample: unknown exception\n");
        return -1;
    }
}

void arkavo_sampler_accept(
    struct llama_sampler *sampler,
    int32_t token)
{
    try {
        llama_sampler_accept(sampler, token);
    } catch (const std::exception &e) {
        fprintf(stderr, "arkavo_sampler_accept: %s\n", e.what());
    } catch (...) {
        fprintf(stderr, "arkavo_sampler_accept: unknown exception\n");
    }
}

void arkavo_sampler_free(struct llama_sampler *sampler) {
    try {
        llama_sampler_free(sampler);
    } catch (...) {
        // Suppress exceptions from destructor
    }
}

} // extern "C"
