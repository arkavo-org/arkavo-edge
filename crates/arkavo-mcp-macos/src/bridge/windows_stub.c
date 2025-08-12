// Minimal stub implementations for Windows build
// These functions are not used on Windows but needed for linking

char stub_return[] = "{\"status\": \"stub\"}";

char* ios_bridge_execute_action(void* bridge, const char* action, const char* params) {
    (void)bridge;
    (void)action;
    (void)params;
    return stub_return;
}

char* ios_bridge_get_current_state(void* bridge) {
    (void)bridge;
    return stub_return;
}

char* ios_bridge_mutate_state(void* bridge, const char* entity, const char* action, const char* data) {
    (void)bridge;
    (void)entity;
    (void)action;
    (void)data;
    return stub_return;
}

void* ios_bridge_create_snapshot(void* bridge, unsigned long long* size) {
    (void)bridge;
    *size = 0;
    return (void*)0;
}

void ios_bridge_restore_snapshot(void* bridge, const void* data, unsigned long long size) {
    (void)bridge;
    (void)data;
    (void)size;
}

void ios_bridge_free_string(char* s) {
    (void)s;
}

void ios_bridge_free_data(void* data) {
    (void)data;
}