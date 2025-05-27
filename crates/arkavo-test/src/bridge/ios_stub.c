#include <stdlib.h>
#include <string.h>

// Stub implementations for iOS bridge functions
// These will be replaced with actual implementations when building for iOS

char* ios_bridge_execute_action(void* bridge, const char* action, const char* params) {
    (void)bridge;
    (void)action;
    (void)params;
    return strdup("{\"status\": \"stub\"}");
}

char* ios_bridge_get_current_state(void* bridge) {
    (void)bridge;
    return strdup("{\"state\": \"stub\"}");
}

char* ios_bridge_mutate_state(void* bridge, const char* entity, const char* action, const char* data) {
    (void)bridge;
    (void)entity;
    (void)action;
    (void)data;
    return strdup("{\"success\": true}");
}

void* ios_bridge_create_snapshot(void* bridge, size_t* size) {
    (void)bridge;
    *size = 4;
    void* data = malloc(4);
    memset(data, 0, 4);
    return data;
}

void ios_bridge_restore_snapshot(void* bridge, const void* data, size_t size) {
    (void)bridge;
    (void)data;
    (void)size;
    // No-op stub
}

void ios_bridge_free_string(char* s) {
    free(s);
}

void ios_bridge_free_data(void* data) {
    free(data);
}