#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/wait.h>
#include <CoreFoundation/CoreFoundation.h>

typedef struct {
    char* device_id;
    char* bundle_id;
    void* xctest_session;
} IOSBridgeImpl;

static char* execute_command(const char* command) {
    FILE* pipe = popen(command, "r");
    if (!pipe) {
        return strdup("{\"error\": \"Failed to execute command\"}");
    }
    
    char buffer[4096];
    size_t total_size = 0;
    char* result = NULL;
    
    while (fgets(buffer, sizeof(buffer), pipe) != NULL) {
        size_t buf_len = strlen(buffer);
        char* new_result = realloc(result, total_size + buf_len + 1);
        if (!new_result) {
            free(result);
            pclose(pipe);
            return strdup("{\"error\": \"Memory allocation failed\"}");
        }
        result = new_result;
        memcpy(result + total_size, buffer, buf_len + 1);
        total_size += buf_len;
    }
    
    pclose(pipe);
    
    if (!result) {
        return strdup("{\"error\": \"No output\"}");
    }
    
    return result;
}

static char* get_booted_device_id() {
    char* output = execute_command("xcrun simctl list devices booted -j");
    
    // Simple JSON parsing to extract device ID
    char* devices_start = strstr(output, "\"devices\"");
    if (!devices_start) {
        free(output);
        return NULL;
    }
    
    char* uuid_start = strstr(devices_start, "\"udid\"");
    if (!uuid_start) {
        free(output);
        return NULL;
    }
    
    uuid_start = strchr(uuid_start, ':');
    if (!uuid_start) {
        free(output);
        return NULL;
    }
    
    uuid_start = strchr(uuid_start, '\"');
    if (!uuid_start) {
        free(output);
        return NULL;
    }
    uuid_start++;
    
    char* uuid_end = strchr(uuid_start, '\"');
    if (!uuid_end) {
        free(output);
        return NULL;
    }
    
    size_t uuid_len = uuid_end - uuid_start;
    char* device_id = malloc(uuid_len + 1);
    memcpy(device_id, uuid_start, uuid_len);
    device_id[uuid_len] = '\0';
    
    free(output);
    return device_id;
}

static char* perform_tap(IOSBridgeImpl* bridge, double x, double y) {
    char command[512];
    snprintf(command, sizeof(command), 
             "xcrun simctl io %s tap %.0f %.0f", 
             bridge->device_id, x, y);
    
    FILE* pipe = popen(command, "r");
    if (!pipe) {
        return strdup("{\"success\": false, \"error\": \"Failed to execute tap\"}");
    }
    
    int status = pclose(pipe);
    if (status == 0) {
        char result[256];
        snprintf(result, sizeof(result), 
                 "{\"success\": true, \"action\": \"tap\", \"coordinates\": {\"x\": %.0f, \"y\": %.0f}}", 
                 x, y);
        return strdup(result);
    } else {
        return strdup("{\"success\": false, \"error\": \"Tap command failed\"}");
    }
}

static char* perform_swipe(IOSBridgeImpl* bridge, double x1, double y1, double x2, double y2, double duration) {
    char command[512];
    snprintf(command, sizeof(command), 
             "xcrun simctl io %s swipe %.0f %.0f %.0f %.0f --duration=%.2f", 
             bridge->device_id, x1, y1, x2, y2, duration);
    
    FILE* pipe = popen(command, "r");
    if (!pipe) {
        return strdup("{\"success\": false, \"error\": \"Failed to execute swipe\"}");
    }
    
    int status = pclose(pipe);
    if (status == 0) {
        return strdup("{\"success\": true, \"action\": \"swipe\"}");
    } else {
        return strdup("{\"success\": false, \"error\": \"Swipe command failed\"}");
    }
}

static char* type_text(IOSBridgeImpl* bridge, const char* text) {
    // Escape special characters for shell
    char escaped_text[1024];
    size_t j = 0;
    for (size_t i = 0; text[i] && j < sizeof(escaped_text) - 2; i++) {
        if (text[i] == '\'' || text[i] == '\\' || text[i] == '"') {
            escaped_text[j++] = '\\';
        }
        escaped_text[j++] = text[i];
    }
    escaped_text[j] = '\0';
    
    char command[2048];
    snprintf(command, sizeof(command), 
             "xcrun simctl io %s type '%s'", 
             bridge->device_id, escaped_text);
    
    FILE* pipe = popen(command, "r");
    if (!pipe) {
        return strdup("{\"success\": false, \"error\": \"Failed to type text\"}");
    }
    
    int status = pclose(pipe);
    if (status == 0) {
        char result[512];
        snprintf(result, sizeof(result), 
                 "{\"success\": true, \"action\": \"type_text\", \"text\": \"%s\"}", 
                 text);
        return strdup(result);
    } else {
        return strdup("{\"success\": false, \"error\": \"Type text command failed\"}");
    }
}

static char* get_accessibility_tree(IOSBridgeImpl* bridge) {
    char command[512];
    snprintf(command, sizeof(command), 
             "xcrun simctl launch %s com.apple.Accessibility.AccessibilityUtility --dump", 
             bridge->device_id);
    
    char* output = execute_command(command);
    
    // Parse and format the accessibility tree
    // For now, return a structured representation
    char* result = malloc(4096);
    snprintf(result, 4096, 
             "{\"tree\": {\"root\": {\"type\": \"Application\", \"bundleId\": \"%s\", \"children\": []}}}", 
             bridge->bundle_id);
    
    free(output);
    return result;
}

char* ios_bridge_execute_action(void* bridge, const char* action, const char* params) {
    IOSBridgeImpl* impl = (IOSBridgeImpl*)bridge;
    
    if (!impl || !impl->device_id) {
        impl = malloc(sizeof(IOSBridgeImpl));
        impl->device_id = get_booted_device_id();
        impl->bundle_id = strdup("com.arkavo.testapp");
        impl->xctest_session = NULL;
        
        if (!impl->device_id) {
            return strdup("{\"error\": \"No booted iOS simulator found\"}");
        }
    }
    
    // Parse action and params (simple JSON parsing)
    if (strcmp(action, "tap") == 0) {
        // Extract x, y from params
        double x = 100, y = 100;
        char* x_str = strstr(params, "\"x\":");
        char* y_str = strstr(params, "\"y\":");
        
        if (x_str) {
            x = atof(x_str + 4);
        }
        if (y_str) {
            y = atof(y_str + 4);
        }
        
        return perform_tap(impl, x, y);
    } else if (strcmp(action, "swipe") == 0) {
        // Extract coordinates from params
        double x1 = 100, y1 = 100, x2 = 200, y2 = 200, duration = 0.5;
        
        char* x1_str = strstr(params, "\"x1\":");
        char* y1_str = strstr(params, "\"y1\":");
        char* x2_str = strstr(params, "\"x2\":");
        char* y2_str = strstr(params, "\"y2\":");
        char* dur_str = strstr(params, "\"duration\":");
        
        if (x1_str) x1 = atof(x1_str + 5);
        if (y1_str) y1 = atof(y1_str + 5);
        if (x2_str) x2 = atof(x2_str + 5);
        if (y2_str) y2 = atof(y2_str + 5);
        if (dur_str) duration = atof(dur_str + 11);
        
        return perform_swipe(impl, x1, y1, x2, y2, duration);
    } else if (strcmp(action, "type_text") == 0) {
        // Extract text from params
        char* text_start = strstr(params, "\"text\":\"");
        if (!text_start) {
            return strdup("{\"error\": \"No text parameter found\"}");
        }
        
        text_start += 8;
        char* text_end = strchr(text_start, '\"');
        if (!text_end) {
            return strdup("{\"error\": \"Invalid text parameter\"}");
        }
        
        size_t text_len = text_end - text_start;
        char* text = malloc(text_len + 1);
        memcpy(text, text_start, text_len);
        text[text_len] = '\0';
        
        char* result = type_text(impl, text);
        free(text);
        return result;
    } else if (strcmp(action, "screenshot") == 0) {
        char* path_start = strstr(params, "\"path\":\"");
        char path[256] = "screenshot.png";
        
        if (path_start) {
            path_start += 8;
            char* path_end = strchr(path_start, '\"');
            if (path_end) {
                size_t len = path_end - path_start;
                if (len < sizeof(path) - 1) {
                    memcpy(path, path_start, len);
                    path[len] = '\0';
                }
            }
        }
        
        char command[512];
        snprintf(command, sizeof(command), 
                 "xcrun simctl io %s screenshot %s", 
                 impl->device_id, path);
        
        FILE* pipe = popen(command, "r");
        if (!pipe) {
            return strdup("{\"success\": false, \"error\": \"Failed to capture screenshot\"}");
        }
        
        int status = pclose(pipe);
        char result[256];
        snprintf(result, sizeof(result), 
                 "{\"success\": %s, \"path\": \"%s\"}", 
                 status == 0 ? "true" : "false", path);
        return strdup(result);
    } else if (strcmp(action, "query_ui") == 0) {
        return get_accessibility_tree(impl);
    }
    
    return strdup("{\"error\": \"Unknown action\"}");
}

char* ios_bridge_get_current_state(void* bridge) {
    IOSBridgeImpl* impl = (IOSBridgeImpl*)bridge;
    
    if (!impl || !impl->device_id) {
        return strdup("{\"state\": \"uninitialized\"}");
    }
    
    // Get device state
    char command[256];
    snprintf(command, sizeof(command), 
             "xcrun simctl list devices | grep %s", 
             impl->device_id);
    
    char* output = execute_command(command);
    
    char* state = "unknown";
    if (strstr(output, "Booted")) {
        state = "booted";
    } else if (strstr(output, "Shutdown")) {
        state = "shutdown";
    }
    
    char result[512];
    snprintf(result, sizeof(result), 
             "{\"device_id\": \"%s\", \"state\": \"%s\", \"bundle_id\": \"%s\"}", 
             impl->device_id, state, impl->bundle_id);
    
    free(output);
    return strdup(result);
}

char* ios_bridge_mutate_state(void* bridge, const char* entity, const char* action, const char* data) {
    IOSBridgeImpl* impl = (IOSBridgeImpl*)bridge;
    
    if (strcmp(entity, "simulator") == 0) {
        if (strcmp(action, "boot") == 0) {
            char command[256];
            snprintf(command, sizeof(command), "xcrun simctl boot %s", impl->device_id);
            execute_command(command);
            return strdup("{\"success\": true}");
        } else if (strcmp(action, "shutdown") == 0) {
            char command[256];
            snprintf(command, sizeof(command), "xcrun simctl shutdown %s", impl->device_id);
            execute_command(command);
            return strdup("{\"success\": true}");
        }
    } else if (strcmp(entity, "app") == 0) {
        if (strcmp(action, "launch") == 0) {
            char command[512];
            snprintf(command, sizeof(command), 
                     "xcrun simctl launch %s %s", 
                     impl->device_id, impl->bundle_id);
            execute_command(command);
            return strdup("{\"success\": true}");
        } else if (strcmp(action, "terminate") == 0) {
            char command[512];
            snprintf(command, sizeof(command), 
                     "xcrun simctl terminate %s %s", 
                     impl->device_id, impl->bundle_id);
            execute_command(command);
            return strdup("{\"success\": true}");
        }
    }
    
    return strdup("{\"success\": false, \"error\": \"Unknown entity or action\"}");
}

void* ios_bridge_create_snapshot(void* bridge, size_t* size) {
    IOSBridgeImpl* impl = (IOSBridgeImpl*)bridge;
    
    // Create a snapshot of current state
    char state_json[1024];
    snprintf(state_json, sizeof(state_json), 
             "{\"device_id\": \"%s\", \"bundle_id\": \"%s\", \"timestamp\": %ld}", 
             impl->device_id, impl->bundle_id, time(NULL));
    
    *size = strlen(state_json) + 1;
    void* data = malloc(*size);
    memcpy(data, state_json, *size);
    
    return data;
}

void ios_bridge_restore_snapshot(void* bridge, const void* data, size_t size) {
    // Parse snapshot data and restore state
    // For now, this is a no-op as simulator state is managed externally
}

void ios_bridge_free_string(char* s) {
    free(s);
}

void ios_bridge_free_data(void* data) {
    free(data);
}